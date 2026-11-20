//! The headless client: sign-in, cursor-based sync over WebSocket with reconnect and backoff,
//! the local cache, and the idempotent outbox. UI layers and the CLI sit on top of `Client`.

use crate::api::*;
use crate::error::{ClientError, Result};
use crate::secrets::{FileSecretStore, SecretStore};
use crate::store::*;
use futures_util::{SinkExt, StreamExt};
use proto::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, Notify};
use tokio_tungstenite::tungstenite::Message as WsFrame;

#[derive(Clone)]
pub struct ClientConfig {
    pub data_dir: PathBuf,
    pub device_name: String,
    pub client_version: String,
    /// Permit `http://` servers (test servers only).
    pub allow_insecure_http: bool,
    pub secrets: Option<Arc<dyn SecretStore>>,
    /// X-04: keep the local cache encrypted with SQLCipher, the key in the platform secret store (on by default).
    pub encrypt_cache: bool,
}

/// Name of the cache key in the secret store.
pub const CACHE_KEY_NAME: &str = "cache_key";

/// X-04: opens the local cache, encrypted when the secret store can keep the key. A plain cache from an older version is
/// converted. If the key cannot be saved (no usable secret store) the cache stays plain instead of risking data loss.
fn open_cache(path: &std::path::Path, secrets: &dyn SecretStore, encrypt: bool) -> Store {
    // The app and its background service may open the cache at the same moment; only one may convert it.
    static OPEN_LOCK: Mutex<()> = Mutex::new(());
    let _g = OPEN_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let plain = || {
        Store::open(path)
            .or_else(|_| {
                // An encrypted file we have no key for: keep it aside (the key may come back) and start a new cache.
                let _ = std::fs::rename(path, path.with_extension("db.unreadable"));
                Store::open(path)
            })
            .expect("open cache")
    };
    if !encrypt {
        return plain();
    }
    let key = match secrets.get(CACHE_KEY_NAME) {
        Ok(Some(k)) if k.len() == 64 => Some(k),
        Ok(_) => {
            use rand::RngCore;
            let mut b = [0u8; 32];
            rand::rngs::OsRng.fill_bytes(&mut b);
            let k: String = b.iter().map(|x| format!("{x:02x}")).collect();
            // Read it back: a store that accepts but loses the value would lock the cache forever.
            match secrets.set(CACHE_KEY_NAME, &k) {
                Ok(()) if matches!(secrets.get(CACHE_KEY_NAME), Ok(Some(ref v)) if *v == k) => Some(k),
                _ => None,
            }
        }
        Err(_) => None,
    };
    match key {
        Some(k) => match Store::open_with_key(path, &k) {
            Ok((s, state)) => {
                if state != crate::store::CacheState::Opened && state != crate::store::CacheState::Created {
                    eprintln!("superchat: local cache {state:?}");
                }
                s
            }
            Err(e) => {
                eprintln!("superchat: encrypted cache failed ({e}); using a plain cache");
                plain()
            }
        },
        None => {
            eprintln!("superchat: no secret store for the cache key; the local cache is not encrypted");
            plain()
        }
    }
}

impl ClientConfig {
    pub fn new(data_dir: impl Into<PathBuf>) -> ClientConfig {
        ClientConfig {
            data_dir: data_dir.into(),
            device_name: "SuperChat".into(),
            client_version: crate::CLIENT_VERSION.into(),
            allow_insecure_http: false,
            secrets: None,
            encrypt_cache: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnState {
    SignedOut,
    Connecting,
    Online,
    Offline { retry_in_secs: u64 },
}

#[derive(Debug, Clone)]
pub enum ClientEvent {
    Connection(ConnState),
    /// The cache changed. `conv_ids` lists touched conversations.
    Changed { conv_ids: Vec<String>, users: bool, me: bool },
    /// A message from someone else arrived (drives desktop notifications, N-02).
    Incoming { message: Message, conversation: Option<ConversationSummary>, sender_name: String },
    Typing { conv_id: String, user_id: String },
    Presence { user_id: String, presence: Presence, last_active_ms: Option<i64> },
    /// The account was disabled / the session revoked / history cleared: local data was wiped.
    SignedOut { code: Option<ErrorCode>, message: String },
    UploadProgress { client_id: String, sent: u64, total: u64 },
    SendFailed { client_id: String, error: String },
    /// 1.4.1: the App Lock state changed (locked, unlocked, a chat marked or unmarked, the PIN set or removed). Every list,
    /// search and count is computed from the lock state on each read, so a view only has to read again.
    LockChanged,
}

mod auto;
mod org;
mod social;
mod pins;
pub use pins::{merge_pinned_order, ListKind, PhoneList};
pub use social::SavedView;

pub struct Inner {
    me: Weak<Inner>,
    cfg: ClientConfig,
    pub store: Store,
    secrets: Arc<dyn SecretStore>,
    applock: Arc<crate::applock::AppLock>,
    api: RwLock<Option<Arc<Api>>>,
    events: broadcast::Sender<ClientEvent>,
    handle: tokio::runtime::Handle,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    outbox_notify: Notify,
    wake: Notify,
    ws_cmd: Mutex<Option<mpsc::UnboundedSender<ClientWsMsg>>>,
    state: Mutex<ConnState>,
    typing_sent: Mutex<HashMap<String, Instant>>,
    cancelled: Mutex<HashSet<String>>,
    uploads: Mutex<HashMap<String, UploadState>>,
    server_meta: Mutex<Option<ServerMeta>>,
}

struct UploadCtx {
    client_id: String,
    inner: Arc<Inner>,
    name: String,
    index: usize,
    count: usize,
    base: u64,
    total_all: u64,
}

/// F-01: where an outbox item's attachments are in their upload (all files of the message together).
#[derive(Debug, Clone, PartialEq)]
pub struct UploadState {
    pub file_name: String,
    pub file_index: usize,
    pub file_count: usize,
    pub sent: u64,
    pub total: u64,
}

#[derive(Clone)]
pub struct Client {
    pub inner: Arc<Inner>,
}

impl Client {
    /// Opens the cache and restores a saved session. Call from within a tokio runtime.
    pub fn open(cfg: ClientConfig) -> Result<Client> {
        Self::open_with_handle(cfg, tokio::runtime::Handle::current())
    }

    pub fn open_with_handle(cfg: ClientConfig, handle: tokio::runtime::Handle) -> Result<Client> {
        std::fs::create_dir_all(&cfg.data_dir)?;
        let secrets: Arc<dyn SecretStore> = cfg
            .secrets
            .clone()
            .unwrap_or_else(|| Arc::new(FileSecretStore::new(cfg.data_dir.join("secrets.json"))));
        let store = open_cache(&cfg.data_dir.join("cache.db"), secrets.as_ref(), cfg.encrypt_cache);
        let (events, _) = broadcast::channel(512);
        let applock = Arc::new(crate::applock::AppLock::open(secrets.clone()));
        let inner = Arc::new_cyclic(|me| Inner {
            me: me.clone(),
            cfg,
            store,
            secrets,
            applock,
            api: RwLock::new(None),
            events,
            handle,
            tasks: Mutex::new(vec![]),
            outbox_notify: Notify::new(),
            wake: Notify::new(),
            ws_cmd: Mutex::new(None),
            state: Mutex::new(ConnState::SignedOut),
            typing_sent: Mutex::new(HashMap::new()),
            cancelled: Mutex::new(HashSet::new()),
            uploads: Mutex::new(HashMap::new()),
            server_meta: Mutex::new(None),
        });
        let client = Client { inner };
        client.inner.restore_session()?;
        Ok(client)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ClientEvent> {
        self.inner.events.subscribe()
    }

    pub fn store(&self) -> &Store {
        &self.inner.store
    }

    pub fn data_dir(&self) -> &Path {
        &self.inner.cfg.data_dir
    }

    pub fn config(&self) -> &ClientConfig {
        &self.inner.cfg
    }

    pub fn handle(&self) -> &tokio::runtime::Handle {
        &self.inner.handle
    }

    pub fn connection(&self) -> ConnState {
        self.inner.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn session(&self) -> Option<Session> {
        self.inner.api().ok().and_then(|a| a.session())
    }

    pub fn signed_in(&self) -> bool {
        self.session().is_some()
    }

    pub fn my_id(&self) -> Option<String> {
        self.session().map(|s| s.user_id)
    }

    pub fn me(&self) -> Option<Me> {
        self.inner.store.me()
    }

    pub fn server_meta(&self) -> Option<ServerMeta> {
        if let Some(m) = self.inner.server_meta.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            return Some(m);
        }
        self.inner.store.kv_get("server_meta").ok().flatten().and_then(|s| serde_json::from_str(&s).ok())
    }

    // ------------------------------------------------------------------ sign-in

    /// Contacts the server without signing in (shows name, version, max upload, setup state).
    pub async fn probe_server(&self, server: &str, pin: Option<&str>) -> Result<ServerMeta> {
        let base = normalize_server_url(server, self.inner.cfg.allow_insecure_http)?;
        let api = Api::new(&base, pin.map(str::to_string), self.inner.secrets.clone(), &self.inner.cfg.client_version)?;
        let meta = api.meta().await?;
        if rules::version_lt(&self.inner.cfg.client_version, &meta.min_client_version) {
            return Err(ClientError::Api {
                code: ErrorCode::ClientTooOld,
                message: format!("This app ({}) is older than the minimum version {} the server accepts. Please update.", self.inner.cfg.client_version, meta.min_client_version),
            });
        }
        *self.inner.server_meta.lock().unwrap_or_else(|e| e.into_inner()) = Some(meta.clone());
        Ok(meta)
    }

    /// X-01: fetch the certificate fingerprint of an https server so the user can verify and pin it.
    pub async fn probe_certificate(&self, server: &str) -> Result<crate::tls::CertProbe> {
        let base = normalize_server_url(server, self.inner.cfg.allow_insecure_http)?;
        let u = url::Url::parse(&base).map_err(|e| ClientError::Local(e.to_string()))?;
        if u.scheme() != "https" {
            return Err(ClientError::Local("only https servers have a certificate".into()));
        }
        crate::tls::probe_certificate(u.host_str().unwrap_or(""), u.port().unwrap_or(443)).await
    }

    pub async fn login(&self, server: &str, username: &str, password: &str, pin: Option<&str>) -> Result<Me> {
        let base = normalize_server_url(server, self.inner.cfg.allow_insecure_http)?;
        let api = Arc::new(Api::new(&base, pin.map(str::to_string), self.inner.secrets.clone(), &self.inner.cfg.client_version)?);
        let resp = api
            .login(&LoginRequest {
                username: username.trim().to_string(),
                password: password.to_string(),
                device_name: self.inner.cfg.device_name.clone(),
                client_version: self.inner.cfg.client_version.clone(),
            })
            .await?;
        let me = self.inner.finish_login(api, &base, resp, pin.map(str::to_string))?;
        self.inner.refresh_meta().await;
        Ok(me)
    }

    /// Signs in with the one-time link or code an admin created. `server` is needed only when
    /// the pasted text is a bare token.
    pub async fn login_with_link(&self, pasted: &str, server: Option<&str>, pin: Option<&str>) -> Result<Me> {
        let parsed = rules::parse_login_link(pasted).ok_or_else(|| ClientError::Local("That link or code does not look right. Paste it exactly as your administrator sent it.".into()))?;
        let srv = parsed
            .server
            .clone()
            .or_else(|| server.filter(|s| !s.trim().is_empty()).map(str::to_string))
            .ok_or_else(|| ClientError::Local("Enter the server address, or paste the full link.".into()))?;
        let base = normalize_server_url(&srv, self.inner.cfg.allow_insecure_http)?;
        let api = Arc::new(Api::new(&base, pin.map(str::to_string), self.inner.secrets.clone(), &self.inner.cfg.client_version)?);
        let resp = api
            .login_link(&LinkLoginRequest {
                token: parsed.token,
                device_name: self.inner.cfg.device_name.clone(),
                client_version: self.inner.cfg.client_version.clone(),
            })
            .await?;
        let me = self.inner.finish_login(api, &base, resp, pin.map(str::to_string))?;
        self.inner.refresh_meta().await;
        Ok(me)
    }

    /// Ends this device's session on the server and erases local data.
    ///
    /// Only admins and members with "Privacy options" set to Allow may do this; for everyone else the server
    /// answers `forbidden` and nothing is wiped.
    pub async fn logout(&self) -> Result<()> {
        if let Ok(api) = self.inner.api() {
            match api.post::<serde_json::Value, _>("/logout", &serde_json::json!({})).await {
                Err(e) if e.code() == Some(ErrorCode::Forbidden) => return Err(e),
                _ => {}
            }
        }
        self.inner.wipe_local(None, "You signed out.".into());
        Ok(())
    }

    /// Starts the background sync (idempotent).
    pub fn start(&self) {
        let mut tasks = self.inner.tasks.lock().unwrap_or_else(|e| e.into_inner());
        tasks.retain(|t| !t.is_finished());
        if !tasks.is_empty() || self.inner.api().is_err() {
            return;
        }
        let a = self.inner.clone();
        tasks.push(self.inner.handle.spawn(async move { a.sync_loop().await }));
        let b = self.inner.clone();
        tasks.push(self.inner.handle.spawn(async move { b.outbox_loop().await }));
    }

    /// Stops sync tasks without signing out (app closing / backgrounded on mobile, X-03).
    pub fn stop(&self) {
        self.inner.abort_tasks();
        self.inner.set_state(ConnState::Offline { retry_in_secs: 0 });
    }

    /// Forces an immediate reconnect attempt (network came back).
    pub fn reconnect_now(&self) {
        self.inner.wake.notify_one();
    }

    // ------------------------------------------------------------------ reads

    /// Conversations the person may see right now: locked ones are left out while App Lock is engaged.
    pub fn conversations(&self) -> Vec<ConversationSummary> {
        let mut v = self.inner.store.conversations().unwrap_or_default();
        v.retain(|c| !self.inner.applock.is_hidden(&c.conversation.id));
        v
    }

    /// The App Lock (PIN, locked conversations, idle timeout).
    pub fn applock(&self) -> &crate::applock::AppLock {
        &self.inner.applock
    }

    fn lock_changed(&self) {
        self.inner.emit(ClientEvent::LockChanged);
    }

    /// Locks the app now: every locked conversation disappears from every list, search and count.
    pub fn lock_app(&self) {
        let was = self.inner.applock.generation();
        self.inner.applock.lock();
        if self.inner.applock.generation() != was {
            self.lock_changed();
        }
    }

    /// Checks the PIN; a correct one shows the locked conversations again at once.
    pub fn unlock_app(&self, pin: &str, now_ms: i64) -> crate::applock::Unlock {
        let r = self.inner.applock.unlock(pin, now_ms);
        if r == crate::applock::Unlock::Ok {
            self.lock_changed();
        }
        r
    }

    /// Marks (or unmarks) a conversation as locked. A newly locked conversation disappears at once.
    pub fn set_conversation_locked(&self, conv_id: &str, locked: bool) -> std::result::Result<(), crate::applock::PinError> {
        self.inner.applock.set_conv_locked(conv_id, locked)?;
        self.lock_changed();
        Ok(())
    }

    /// The idle and background timers; true when this call locked the app.
    pub fn lock_timers(&self, background_changed: Option<bool>, now_ms: i64) -> bool {
        let a = &self.inner.applock;
        let mut locked = false;
        if let Some(bg) = background_changed {
            locked |= a.set_background(bg, now_ms);
        }
        locked |= a.check_idle(now_ms);
        if locked {
            self.lock_changed();
        }
        locked
    }

    /// 1.4.1: every conversation that can receive something shared from another app or forwarded, newest first. Locked
    /// conversations are included on purpose (and the lock state is not looked at): the person picks a destination, nothing is
    /// shown from inside it.
    pub fn share_targets(&self) -> Vec<ConversationSummary> {
        let mut v = self.inner.store.conversations().unwrap_or_default();
        v.retain(|c| !c.conversation.archived);
        v
    }

    /// 1.4.1: sends shared text and files into a conversation WITHOUT opening it and without touching the App Lock: a locked
    /// conversation stays locked and hidden. (The caption is the text; files go in one message.)
    pub fn share_to(&self, conv_id: &str, text: &str, files: &[PathBuf]) -> Result<()> {
        let text = text.trim();
        if text.is_empty() && files.is_empty() {
            return Ok(());
        }
        if !files.is_empty() {
            self.send_files(conv_id, text, files, SendOptions::default())?;
        } else {
            self.send_text(conv_id, text, SendOptions::default())?;
        }
        Ok(())
    }

    /// True while this conversation must not appear anywhere (lists, search, counts, threads, drafts).
    pub fn is_hidden(&self, conv_id: &str) -> bool {
        self.inner.applock.is_hidden(conv_id)
    }

    /// True for conversations marked as locked, even while the app is unlocked: notifications about them never show
    /// the conversation name or the text.
    pub fn is_conv_locked(&self, conv_id: &str) -> bool {
        self.inner.applock.is_conv_locked(conv_id)
    }

    pub fn users(&self) -> Vec<User> {
        self.inner.store.users().unwrap_or_default()
    }

    pub fn user(&self, id: &str) -> Option<User> {
        self.inner.store.user(id).ok().flatten()
    }

    pub fn timeline(&self, conv_id: &str, limit: usize, before_seq: Option<i64>) -> Vec<TimelineItem> {
        if self.is_hidden(conv_id) {
            return vec![];
        }
        let me = self.my_id().unwrap_or_default();
        self.inner.store.timeline(conv_id, limit, before_seq, &me).unwrap_or_default()
    }

    /// Display name of a conversation: the other person for DMs, "Notes to self", or the channel name.
    pub fn conversation_title(&self, cs: &ConversationSummary) -> String {
        let me = self.my_id().unwrap_or_default();
        match cs.conversation.kind {
            ConversationKind::SelfNote => "Notes to self".into(),
            ConversationKind::Dm => cs
                .conversation
                .members
                .iter()
                .find(|m| m.user_id != me)
                .and_then(|m| self.user(&m.user_id))
                .map(|u| u.display_name)
                .unwrap_or_else(|| "Direct message".into()),
            _ => cs.conversation.name.clone().unwrap_or_else(|| "Conversation".into()),
        }
    }

    pub fn display_name(&self, user_id: &str) -> String {
        self.user(user_id).map(|u| u.display_name).unwrap_or_else(|| "Unknown".into())
    }

    /// M-08 mirror of the server edit rule, for hiding the Edit action. The server still decides.
    pub fn can_edit(&self, m: &Message) -> bool {
        let Some(me) = self.my_id() else { return false };
        if m.sender_id != me || m.deleted || m.id.starts_with("local:") {
            return false;
        }
        let Ok(Some(cs)) = self.inner.store.conversation(&m.conv_id) else { return false };
        if cs.conversation.archived {
            return false;
        }
        rules::edit_allowed(cs.conversation.kind, self.inner.store.other_posted_after(&m.conv_id, m.seq, &me))
    }

    pub fn unread_total(&self) -> i64 {
        self.conversations().iter().map(|c| c.unread).sum()
    }

    // ------------------------------------------------------------------ actions

    /// Queues a message in the outbox (shown immediately as "sending") and returns its client id.
    pub fn send_text(&self, conv_id: &str, body: &str, opts: SendOptions) -> Result<String> {
        self.send(conv_id, body, vec![], opts)
    }

    /// Queues a message with file attachments (F-01); files upload in the background.
    pub fn send_files(&self, conv_id: &str, body: &str, paths: &[PathBuf], opts: SendOptions) -> Result<String> {
        let list: Vec<(PathBuf, Option<proto::AttachmentMeta>)> = paths.iter().map(|p| (p.clone(), None)).collect();
        self.send_files_with(conv_id, body, &list, opts)
    }

    /// Like `send_files`, with media details the app already has (a video's poster frame and size). Audio files get
    /// their duration and waveform computed here when no details are given (M-30).
    pub fn send_files_with(&self, conv_id: &str, body: &str, paths: &[(PathBuf, Option<proto::AttachmentMeta>)], opts: SendOptions) -> Result<String> {
        let mut files = vec![];
        for (p, given) in paths {
            let meta = std::fs::metadata(p)?;
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
            if let Some(m) = self.server_meta() {
                if meta.len() > m.max_upload_bytes {
                    return Err(ClientError::Api {
                        code: ErrorCode::FileTooLarge,
                        message: format!("{name} is {} MB; the limit is {} MB", meta.len() / 1024 / 1024, m.max_upload_bytes / 1024 / 1024),
                    });
                }
            }
            let ct = guess_content_type(&name);
            let media_meta = given.clone().or_else(|| {
                if ct.starts_with("audio/") {
                    crate::media::analyze_audio(p)
                } else if ct.starts_with("video/") {
                    crate::media::analyze_video(p)
                } else {
                    None
                }
            });
            files.push(OutFile { path: p.to_string_lossy().to_string(), content_type: ct, name, size: meta.len(), uploaded_id: None, meta: media_meta });
        }
        self.send(conv_id, body, files, opts)
    }

    fn send(&self, conv_id: &str, body: &str, files: Vec<OutFile>, opts: SendOptions) -> Result<String> {
        if body.trim().is_empty() && files.is_empty() {
            return Err(ClientError::Local("Type a message first.".into()));
        }
        let client_id = uuid::Uuid::now_v7().simple().to_string();
        // Resolve @mentions to ids from the cached members of this conversation.
        let mentions = self.resolve_mentions(conv_id, body);
        self.inner.store.outbox_add(&OutboxItem {
            client_id: client_id.clone(),
            conv_id: conv_id.to_string(),
            body: body.trim_end().to_string(),
            mentions,
            thread_root: opts.thread_root,
            reply_to: opts.reply_to,
            files,
            state: "pending".into(),
            attempts: 0,
            last_error: None,
            created_ms: now_ms(),
        })?;
        let _ = self.inner.store.set_draft(conv_id, "", now_ms());
        self.inner.emit(ClientEvent::Changed { conv_ids: vec![conv_id.to_string()], users: false, me: false });
        self.inner.outbox_notify.notify_one();
        Ok(client_id)
    }

    /// M-16: structured mention ids for `@username` tokens of members of the conversation.
    pub fn resolve_mentions(&self, conv_id: &str, body: &str) -> Vec<String> {
        let names = crate::logic::markup::mentioned_usernames(body);
        if names.is_empty() {
            return vec![];
        }
        let Ok(Some(cs)) = self.inner.store.conversation(conv_id) else { return vec![] };
        let mut out = vec![];
        for n in names {
            for m in &cs.conversation.members {
                if let Some(u) = self.user(&m.user_id) {
                    if u.username.eq_ignore_ascii_case(&n) && !out.contains(&u.id) {
                        out.push(u.id);
                    }
                }
            }
        }
        out
    }

    pub fn retry_send(&self, client_id: &str) -> Result<()> {
        self.inner.store.outbox_retry(client_id)?;
        self.inner.outbox_notify.notify_one();
        Ok(())
    }

    /// F-01: upload progress of a queued message with files, `None` when it is not uploading.
    pub fn upload_progress(&self, client_id: &str) -> Option<UploadState> {
        self.inner.uploads.lock().unwrap_or_else(|e| e.into_inner()).get(client_id).cloned()
    }

    /// Cancels a queued or uploading message (F-01).
    pub fn cancel_send(&self, client_id: &str) -> Result<()> {
        self.inner.uploads.lock().unwrap_or_else(|e| e.into_inner()).remove(client_id);
        self.inner.cancelled.lock().unwrap_or_else(|e| e.into_inner()).insert(client_id.to_string());
        self.inner.store.outbox_remove(client_id)?;
        self.inner.emit(ClientEvent::Changed { conv_ids: vec![], users: false, me: false });
        Ok(())
    }

    /// C-06: owner/admin deletes a channel or group chat for everyone (messages, files, pins, saved items). The server
    /// removes it from every member's list with a `conversation_remove` event.
    pub async fn delete_conversation(&self, conv_id: &str) -> Result<()> {
        let _: serde_json::Value = self.api()?.delete(&format!("/admin/conversations/{conv_id}")).await?;
        self.inner.emit(ClientEvent::Changed { conv_ids: vec![conv_id.to_string()], users: false, me: false });
        Ok(())
    }

    pub async fn edit_message(&self, id: &str, body: &str) -> Result<Message> {
        let conv = self.inner.store.message(id)?.map(|m| m.conv_id).unwrap_or_default();
        let mentions = self.resolve_mentions(&conv, body);
        let m: Message = self.api()?.patch(&format!("/messages/{id}"), &EditMessage { body: body.to_string(), mentions }).await?;
        self.inner.store.put_messages(std::slice::from_ref(&m))?;
        self.inner.emit(ClientEvent::Changed { conv_ids: vec![m.conv_id.clone()], users: false, me: false });
        Ok(m)
    }

    pub async fn delete_message(&self, id: &str) -> Result<Message> {
        let m: Message = self.api()?.delete(&format!("/messages/{id}")).await?;
        self.inner.store.put_messages(std::slice::from_ref(&m))?;
        self.inner.emit(ClientEvent::Changed { conv_ids: vec![m.conv_id.clone()], users: false, me: false });
        Ok(m)
    }

    pub async fn react(&self, id: &str, emoji: &str, add: bool) -> Result<Message> {
        let enc = url::form_urlencoded::byte_serialize(emoji.as_bytes()).collect::<String>();
        let path = format!("/messages/{id}/reactions/{enc}");
        let m: Message = if add { self.api()?.put(&path).await? } else { self.api()?.delete(&path).await? };
        self.inner.store.put_messages(std::slice::from_ref(&m))?;
        self.inner.emit(ClientEvent::Changed { conv_ids: vec![m.conv_id.clone()], users: false, me: false });
        Ok(m)
    }

    pub async fn mark_read(&self, conv_id: &str) -> Result<()> {
        let Some(cs) = self.inner.store.conversation(conv_id)? else { return Ok(()) };
        if cs.unread == 0 && !cs.marked_unread {
            return Ok(());
        }
        let seq = self.inner.store.newest_seq(conv_id).unwrap_or(cs.conversation.last_seq).max(cs.conversation.last_seq);
        let s: ConversationSummary = self.api()?.post(&format!("/conversations/{conv_id}/read"), &MarkReadRequest { seq }).await?;
        self.inner.store.put_conversation(&s)?;
        self.inner.emit(ClientEvent::Changed { conv_ids: vec![conv_id.to_string()], users: false, me: false });
        Ok(())
    }

    pub async fn mark_unread(&self, conv_id: &str) -> Result<()> {
        let s: ConversationSummary = self.api()?.post(&format!("/conversations/{conv_id}/unread"), &serde_json::json!({})).await?;
        self.inner.store.put_conversation(&s)?;
        self.inner.emit(ClientEvent::Changed { conv_ids: vec![conv_id.to_string()], users: false, me: false });
        Ok(())
    }

    /// M-02: start (or open) a direct message with someone.
    pub async fn open_dm(&self, user_id: &str) -> Result<String> {
        let s: ConversationSummary = self.api()?.post("/conversations/dm", &CreateDm { user_id: user_id.to_string() }).await?;
        self.inner.store.put_conversation(&s)?;
        self.inner.emit(ClientEvent::Changed { conv_ids: vec![s.conversation.id.clone()], users: false, me: false });
        Ok(s.conversation.id)
    }

    /// Creates a channel (the server only allows admins to) and returns its id.
    pub async fn create_channel(&self, name: &str) -> Result<String> {
        let s: ConversationSummary = self
            .api()?
            .post("/conversations", &CreateConversation { kind: ConversationKind::Channel, name: name.into(), topic: None, members: vec![] })
            .await?;
        self.inner.store.put_conversation(&s)?;
        self.inner.emit(ClientEvent::Changed { conv_ids: vec![s.conversation.id.clone()], users: false, me: false });
        Ok(s.conversation.id)
    }

    /// M-04: the notes-to-self conversation.
    pub fn self_conversation(&self) -> Option<ConversationSummary> {
        self.conversations().into_iter().find(|c| c.conversation.kind == ConversationKind::SelfNote)
    }

    /// M-29: loads one page of older history. `before_seq` is the oldest sequence number shown.
    /// Returns how many messages were added; check `has_more_before` for more.
    pub async fn load_older(&self, conv_id: &str, before_seq: Option<i64>) -> Result<usize> {
        let before = before_seq.or_else(|| self.inner.store.oldest_seq(conv_id));
        let path = match before {
            Some(b) => format!("/conversations/{conv_id}/messages?before={b}&limit=50"),
            None => format!("/conversations/{conv_id}/messages?limit=50"),
        };
        let h: HistoryResponse = self.api()?.get(&path).await?;
        self.inner.store.put_messages(&h.messages)?;
        self.inner.store.set_has_more_before(conv_id, h.has_more_before)?;
        self.inner.emit(ClientEvent::Changed { conv_ids: vec![conv_id.to_string()], users: false, me: false });
        Ok(h.messages.len())
    }

    pub fn has_more_before(&self, conv_id: &str) -> bool {
        self.inner.store.has_more_before(conv_id)
    }

    /// M-29: jump to a date (UTC ms). Fetches a window around it and returns its first message.
    pub async fn jump_to_date(&self, conv_id: &str, ts_ms: i64) -> Result<Option<Message>> {
        let h: HistoryResponse = self.api()?.get(&format!("/conversations/{conv_id}/messages?around_ts={ts_ms}&limit=60")).await?;
        self.inner.store.put_messages(&h.messages)?;
        self.inner.emit(ClientEvent::Changed { conv_ids: vec![conv_id.to_string()], users: false, me: false });
        Ok(h.messages.into_iter().find(|m| m.ts >= ts_ms))
    }

    /// Server-side search (members-only results).
    pub async fn search(&self, q: &str) -> Result<SearchResponse> {
        let enc = url::form_urlencoded::byte_serialize(q.as_bytes()).collect::<String>();
        let r: SearchResponse = self.api()?.get(&format!("/search?q={enc}")).await?;
        Ok(self.without_hidden_hits(r))
    }

    /// Drops search results that belong to hidden (locked) conversations.
    pub(crate) fn without_hidden_hits(&self, mut r: SearchResponse) -> SearchResponse {
        r.hits.retain(|h| !self.is_hidden(&h.message.conv_id));
        r.files.retain(|f| !self.is_hidden(&f.conv_id));
        r.channels.retain(|c| !self.is_hidden(&c.conversation.id));
        r
    }

    pub fn search_local(&self, q: &str) -> Vec<Message> {
        let mut v = self.inner.store.search_local(q, 50).unwrap_or_default();
        v.retain(|m| !self.is_hidden(&m.conv_id));
        v
    }

    /// Typing indicator, throttled to once per 3 seconds per conversation (M-25).
    pub fn typing(&self, conv_id: &str) {
        {
            let mut t = self.inner.typing_sent.lock().unwrap_or_else(|e| e.into_inner());
            if t.get(conv_id).map(|i| i.elapsed() < Duration::from_secs(3)).unwrap_or(false) {
                return;
            }
            t.insert(conv_id.to_string(), Instant::now());
        }
        self.inner.send_ws(ClientWsMsg::Typing { conv_id: conv_id.to_string() });
    }

    /// P-02: report the user going idle / active so the server shows "away".
    pub fn set_idle(&self, idle: bool) {
        self.inner.send_ws(ClientWsMsg::Idle { idle });
    }

    pub async fn set_presence_pref(&self, pref: &str) -> Result<Me> {
        self.update_profile(UpdateMeRequest { presence_pref: Some(pref.into()), ..Default::default() }).await
    }

    pub async fn update_profile(&self, req: UpdateMeRequest) -> Result<Me> {
        let me: Me = self.api()?.patch("/me", &req).await?;
        self.inner.store.set_me(&me)?;
        self.inner.emit(ClientEvent::Changed { conv_ids: vec![], users: true, me: true });
        Ok(me)
    }

    /// The file of a downloaded picture (`avatars/<attachment id>`), when it is on disk.
    fn avatar_file(&self, att: &str) -> Option<PathBuf> {
        if att.is_empty() || att.contains('/') || att.contains("..") {
            return None;
        }
        let p = self.inner.cfg.data_dir.join("avatars").join(att);
        p.is_file().then_some(p)
    }

    /// The private picture this account chose for a user (`kind` "user") or a conversation (`kind` "conv"), if any.
    pub fn avatar_override(&self, kind: &str, target: &str) -> Option<String> {
        self.inner.store.avatar_overrides().into_iter().find(|o| o.kind == kind && o.target == target).map(|o| o.attachment)
    }

    /// P-03 and 1.4.1: the picture shown for a user. Order: the picture this account chose for them, then their own profile
    /// photo (when both are downloaded), else None (the app draws initials).
    pub fn avatar_path(&self, user_id: &str) -> Option<PathBuf> {
        if let Some(p) = self.avatar_override("user", user_id).and_then(|a| self.avatar_file(&a)) {
            return Some(p);
        }
        let att = self.inner.store.user(user_id).ok().flatten()?.avatar?;
        self.avatar_file(&att)
    }

    /// 1.4.1: the picture shown for a conversation. Order: the picture this account chose for it, then the channel's or
    /// group's own picture. (A direct message uses `avatar_path` of the other person.)
    pub fn conversation_avatar_path(&self, conv_id: &str) -> Option<PathBuf> {
        if let Some(p) = self.avatar_override("conv", conv_id).and_then(|a| self.avatar_file(&a)) {
            return Some(p);
        }
        let att = self.inner.store.conversation(conv_id).ok().flatten()?.conversation.avatar?;
        self.avatar_file(&att)
    }

    /// 1.4.1: true when the picture shown for this user or conversation is one this account chose (the UI offers "Remove").
    pub fn has_avatar_override(&self, kind: &str, target: &str) -> bool {
        self.avatar_override(kind, target).is_some()
    }

    /// 1.4.1: sets the picture everyone sees for a channel or group DM (owners and admins). Pass a pre-cropped image file.
    pub async fn set_conversation_avatar(&self, conv_id: &str, path: &Path) -> Result<ConversationSummary> {
        let (path, name) = self.square_for_upload(path);
        let up = self.upload_path(&path, &name, Some("avatar"), None).await?;
        self.put_conv_avatar(conv_id, &up.attachment.id).await
    }

    /// 1.4.1: pictures are cut to a centred square (at most 512 px) before they are uploaded as a profile, channel or private
    /// picture. A file that cannot be decoded is uploaded as it is (the server decides).
    fn square_for_upload(&self, path: &Path) -> (PathBuf, String) {
        let orig_name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "avatar.png".into());
        let Ok(img) = image::open(path) else { return (path.to_path_buf(), orig_name) };
        let (w, h) = (img.width(), img.height());
        if w == 0 || h == 0 {
            return (path.to_path_buf(), orig_name);
        }
        let side = w.min(h);
        let sq = img.crop_imm((w - side) / 2, (h - side) / 2, side, side);
        let sq = if side > 512 { sq.resize_exact(512, 512, image::imageops::FilterType::Triangle) } else { sq };
        let out = self.inner.cfg.data_dir.join(format!("avatar-upload-{}.jpg", std::process::id()));
        match sq.to_rgb8().save_with_format(&out, image::ImageFormat::Jpeg) {
            Ok(()) => (out, "avatar.jpg".into()),
            Err(_) => (path.to_path_buf(), orig_name),
        }
    }

    /// 1.4.1: removes the picture everyone sees for a channel or group DM.
    pub async fn clear_conversation_avatar(&self, conv_id: &str) -> Result<ConversationSummary> {
        self.put_conv_avatar(conv_id, "").await
    }

    async fn put_conv_avatar(&self, conv_id: &str, att: &str) -> Result<ConversationSummary> {
        let cs: ConversationSummary = self.api()?.put_json(&format!("/conversations/{conv_id}/avatar"), &SetConvAvatar { avatar: att.to_string() }).await?;
        self.inner.store.put_conversation(&cs)?;
        self.inner.sync_avatars().await;
        self.inner.emit(ClientEvent::Changed { conv_ids: vec![conv_id.to_string()], users: true, me: false });
        Ok(cs)
    }

    /// 1.4.1: a private picture for another user (`kind` "user") or a conversation (`kind` "conv"). Only this account sees it;
    /// it is stored on the server for this account and synced to its other devices.
    pub async fn set_avatar_override(&self, kind: &str, target: &str, path: &Path) -> Result<()> {
        let (path, name) = self.square_for_upload(path);
        let up = self.upload_path(&path, &name, Some("avatar_private"), None).await?;
        let o: AvatarOverride = self.api()?.put_json(&format!("/me/avatar-overrides/{kind}/{target}"), &SetAvatarOverride { attachment: up.attachment.id }).await?;
        let mut all = self.inner.store.avatar_overrides();
        all.retain(|x| !(x.kind == kind && x.target == target));
        all.push(o);
        self.inner.store.put_avatar_overrides(&all)?;
        self.inner.sync_avatars().await;
        self.inner.emit(ClientEvent::Changed { conv_ids: if kind == "conv" { vec![target.to_string()] } else { vec![] }, users: true, me: false });
        Ok(())
    }

    /// 1.4.1: removes the private picture; the person's or channel's own picture (or initials) shows again.
    pub async fn clear_avatar_override(&self, kind: &str, target: &str) -> Result<()> {
        let _: serde_json::Value = self.api()?.delete(&format!("/me/avatar-overrides/{kind}/{target}")).await?;
        let mut all = self.inner.store.avatar_overrides();
        all.retain(|x| !(x.kind == kind && x.target == target));
        self.inner.store.put_avatar_overrides(&all)?;
        self.inner.emit(ClientEvent::Changed { conv_ids: if kind == "conv" { vec![target.to_string()] } else { vec![] }, users: true, me: false });
        Ok(())
    }

    /// P-03: downloads the photos still missing (the client does this by itself after sign-in and when people change).
    pub async fn refresh_avatars(&self) {
        self.inner.sync_avatars().await
    }

    /// P-03: upload and set a profile photo. Pass pre-cropped image bytes' file path.
    pub async fn set_avatar(&self, path: &Path) -> Result<Me> {
        let (path, name) = self.square_for_upload(path);
        let up = self.upload_path(&path, &name, Some("avatar"), None).await?;
        let me = self.update_profile(UpdateMeRequest { avatar: Some(up.attachment.id), ..Default::default() }).await?;
        let _ = self.inner.store.put_users(std::slice::from_ref(&me.user));
        self.inner.sync_avatars().await;
        Ok(me)
    }

    pub async fn change_password(&self, current: Option<&str>, new: &str) -> Result<()> {
        let _: serde_json::Value = self
            .api()?
            .post("/me/password", &ChangePasswordRequest { current_password: current.map(str::to_string), new_password: new.to_string() })
            .await?;
        Ok(())
    }

    pub async fn sessions(&self) -> Result<Vec<SessionInfo>> {
        self.api()?.get("/sessions").await
    }

    /// True when the signed-in user may use the self-service sign-out controls (admins, or members with
    /// "Privacy options" set to Allow). Follows live `me_update` events.
    pub fn can_sign_out(&self) -> bool {
        self.me().map(|m| m.user.role.is_admin() || m.privacy_controls_enabled).unwrap_or(false)
    }

    /// Signs out of every device including this one, then erases local data.
    pub async fn logout_all(&self) -> Result<()> {
        let _: serde_json::Value = self.api()?.post("/sessions/revoke-all", &serde_json::json!({})).await?;
        self.inner.wipe_local(None, "You signed out.".into());
        Ok(())
    }

    pub async fn revoke_other_sessions(&self) -> Result<()> {
        let _: serde_json::Value = self.api()?.post("/sessions/revoke-others", &serde_json::json!({})).await?;
        Ok(())
    }

    pub fn set_draft(&self, conv_id: &str, body: &str) {
        let _ = self.inner.store.set_draft(conv_id, body, now_ms());
    }

    pub fn draft(&self, conv_id: &str) -> String {
        self.inner.store.draft(conv_id).unwrap_or_default()
    }

    pub fn api(&self) -> Result<Arc<Api>> {
        self.inner.api()
    }

    // ------------------------------------------------------------------ files

    async fn upload_path(&self, path: &Path, name: &str, purpose: Option<&str>, progress: Option<UploadCtx>) -> Result<UploadResponse> {
        let api = self.api()?;
        let file = tokio::fs::File::open(path).await?;
        let total = file.metadata().await?.len();
        let sent = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let stream = tokio_util::io::ReaderStream::with_capacity(file, 64 * 1024);
        let cid = progress.as_ref().map(|p| p.client_id.clone());
        let ctx = progress;
        let cancelled = self.inner.clone();
        let sent2 = sent.clone();
        let mapped = stream.map(move |chunk| {
            if let Some(c) = &cid {
                if cancelled.cancelled.lock().unwrap_or_else(|e| e.into_inner()).contains(c) {
                    return Err(std::io::Error::other("cancelled"));
                }
            }
            if let Ok(b) = &chunk {
                let n = sent2.fetch_add(b.len() as u64, Ordering::Relaxed) + b.len() as u64;
                if let (Some(c), Some(x)) = (&cid, &ctx) {
                    if n == total || n % (256 * 1024) < 64 * 1024 {
                        let st = UploadState { file_name: x.name.clone(), file_index: x.index, file_count: x.count, sent: x.base + n, total: x.total_all };
                        let mut map = x.inner.uploads.lock().unwrap_or_else(|e| e.into_inner());
                        // A cancelled message must not reappear in the progress map.
                        if !x.inner.cancelled.lock().unwrap_or_else(|e| e.into_inner()).contains(c) {
                            map.insert(c.clone(), st.clone());
                            drop(map);
                            x.inner.emit(ClientEvent::UploadProgress { client_id: c.clone(), sent: st.sent, total: st.total });
                        }
                    }
                }
            }
            chunk
        });
        api.upload(name, &guess_content_type(name), total, purpose, reqwest::Body::wrap_stream(mapped)).await
    }

    /// Path of an already downloaded attachment (no network), for instant previews.
    pub fn cached_attachment_path(&self, att: &Attachment) -> Option<PathBuf> {
        let safe: String = att.name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') { c } else { '_' }).collect();
        let p = self.inner.cfg.data_dir.join("files").join(format!("{}_{}", att.id, safe));
        p.exists().then_some(p)
    }

    /// F-03: where the preview PNG of a PDF is cached, if it has been fetched.
    pub fn pdf_thumb_path(&self, att: &Attachment) -> Option<PathBuf> {
        let p = self.inner.cfg.data_dir.join("thumbs").join(format!("{}.png", att.id));
        p.exists().then_some(p)
    }

    /// F-03: fetches the server-drawn preview of the first page of a PDF and caches it. A PDF the server cannot draw
    /// leaves an empty marker so the app does not ask again (the message then shows a plain file card).
    pub async fn fetch_pdf_thumb(&self, att: &Attachment) -> Result<PathBuf> {
        let dir = self.inner.cfg.data_dir.join("thumbs");
        std::fs::create_dir_all(&dir)?;
        let dest = dir.join(format!("{}.png", att.id));
        if dest.exists() {
            return Ok(dest);
        }
        let none = dir.join(format!("{}.none", att.id));
        if none.exists() {
            return Err(ClientError::Local("No preview".into()));
        }
        let api = self.api()?;
        let url = format!("{}/thumb", api.download_url(&att.id).split('?').next().unwrap_or_default());
        match api.exec(|c| c.get(&url)).await {
            Ok(resp) => {
                let bytes = resp.bytes().await.map_err(|e| ClientError::Network(e.to_string()))?;
                let tmp = dest.with_extension("part");
                std::fs::write(&tmp, &bytes)?;
                std::fs::rename(&tmp, &dest)?;
                Ok(dest)
            }
            Err(e) => {
                if matches!(e.code(), Some(proto::ErrorCode::NotFound)) {
                    let _ = std::fs::write(&none, b"");
                }
                Err(e)
            }
        }
    }

    /// F-03: downloads (and caches) an attachment, returning the local file path.
    pub async fn download_attachment(&self, att: &Attachment) -> Result<PathBuf> {
        if att.removed {
            return Err(ClientError::Local("File removed".into()));
        }
        let dir = self.inner.cfg.data_dir.join("files");
        std::fs::create_dir_all(&dir)?;
        let safe: String = att.name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') { c } else { '_' }).collect();
        let dest = dir.join(format!("{}_{}", att.id, safe));
        if dest.exists() {
            // Touch for LRU.
            let _ = std::fs::OpenOptions::new().append(true).open(&dest);
            return Ok(dest);
        }
        let api = self.api()?;
        let url = api.download_url(&att.id);
        let resp = api.exec(|c| c.get(&url)).await?;
        let tmp = dest.with_extension("part");
        let mut f = tokio::fs::File::create(&tmp).await?;
        let mut stream = resp.bytes_stream();
        use tokio::io::AsyncWriteExt;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| ClientError::Network(e.to_string()))?;
            f.write_all(&chunk).await?;
        }
        f.flush().await?;
        drop(f);
        std::fs::rename(&tmp, &dest)?;
        let _ = self.trim_cache(500 * 1024 * 1024);
        Ok(dest)
    }

    /// Keeps the downloaded-file cache under `max_bytes`, deleting the least recently used first.
    pub fn trim_cache(&self, max_bytes: u64) -> Result<()> {
        let dir = self.inner.cfg.data_dir.join("files");
        let mut files: Vec<(PathBuf, u64, std::time::SystemTime)> = std::fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let m = e.metadata().ok()?;
                Some((e.path(), m.len(), m.modified().ok()?))
            })
            .collect();
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        files.sort_by_key(|f| f.2);
        for (p, len, _) in files {
            if total <= max_bytes {
                break;
            }
            if std::fs::remove_file(&p).is_ok() {
                total -= len;
            }
        }
        Ok(())
    }

    /// A short-lived URL for browsers/webviews that cannot send headers.
    pub async fn signed_file_url(&self, id: &str) -> Result<String> {
        #[derive(serde::Deserialize)]
        struct S {
            url: String,
        }
        let s: S = self.api()?.get(&format!("/files/{id}/url")).await?;
        Ok(format!("{}{}", self.api()?.base, s.url))
    }

    /// Signed-in user, as a quick reachability check; surfaces wipe errors.
    pub async fn refresh_me(&self) -> Result<Me> {
        let me: Me = self.api()?.get("/me").await?;
        self.inner.store.set_me(&me)?;
        Ok(me)
    }
}

#[derive(Debug, Clone, Default)]
pub struct SendOptions {
    pub thread_root: Option<String>,
    pub reply_to: Option<String>,
}

pub fn guess_content_type(name: &str) -> String {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" | "log" | "md" => "text/plain",
        "json" => "application/json",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "m4a" | "aac" => "audio/aac",
        "wav" => "audio/wav",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
    .to_string()
}

impl Inner {
    /// P-03: downloads the profile photos of everyone in the cache that are not on disk yet.
    /// A photo is stored as `avatars/<attachment id>`; a changed photo has a new id, so nothing is ever stale.
    pub(crate) async fn sync_avatars(self: &Arc<Self>) {
        let dir = self.cfg.data_dir.join("avatars");
        let _ = std::fs::create_dir_all(&dir);
        // Everything shown as a picture: profile photos, channel and group pictures, and this account's private pictures.
        let mut wanted: Vec<String> = self.store.users().unwrap_or_default().into_iter().filter_map(|u| u.avatar).collect();
        wanted.extend(self.store.conversations().unwrap_or_default().into_iter().filter_map(|c| c.conversation.avatar));
        wanted.extend(self.store.avatar_overrides().into_iter().map(|o| o.attachment));
        let mut changed = false;
        for att in wanted {
            if att.is_empty() || att.contains('/') || att.contains("..") || dir.join(&att).exists() {
                continue;
            }
            let Ok(api) = self.api() else { return };
            let url = api.download_url(&att);
            let Ok(resp) = api.exec(|c| c.get(&url)).await else { continue };
            let Ok(bytes) = resp.bytes().await else { continue };
            if bytes.len() > 8 * 1024 * 1024 {
                continue;
            }
            let tmp = dir.join(format!("{att}.part"));
            if std::fs::write(&tmp, &bytes).is_ok() && std::fs::rename(&tmp, dir.join(&att)).is_ok() {
                changed = true;
            }
        }
        if changed {
            self.emit(ClientEvent::Changed { conv_ids: vec![], users: true, me: false });
        }
    }

    pub fn api(&self) -> Result<Arc<Api>> {
        self.api.read().unwrap_or_else(|e| e.into_inner()).clone().ok_or(ClientError::NotSignedIn)
    }

    async fn refresh_meta(&self) {
        if let Ok(api) = self.api() {
            if let Ok(m) = api.meta().await {
                if let Ok(j) = serde_json::to_string(&m) {
                    let _ = self.store.kv_set("server_meta", &j);
                }
                *self.server_meta.lock().unwrap_or_else(|e| e.into_inner()) = Some(m);
            }
        }
    }

    fn emit(&self, e: ClientEvent) {
        let _ = self.events.send(e);
    }

    fn set_state(&self, s: ConnState) {
        let changed = {
            let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let c = *g != s;
            *g = s.clone();
            c
        };
        if changed {
            self.emit(ClientEvent::Connection(s));
        }
    }

    fn send_ws(&self, m: ClientWsMsg) {
        if let Some(tx) = self.ws_cmd.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            let _ = tx.send(m);
        }
    }

    fn abort_tasks(&self) {
        for t in self.tasks.lock().unwrap_or_else(|e| e.into_inner()).drain(..) {
            t.abort();
        }
        *self.ws_cmd.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    fn install_api(self: &Arc<Self>, api: Arc<Api>) {
        let weak: Weak<Inner> = Arc::downgrade(self);
        api.set_fatal_handler(Arc::new(move |code, msg| {
            if let Some(inner) = weak.upgrade() {
                inner.wipe_local(Some(code), msg);
            }
        }));
        *self.api.write().unwrap_or_else(|e| e.into_inner()) = Some(api);
    }

    fn restore_session(self: &Arc<Self>) -> Result<()> {
        let raw = match self.secrets.get(SESSION_SECRET)? {
            Some(r) => r,
            None => match self.secrets.get(crate::api::LEGACY_SESSION_SECRET)? {
                Some(r) => {
                    let _ = self.secrets.set(SESSION_SECRET, &r);
                    let _ = self.secrets.delete(crate::api::LEGACY_SESSION_SECRET);
                    r
                }
                None => return Ok(()),
            },
        };
        let Ok(sess) = serde_json::from_str::<Session>(&raw) else { return Ok(()) };
        let api = Arc::new(Api::new(&sess.server, sess.pin.clone(), self.secrets.clone(), &self.cfg.client_version)?);
        api.set_session(Some(sess));
        self.install_api(api);
        Ok(())
    }

    fn finish_login(self: &Arc<Self>, api: Arc<Api>, base: &str, resp: LoginResponse, pin: Option<String>) -> Result<Me> {
        // A cache that belongs to another account must never be shown to this one.
        if let Some(prev) = self.store.me() {
            if prev.user.id != resp.me.user.id {
                self.store.wipe()?;
                let _ = std::fs::remove_dir_all(self.cfg.data_dir.join("files"));
            }
        }
        let sess = Session {
            server: base.to_string(),
            username: resp.me.user.username.clone(),
            user_id: resp.me.user.id.clone(),
            session_id: resp.session_id.clone(),
            access_token: resp.tokens.access_token.clone(),
            refresh_token: resp.tokens.refresh_token.clone(),
            expires_at_ms: now_ms() + resp.tokens.expires_in * 1000,
            pin,
        };
        api.set_session(Some(sess));
        api.persist()?;
        self.store.set_me(&resp.me)?;
        self.install_api(api);
        let mut p = crate::prefs::Prefs::load(&self.cfg.data_dir);
        p.last_server = base.to_string();
        p.last_username = resp.me.user.username.clone();
        p.save(&self.cfg.data_dir);
        Ok(resp.me)
    }

    /// X-05 / X-07: erase every trace of the account on this device and report why.
    pub fn wipe_local(&self, code: Option<ErrorCode>, message: String) {
        self.abort_tasks();
        let _ = self.store.wipe();
        let _ = std::fs::remove_dir_all(self.cfg.data_dir.join("files"));
        let _ = std::fs::remove_dir_all(self.cfg.data_dir.join("avatars"));
        let _ = self.secrets.delete(SESSION_SECRET);
        let _ = self.secrets.delete(crate::api::LEGACY_SESSION_SECRET);
        self.applock.clear();
        *self.api.write().unwrap_or_else(|e| e.into_inner()) = None;
        let msg = match code {
            Some(ErrorCode::AccountDisabled) => "Your account was disabled by an administrator. Local chats were removed from this device.".to_string(),
            Some(ErrorCode::PasswordReset) => "An administrator reset your password. You were signed out; sign in again with your new password or link.".to_string(),
            Some(ErrorCode::HistoryCleared) => "An administrator cleared your history. You were signed out; sign in again to load your chats.".to_string(),
            Some(ErrorCode::SessionRevoked) => "This device was signed out. Local chats were removed.".to_string(),
            _ => message,
        };
        self.set_state(ConnState::SignedOut);
        self.emit(ClientEvent::SignedOut { code, message: msg });
    }

    // ------------------------------------------------------------------ sync

    async fn sync_loop(self: Arc<Self>) {
        let mut backoff: u64 = 1;
        loop {
            if self.api().is_err() {
                return;
            }
            self.set_state(ConnState::Connecting);
            let connected = Arc::new(AtomicBool::new(false));
            let res = self.sync_once(connected.clone()).await;
            *self.ws_cmd.lock().unwrap_or_else(|e| e.into_inner()) = None;
            if connected.load(Ordering::Relaxed) {
                backoff = 1;
            }
            match res {
                Ok(()) => {}
                Err(e) if e.requires_wipe() => {
                    let (code, msg) = match &e {
                        ClientError::Api { code, message } => (Some(*code), message.clone()),
                        _ => (None, e.to_string()),
                    };
                    self.wipe_local(code, msg);
                    return;
                }
                Err(ClientError::Api { code: ErrorCode::CursorExpired, .. }) => {
                    let _ = self.store.kv_del("bootstrapped");
                    continue;
                }
                Err(ClientError::NotSignedIn) => return,
                Err(e) => {
                    tracing::debug!("sync error: {e}");
                    if matches!(e.code(), Some(ErrorCode::TokenExpired) | Some(ErrorCode::Unauthorized)) {
                        if let Ok(api) = self.api() {
                            if let Err(re) = api.refresh().await {
                                if re.requires_wipe() {
                                    continue; // the fatal handler already wiped
                                }
                            }
                        }
                        continue;
                    }
                }
            }
            let jitter = (rand_u64() % 700) as f64 / 1000.0;
            let wait = Duration::from_secs_f64(backoff as f64 + jitter);
            self.set_state(ConnState::Offline { retry_in_secs: backoff });
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = self.wake.notified() => { backoff = 1; }
            }
            backoff = (backoff * 2).min(30);
        }
    }

    async fn bootstrap(&self) -> Result<()> {
        let api = self.api()?;
        let b: Bootstrap = api.get("/sync/bootstrap").await?;
        self.store.apply_bootstrap(&b)?;
        if let Ok(v) = api.get::<Vec<ScheduledMessage>>("/scheduled").await {
            let _ = self.store.kv_set("scheduled", &serde_json::to_string(&v).unwrap_or_default());
        }
        if let Ok(f) = api.get::<ActivityFeed>("/activity?limit=150").await {
            let _ = self.store.put_activity(&f.items);
            let _ = self.store.set_activity_read_ms(f.read_ms);
        }
        self.emit(ClientEvent::Changed { conv_ids: b.conversations.iter().map(|c| c.conversation.id.clone()).collect(), users: true, me: true });
        Ok(())
    }

    async fn sync_once(self: &Arc<Self>, connected: Arc<AtomicBool>) -> Result<()> {
        let api = self.api()?;
        api.ensure_fresh().await?;
        if !self.store.has_snapshot() {
            self.bootstrap().await?;
        }
        let my_id = api.session().map(|s| s.user_id).unwrap_or_default();
        let url = api.ws_url();
        let connector = tokio_tungstenite::Connector::Rustls(crate::tls::client_config(api.pin.as_deref()));
        let (mut ws, _) = tokio::time::timeout(
            Duration::from_secs(20),
            tokio_tungstenite::connect_async_tls_with_config(url, None, false, Some(connector)),
        )
        .await
        .map_err(|_| ClientError::Network("connection timed out".into()))?
        .map_err(|e| ClientError::Network(e.to_string()))?;
        let token = api.current_access_token().unwrap_or_default();
        let hello = ClientWsMsg::Hello { token, cursor: self.store.cursor(), client_version: self.cfg.client_version.clone() };
        ws.send(WsFrame::Text(serde_json::to_string(&hello)?.into())).await.map_err(|e| ClientError::Network(e.to_string()))?;
        let (tx, mut rx) = mpsc::unbounded_channel::<ClientWsMsg>();
        *self.ws_cmd.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
        let mut ping = tokio::time::interval(Duration::from_secs(25));
        ping.tick().await;
        let mut last_rx = Instant::now();
        loop {
            tokio::select! {
                frame = ws.next() => {
                    let Some(frame) = frame else { return Ok(()) };
                    let frame = frame.map_err(|e| ClientError::Network(e.to_string()))?;
                    last_rx = Instant::now();
                    match frame {
                        WsFrame::Text(t) => {
                            let msg: ServerWsMsg = match serde_json::from_str(&t) { Ok(m) => m, Err(_) => continue };
                            self.handle_server_msg(msg, &my_id, &connected, &mut ws).await?;
                        }
                        WsFrame::Close(_) => return Ok(()),
                        WsFrame::Ping(p) => { let _ = ws.send(WsFrame::Pong(p)).await; }
                        _ => {}
                    }
                }
                cmd = rx.recv() => {
                    if let Some(c) = cmd {
                        ws.send(WsFrame::Text(serde_json::to_string(&c)?.into())).await.map_err(|e| ClientError::Network(e.to_string()))?;
                    }
                }
                _ = ping.tick() => {
                    if last_rx.elapsed() > Duration::from_secs(75) {
                        return Err(ClientError::Network("connection lost (no heartbeat)".into()));
                    }
                    ws.send(WsFrame::Text(serde_json::to_string(&ClientWsMsg::Ping)?.into())).await.map_err(|e| ClientError::Network(e.to_string()))?;
                }
                _ = self.wake.notified() => {}
            }
        }
    }

    async fn handle_server_msg<S>(self: &Arc<Self>, msg: ServerWsMsg, my_id: &str, connected: &Arc<AtomicBool>, ws: &mut S) -> Result<()>
    where
        S: futures_util::Sink<WsFrame, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
    {
        match msg {
            ServerWsMsg::HelloOk { .. } => {
                connected.store(true, Ordering::Relaxed);
                self.set_state(ConnState::Online);
                self.outbox_notify.notify_one();
                let me = self.clone();
                self.handle.spawn(async move {
                    me.refresh_meta().await;
                    if let Ok(api) = me.api() {
                        if let Ok(users) = api.get::<Vec<User>>("/users").await {
                            let _ = me.store.put_users(&users);
                            me.emit(ClientEvent::Changed { conv_ids: vec![], users: true, me: false });
                            me.sync_avatars().await;
                        }
                    }
                });
            }
            ServerWsMsg::Events { events, .. } => {
                let mut applied = Applied::default();
                let mut last = 0;
                for ev in &events {
                    applied.merge(self.store.apply_event(ev, my_id)?);
                    last = last.max(ev.cursor);
                }
                if applied.resync {
                    // The server restored data for this account: drop the cache and load it again.
                    let _ = self.store.kv_del("bootstrapped");
                    self.bootstrap().await?;
                    return Ok(());
                }
                for cid in applied.unknown_convs.clone() {
                    if self.store.conversation(&cid)?.is_none() && !applied.removed_convs.contains(&cid) {
                        self.fetch_conversation(&cid).await;
                        applied.conv_ids.push(cid);
                    }
                }
                if applied.activity_dirty {
                    let me = self.clone();
                    self.handle.spawn(async move {
                        if let Ok(api) = me.api() {
                            if let Ok(f) = api.get::<ActivityFeed>("/activity?limit=150").await {
                                let _ = me.store.put_activity(&f.items);
                                let _ = me.store.set_activity_read_ms(f.read_ms);
                                me.emit(ClientEvent::Changed { conv_ids: vec![], users: false, me: false });
                            }
                        }
                    });
                }
                if applied.users_changed || applied.me_changed || applied.avatars_changed {
                    let me = self.clone();
                    self.handle.spawn(async move { me.sync_avatars().await });
                }
                if !applied.conv_ids.is_empty() || applied.users_changed || applied.me_changed || applied.org_changed {
                    if applied.me_changed {
                        // keep a fresh copy of me for the UI
                    }
                    self.emit(ClientEvent::Changed { conv_ids: applied.conv_ids.clone(), users: applied.users_changed, me: applied.me_changed });
                }
                let dnd = self.store.me().map(|m| m.presence_pref == "dnd").unwrap_or(false);
                if !dnd {
                    for (m, c) in applied.incoming {
                        let sender_name = self.store.user(&m.sender_id).ok().flatten().map(|u| u.display_name).unwrap_or_default();
                        self.emit(ClientEvent::Incoming { message: m, conversation: c, sender_name });
                    }
                }
                if last > 0 {
                    let _ = ws.send(WsFrame::Text(serde_json::to_string(&ClientWsMsg::Ack { cursor: last })?.into())).await;
                }
            }
            ServerWsMsg::Typing { conv_id, user_id } => self.emit(ClientEvent::Typing { conv_id, user_id }),
            ServerWsMsg::Presence { user_id, presence, last_active_ms } => {
                if let Ok(Some(mut u)) = self.store.user(&user_id) {
                    u.presence = Some(presence);
                    u.last_active_ms = last_active_ms;
                    let _ = self.store.put_users(&[u]);
                }
                self.emit(ClientEvent::Presence { user_id, presence, last_active_ms });
            }
            ServerWsMsg::Pong => {}
            ServerWsMsg::Error { code, message } => return Err(ClientError::Api { code, message }),
        }
        Ok(())
    }

    async fn fetch_conversation(&self, id: &str) {
        let Ok(api) = self.api() else { return };
        if let Ok(cs) = api.get::<ConversationSummary>(&format!("/conversations/{id}")).await {
            let _ = self.store.put_conversation(&cs);
            if let Ok(h) = api.get::<HistoryResponse>(&format!("/conversations/{id}/messages?limit=40")).await {
                let _ = self.store.put_messages(&h.messages);
                let _ = self.store.set_has_more_before(id, h.has_more_before);
            }
        }
    }

    // ------------------------------------------------------------------ outbox

    async fn outbox_loop(self: Arc<Self>) {
        loop {
            tokio::select! {
                _ = self.outbox_notify.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(8)) => {}
            }
            if self.api().is_err() {
                return;
            }
            if let Err(e) = self.flush_outbox().await {
                tracing::debug!("outbox flush stopped: {e}");
            }
        }
    }

    /// Sends queued messages in order. Network errors keep the item pending (retried later with
    /// the same client id, so a retry never duplicates); other errors mark it failed.
    pub async fn flush_outbox(&self) -> Result<()> {
        let api = self.api()?;
        let items = self.store.outbox_pending()?;
        for mut item in items {
            if self.cancelled.lock().unwrap_or_else(|e| e.into_inner()).remove(&item.client_id) {
                self.store.outbox_remove(&item.client_id)?;
                continue;
            }
            let mut attachment_ids = vec![];
            let mut failed: Option<ClientError> = None;
            for i in 0..item.files.len() {
                if let Some(id) = &item.files[i].uploaded_id {
                    attachment_ids.push(id.clone());
                    continue;
                }
                let f = item.files[i].clone();
                let base: u64 = item.files.iter().filter(|x| x.uploaded_id.is_some()).map(|x| x.size).sum();
                let total_all: u64 = item.files.iter().map(|x| x.size).sum();
                let ctx = UploadCtx { client_id: item.client_id.clone(), inner: self.self_arc(), name: f.name.clone(), index: i, count: item.files.len(), base, total_all };
                let res = Client { inner: self.self_arc() }.upload_path(Path::new(&f.path), &f.name, None, Some(ctx)).await;
                match res {
                    Ok(up) => {
                        if let Some(m) = &f.meta {
                            // Waveform / poster are a bonus: a failure here never blocks the message.
                            let _: Result<serde_json::Value> = api.put_json(&format!("/files/{}/meta", up.attachment.id), m).await;
                        }
                        item.files[i].uploaded_id = Some(up.attachment.id.clone());
                        self.store.outbox_update_files(&item.client_id, &item.files)?;
                        attachment_ids.push(up.attachment.id);
                    }
                    Err(e) => {
                        failed = Some(e);
                        break;
                    }
                }
            }
            self.uploads.lock().unwrap_or_else(|e| e.into_inner()).remove(&item.client_id);
            if failed.is_some() && self.cancelled.lock().unwrap_or_else(|e| e.into_inner()).remove(&item.client_id) {
                // Cancelled while uploading: the message is already gone from the outbox.
                self.store.outbox_remove(&item.client_id)?;
                self.emit(ClientEvent::Changed { conv_ids: vec![item.conv_id.clone()], users: false, me: false });
                continue;
            }
            let result = match failed {
                Some(e) => Err(e),
                None => {
                    api.post::<Message, _>(
                        &format!("/conversations/{}/messages", item.conv_id),
                        &SendMessage {
                            client_id: item.client_id.clone(),
                            body: item.body.clone(),
                            thread_root: item.thread_root.clone(),
                            reply_to: item.reply_to.clone(),
                            mentions: item.mentions.clone(),
                            attachment_ids,
                        },
                    )
                    .await
                }
            };
            match result {
                Ok(m) => {
                    self.store.put_messages(std::slice::from_ref(&m))?;
                    self.emit(ClientEvent::Changed { conv_ids: vec![m.conv_id.clone()], users: false, me: false });
                }
                Err(e) if e.requires_wipe() => return Err(e),
                Err(e) if e.is_retryable() => {
                    self.store.outbox_attempt(&item.client_id, Some(&e.to_string()))?;
                    return Err(e);
                }
                Err(e) => {
                    let msg = e.to_string();
                    if self.cancelled.lock().unwrap_or_else(|e| e.into_inner()).remove(&item.client_id) {
                        self.store.outbox_remove(&item.client_id)?;
                    } else {
                        self.store.outbox_fail(&item.client_id, &msg)?;
                        self.emit(ClientEvent::SendFailed { client_id: item.client_id.clone(), error: msg });
                    }
                    self.emit(ClientEvent::Changed { conv_ids: vec![item.conv_id.clone()], users: false, me: false });
                }
            }
        }
        Ok(())
    }

    fn self_arc(&self) -> Arc<Inner> {
        // `Inner` is only ever created inside an Arc by `Client::open*`.
        self.me.upgrade().expect("client dropped")
    }
}

fn rand_u64() -> u64 {
    use rand::RngCore;
    rand::thread_rng().next_u64()
}
