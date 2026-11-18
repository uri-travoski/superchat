//! SuperChat UI shared by the desktop binary and the Android shell.
//!
//! The UI is declarative Slint (`ui/*.slint`); this file wires it to `client-core`, which owns all
//! state (cache, outbox, sync). The UI thread only renders snapshots from the client.
slint::include_modules!();

mod emoji_data;
mod forms;
mod model;
pub mod handoff;
pub mod nav;
pub mod lock;
pub mod platform;
mod sched;
mod shell;
mod shortcodes;

use client_core::logic::{datefmt, emoji, initials, markup, searchq};
use client_core::secrets::SecretStore;
use client_core::{Client, ClientConfig, ClientEvent, ConnState, Prefs, SendOptions};
use model::{color_of, model, row, Ctx, View};
use proto::types::{Attachment, ConvPrefRequest};
use slint::{ComponentHandle, Model, Weak};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Audio playback and recording (M-30 voice messages). Desktop shells out to players/recorders; Android uses
/// MediaPlayer and MediaRecorder through JNI.
pub trait Media: Send + Sync {
    fn play(&self, path: &Path) -> bool;
    fn stop(&self);
    fn playing(&self) -> bool;
    fn can_record(&self) -> bool;
    fn record_start(&self) -> bool;
    /// Stops recording and returns the finished file.
    fn record_stop(&self) -> Option<PathBuf>;
}

/// Desktop: ffplay/mpv/paplay for playback, `arecord` for recording when installed.
pub struct DesktopMedia {
    player: Mutex<Option<std::process::Child>>,
    recorder: Mutex<Option<(std::process::Child, PathBuf)>>,
    dir: PathBuf,
}

impl DesktopMedia {
    pub fn new(dir: PathBuf) -> DesktopMedia {
        DesktopMedia { player: Mutex::new(None), recorder: Mutex::new(None), dir }
    }
}

impl Media for DesktopMedia {
    fn play(&self, path: &Path) -> bool {
        self.stop();
        match platform::play_audio(path) {
            Some(ch) => {
                *self.player.lock().unwrap() = Some(ch);
                true
            }
            None => false,
        }
    }
    fn stop(&self) {
        if let Some(mut ch) = self.player.lock().unwrap().take() {
            let _ = ch.kill();
            let _ = ch.wait();
        }
    }
    fn playing(&self) -> bool {
        match self.player.lock().unwrap().as_mut() {
            Some(ch) => matches!(ch.try_wait(), Ok(None)),
            None => false,
        }
    }
    fn can_record(&self) -> bool {
        cfg!(unix) && std::process::Command::new("arecord").arg("--version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
    }
    fn record_start(&self) -> bool {
        let path = self.dir.join(format!("voice-message-{}.wav", now_ms() / 1000));
        let _ = std::fs::create_dir_all(&self.dir);
        match std::process::Command::new("arecord").args(["-q", "-f", "S16_LE", "-r", "16000", "-c", "1", "-t", "wav"]).arg(&path).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn() {
            Ok(ch) => {
                *self.recorder.lock().unwrap() = Some((ch, path));
                true
            }
            Err(_) => false,
        }
    }
    fn record_stop(&self) -> Option<PathBuf> {
        let (mut ch, path) = self.recorder.lock().unwrap().take()?;
        #[cfg(unix)]
        {
            // SIGINT lets arecord finish the WAV header.
            let _ = std::process::Command::new("kill").args(["-INT", &ch.id().to_string()]).status();
        }
        #[cfg(not(unix))]
        let _ = ch.kill();
        let _ = ch.wait();
        std::fs::metadata(&path).ok().filter(|m| m.len() > 44).map(|_| path)
    }
}

/// Text and files handed to the app by the OS share sheet (E-06).
pub type Shared = (String, Vec<PathBuf>);

/// Platform hooks (Android supplies its own data dir and system font scale).
pub struct Hooks {
    pub data_dir: PathBuf,
    pub allow_insecure_http: bool,
    pub device_name: String,
    pub system_scale: Box<dyn Fn() -> f32>,
    /// Returns a conversation id once when the OS asks to open one (notification tap).
    pub take_open_conv: Box<dyn Fn() -> Option<String>>,
    /// Token storage; None lets the client use its private-file default (Android).
    pub secrets: Option<Arc<dyn SecretStore>>,
    pub is_android: bool,
    pub media: Arc<dyn Media>,
    /// Returns what another app shared with us, once.
    pub take_shared: Box<dyn Fn() -> Option<Shared>>,
    /// Android: opens the camera scanner for a login-link QR code. None on desktop (paste the link instead).
    pub scan_qr: Option<Box<dyn Fn()>>,
    /// Android: "Choose image with QR": the photo picker, then decoding.
    pub scan_image: Option<Box<dyn Fn()>>,
    /// A message from the platform's scanner about why a scan failed (no camera, permission refused...), once.
    pub take_login_error: Box<dyn Fn() -> Option<String>>,
    /// A one-time login link that arrived from outside (QR scan result, a tapped superchat:// link), once.
    pub take_login_link: Box<dyn Fn() -> Option<String>>,
    /// Android: the clipboard text (desktop pastes through the text field itself).
    pub clipboard: Option<Box<dyn Fn() -> Option<String>>>,
}

impl Hooks {
    pub fn desktop() -> Hooks {
        let data_dir = default_data_dir();
        Hooks {
            secrets: Some(platform::desktop_secrets(&data_dir)),
            data_dir,
            allow_insecure_http: proto::env_var("ATOMIC_ALLOW_HTTP").is_ok(),
            device_name: hostname(),
            system_scale: Box::new(|| 1.0),
            take_open_conv: Box::new(|| None),
            is_android: false,
            media: Arc::new(DesktopMedia::new(default_data_dir().join("recordings"))),
            take_shared: Box::new(|| None),
            scan_qr: None,
            scan_image: None,
            take_login_error: Box::new(|| None),
            take_login_link: Box::new(|| None),
            clipboard: None,
        }
    }
}

fn hostname() -> String {
    std::env::var("HOSTNAME").or_else(|_| std::fs::read_to_string("/etc/hostname")).map(|s| s.trim().to_string()).unwrap_or_else(|_| "Desktop".into())
}

pub fn default_data_dir() -> PathBuf {
    if let Ok(d) = proto::env_var("ATOMIC_DATA_DIR") {
        return PathBuf::from(d);
    }
    let (new, old) = if let Ok(d) = std::env::var("XDG_DATA_HOME") {
        (PathBuf::from(&d).join("superchat"), PathBuf::from(d).join("atomic-chat"))
    } else if let Ok(d) = std::env::var("APPDATA") {
        (PathBuf::from(&d).join("SuperChat"), PathBuf::from(d).join("AtomicChat"))
    } else {
        let h = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".local/share");
        (h.join("superchat"), h.join("atomic-chat"))
    };
    migrate_legacy_dir(&old, &new);
    new
}

/// One-time move of the data folder used by Atomic Chat 1.0 to the SuperChat folder.
pub fn migrate_legacy_dir(old: &std::path::Path, new: &std::path::Path) {
    if old.is_dir() && !new.exists() {
        if let Some(parent) = new.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::rename(old, new);
    }
}

static CLIENT: OnceLock<Client> = OnceLock::new();
static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Background runtime that runs the client's network tasks (the UI thread stays free).
pub fn runtime() -> &'static tokio::runtime::Runtime {
    RT.get_or_init(|| tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("tokio runtime"))
}

/// The process-wide client (one per data dir). Also used by the Android foreground service.
pub fn shared_client(data_dir: &Path, insecure: bool, device: &str) -> Client {
    shared_client_with(data_dir, insecure, device, None)
}

pub fn shared_client_with(data_dir: &Path, insecure: bool, device: &str, secrets: Option<Arc<dyn SecretStore>>) -> Client {
    CLIENT
        .get_or_init(|| {
            let mut cfg = ClientConfig::new(data_dir.join("client"));
            cfg.allow_insecure_http = insecure;
            cfg.device_name = device.to_string();
            cfg.secrets = secrets;
            Client::open_with_handle(cfg, runtime().handle().clone()).expect("open client")
        })
        .clone()
}

/// Android cannot tell chrono its zone, so the Kotlin shell pushes the offset (seconds) here; `i32::MIN` = unset.
static TZ_OVERRIDE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(i32::MIN);

pub use lock::{note_background, note_input};

pub fn set_tz_offset(secs: i32) {
    TZ_OVERRIDE.store(secs, std::sync::atomic::Ordering::Relaxed);
}

fn tz_offset() -> i32 {
    use chrono::Offset;
    match TZ_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed) {
        i32::MIN => chrono::Local::now().offset().fix().local_minus_utc(),
        v => v,
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

#[derive(Default)]
struct St {
    prefs: Prefs,
    data_dir: PathBuf,
    system_scale: f32,
    current: String,
    thread: Option<(String, String)>,
    typing: Option<(String, String, Instant)>,
    hist: Vec<String>,
    hist_pos: usize,
    limit: usize,
    first_unread: Option<usize>,
    msg_count: usize,
    /// last_read_seq when the conversation was opened (keeps the "New messages" divider in place).
    read_mark: i64,
    loading_older: bool,
    atts: HashMap<String, Attachment>,
    img_inflight: HashSet<String>,
    pending_login: Option<(String, String, String, String, String)>,
    search_gen: u64,
    scroll_after: Option<f32>,
    keep_scroll: Option<(f32, f32)>,
    form: Option<forms::FormState>,
    recording: bool,
    shared: Option<Shared>,
    home_bot: String,
    home_forms: Vec<proto::automation::Form>,
    pp_sel: HashSet<String>,
}

type Sh = Arc<Mutex<St>>;

/// N-01: registers this device for wake-up pushes with the server and tells the Android shell which ntfy topic to
/// listen to (it reads `ntfy.json`; Google push writes `fcm_token.txt`). Content-free: the push only wakes the app.
fn sync_push(c: &Client, sh: &Sh) {
    use proto::types::PushRegistration;
    let (choice, url, topic, old_id, dir) = {
        let g = sh.lock().unwrap();
        (g.prefs.push_choice.clone(), g.prefs.ntfy_url.trim().to_string(), g.prefs.ntfy_topic.clone(), g.prefs.push_reg_id.clone(), g.data_dir.clone())
    };
    if !c.signed_in() {
        return;
    }
    let (c2, sh2) = (c.clone(), sh.clone());
    c.handle().spawn(async move {
        let ntfy_file = dir.join("ntfy.json");
        let reg = match choice.as_str() {
            "ntfy" => {
                let effective = if url.is_empty() { c2.server_meta().and_then(|m| m.ntfy_url).unwrap_or_default() } else { url.clone() };
                if effective.is_empty() {
                    let _ = std::fs::remove_file(&ntfy_file);
                    return;
                }
                let topic = if topic.is_empty() { random_topic() } else { topic };
                let _ = std::fs::write(&ntfy_file, serde_json::json!({"url": effective.trim_end_matches('/'), "topic": topic}).to_string());
                Some(PushRegistration { id: String::new(), kind: "ntfy".into(), target: topic, ntfy_url: if url.is_empty() { None } else { Some(url) }, device_name: "Android".into(), created_ms: 0 })
            }
            "fcm" => {
                let _ = std::fs::remove_file(&ntfy_file);
                std::fs::read_to_string(dir.join("fcm_token.txt")).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).map(|t| PushRegistration { id: String::new(), kind: "fcm".into(), target: t, ntfy_url: None, device_name: "Android".into(), created_ms: 0 })
            }
            _ => {
                let _ = std::fs::remove_file(&ntfy_file);
                None
            }
        };
        if !old_id.is_empty() {
            let _ = c2.unregister_push(&old_id).await;
        }
        let (new_id, new_topic) = match reg {
            Some(r) => {
                let topic = if r.kind == "ntfy" { r.target.clone() } else { String::new() };
                match c2.register_push(&r).await {
                    Ok(saved) => (saved.id, topic),
                    Err(_) => (String::new(), topic),
                }
            }
            None => (String::new(), String::new()),
        };
        let mut g = sh2.lock().unwrap();
        g.prefs.push_reg_id = new_id;
        g.prefs.ntfy_topic = new_topic;
        save_prefs(&g);
    });
}

fn random_topic() -> String {
    use std::io::Read;
    let mut b = [0u8; 12];
    if std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).is_err() {
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        b.copy_from_slice(&n.to_le_bytes()[..12]);
    }
    format!("atomic-{}", b.iter().map(|x| format!("{x:02x}")).collect::<String>())
}

fn view(st: &St) -> View {
    View { hour24: st.prefs.hour24, accent: st.prefs.accent as usize, off: tz_offset(), now: now_ms() }
}

fn scale_of(st: &St) -> f32 {
    (st.prefs.font_scale as f32 / 100.0 * st.prefs.zoom as f32 / 100.0 * st.system_scale).clamp(0.6, 2.4)
}

fn apply_prefs(ui: &AppWindow, st: &St) {
    let t = ui.global::<Theme>();
    t.set_scale(scale_of(st));
    t.set_compact(st.prefs.compact);
    t.set_reduce_motion(st.prefs.reduce_motion);
    t.set_accent_index(st.prefs.accent.min(3) as i32);
    ui.set_theme_mode(match st.prefs.theme.as_str() {
        "light" => 1,
        "dark" => 2,
        _ => 0,
    });
    ui.set_font_step(match st.prefs.font_scale {
        0..=107 => 0,
        108..=122 => 1,
        _ => 2,
    });
    ui.set_zoom(st.prefs.zoom as i32);
    ui.set_hour24(st.prefs.hour24);
    ui.set_notif(st.prefs.notifications);
    ui.set_notif_preview(st.prefs.notification_preview);
    ui.set_push_choice(st.prefs.push_choice.clone().into());
    ui.set_ntfy_url(st.prefs.ntfy_url.clone().into());
    ui.set_launch_login(st.prefs.launch_on_login);
    ui.set_start_min(st.prefs.start_minimised);
    ui.set_sort_alpha(st.prefs.sort == "alpha");
    ui.set_unread_only(st.prefs.unread_only);
    ui.set_hide_muted(st.prefs.hide_muted);
}

fn save_prefs(st: &St) {
    st.prefs.save(&st.data_dir);
}

// ------------------------------------------------------------------------------------------------
// Rendering

/// People you can message, optionally filtered by a name or username fragment.
fn people_list(c: &Client, filter: &str) -> Vec<PersonItem> {
    let me = c.my_id().unwrap_or_default();
    let q = filter.trim().to_lowercase();
    let mut v: Vec<PersonItem> = c
        .users()
        .into_iter()
        .filter(|u| u.id != me && !u.disabled && (q.is_empty() || u.display_name.to_lowercase().contains(&q) || u.username.to_lowercase().contains(&q)))
        .map(|u| model::person(&u))
        .collect();
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    v
}

fn refresh(ui: &AppWindow, c: &Client, sh: &Sh) {
    let mut g = sh.lock().unwrap();
    let v = view(&g);
    let me_id = c.my_id().unwrap_or_default();
    ui.set_conversations(model(model::conv_items(c, g.prefs.sort == "alpha", g.prefs.unread_only, g.prefs.hide_muted, &v)));
    ui.set_total_unread(c.unread_total() as i32);
    shell::refresh_shell(ui, c, &g);
    ui.set_scheduled_count(c.scheduled_pending(None).len() as i32);
    ui.set_ai_enabled(c.ai_enabled());
    if let Some(me) = c.me() {
        ui.set_me_name(me.user.display_name.clone().into());
        ui.set_me_initials(initials::initials(&me.user.display_name).into());
        ui.set_me_color(color_of(&me.user.id));
        ui.set_presence_pref(me.presence_pref.clone().into());
        ui.set_privacy_enabled(me.privacy_controls_enabled);
        let allowed = me.user.role.is_admin() || me.privacy_controls_enabled;
        ui.set_sign_out_allowed(allowed);
        if !allowed {
            ui.set_confirm_sign_out_all(false);
        }
        ui.set_hide_last_active(me.hide_last_active);
        ui.set_me_username(me.user.username.clone().into());
        ui.set_is_admin(me.user.role.is_admin());
        if let Some(m) = c.server_meta() {
            ui.set_server_text(format!("{} (server {})", m.name, m.server_version).into());
        }
    }
    ui.set_can_back(g.hist_pos > 0);
    ui.set_can_forward(g.hist_pos + 1 < g.hist.len());
    let cur = g.current.clone();
    if cur.is_empty() {
        return;
    }
    let Some(cs) = c.conversations().into_iter().find(|x| x.conversation.id == cur) else { return };
    ui.set_current_title(c.conversation_title(&cs).into());
    ui.set_current_initials(initials::initials(&c.conversation_title(&cs)).into());
    ui.set_current_color(color_of(&cs.conversation.id));
    ui.set_members(model(
        cs.conversation
            .members
            .iter()
            .map(|m| match c.user(&m.user_id) {
                Some(u) => {
                    let mut p = model::person(&u);
                    p.sub = if m.role == "owner" { format!("@{}  |  owner", u.username).into() } else { p.sub };
                    if m.user_id == me_id {
                        p.tag = "me".into();
                        p.name = format!("{} (you)", u.display_name).into();
                    }
                    p
                }
                None => PersonItem { id: m.user_id.clone().into(), name: "Unknown".into(), initials: "?".into(), color: color_of(&m.user_id), sub: Default::default(), sel: false, tag: Default::default() },
            })
            .collect(),
    ));
    let members = cs.conversation.members.len();
    ui.set_current_sub(
        match cs.conversation.kind {
            proto::types::ConversationKind::Channel | proto::types::ConversationKind::GroupDm => {
                let topic = cs.conversation.topic.clone().unwrap_or_default();
                if topic.is_empty() { format!("{members} members") } else { format!("{members} members  |  {topic}") }
            }
            proto::types::ConversationKind::SelfNote => "Notes to self".into(),
            proto::types::ConversationKind::Dm => cs
                .conversation
                .members
                .iter()
                .find(|m| m.user_id != me_id)
                .and_then(|m| c.user(&m.user_id))
                .map(|u| match u.presence {
                    Some(proto::types::Presence::Online) => "Online".to_string(),
                    Some(proto::types::Presence::Dnd) => "Do not disturb".to_string(),
                    _ => u.last_active_ms.map(|t| format!("Last active {}", datefmt::last_active(t, v.now, v.off, v.hour24))).unwrap_or_default(),
                })
                .unwrap_or_default(),
        }
        .into(),
    );
    ui.set_current_fav(c.pref(&cur).favourite);
    shell::refresh_details(ui, c, &cs, &me_id, &v);
    ui.set_can_write(!cs.conversation.archived);
    ui.set_bookmarks(model(model::bookmarks(c, &cur)));
    let is_admin = c.me().map(|m| m.user.role.is_admin()).unwrap_or(false);
    ui.set_can_curate(is_admin || cs.conversation.members.iter().any(|m| m.user_id == me_id && m.role == "owner"));
    let bot = bot_of_dm(c, &cs, &me_id);
    ui.set_is_bot_dm(bot.is_some());
    if bot.is_none() && ui.get_tab() == 3 {
        ui.set_tab(0);
    }
    let pinned: HashSet<String> = c.pins(&cur).into_iter().map(|p| p.message_id).collect();
    let mut want = vec![];
    let mut atts = std::mem::take(&mut g.atts);
    let limit = g.limit.max(40);
    if g.prefs.sort.is_empty() {
        g.prefs.sort = "activity".into();
    }
    let (items, first_unread) = {
        let mut cx = Ctx { c, me: me_id.clone(), v: &v, atts: &mut atts, want_images: &mut want, pinned: pinned.clone(), replies: c.reply_counts(&cur), can_pin: is_admin || cs.conversation.members.iter().any(|m| m.user_id == me_id && m.role == "owner"), saved: c.saved_items().into_iter().map(|s| s.item.message_id).collect() };
        model::msg_items(&mut cx, &cur, limit, g.read_mark)
    };
    g.atts = atts;
    g.first_unread = first_unread;
    ui.set_first_unread(first_unread.is_some());
    ui.set_has_more_older(c.has_more_before(&cur) || c.timeline(&cur, limit + 1, None).len() > limit);
    let grew = items.len() != g.msg_count;
    g.msg_count = items.len();
    let at_bottom = ui.get_list_y() <= ui.get_list_vh() - ui.get_list_h() + 40.0 || ui.get_messages().row_count() == 0;
    ui.set_messages(model(items));
    if let Some((old_y, old_h)) = g.keep_scroll.take() {
        let wk = ui.as_weak();
        slint::Timer::single_shot(Duration::from_millis(80), move || {
            if let Some(ui) = wk.upgrade() {
                let added = ui.get_list_h() - old_h;
                ui.set_list_y((old_y - added).min(0.0));
            }
        });
    } else if let Some(f) = g.scroll_after.take() {
        let wk = ui.as_weak();
        slint::Timer::single_shot(Duration::from_millis(100), move || {
            if let Some(ui) = wk.upgrade() {
                ui.invoke_scroll_to_fraction(f);
            }
        });
    } else if grew && at_bottom {
        let wk = ui.as_weak();
        slint::Timer::single_shot(Duration::from_millis(60), move || {
            if let Some(ui) = wk.upgrade() {
                ui.invoke_scroll_bottom();
            }
        });
    }
    // Tabs.
    ui.set_pins(model(
        c.pins(&cur)
            .into_iter()
            .filter_map(|p| {
                let m = c.store().message(&p.message_id).ok().flatten()?;
                Some(row(0, &p.message_id, &cur, format!("{} - {}", c.display_name(&m.sender_id), datefmt::format_date(m.ts, v.off)), model::plain(&m.body), String::new()))
            })
            .collect(),
    ));
    ui.set_files(model(
        c.store()
            .cached_files(Some(&cur), 200)
            .into_iter()
            .map(|(m, a)| {
                g.atts.insert(a.id.clone(), a.clone());
                row(1, &a.id, &cur, a.name.clone(), String::new(), format!("{}  |  {}  |  {}", model::human_size(a.size), c.display_name(&m.sender_id), datefmt::format_date(m.ts, v.off)))
            })
            .collect(),
    ));
    // Thread panel.
    if let Some((tc, root)) = g.thread.clone() {
        if let Some(rm) = c.store().message(&root).ok().flatten() {
            let mut atts = std::mem::take(&mut g.atts);
            let mut cx = Ctx { c, me: me_id.clone(), v: &v, atts: &mut atts, want_images: &mut want, pinned: HashSet::new(), replies: c.reply_counts(&tc), can_pin: false, saved: c.saved_items().into_iter().map(|s| s.item.message_id).collect() };
            ui.set_thread_conv_title(conv_title_of_id(c, &tc).into());
            let rt = client_core::TimelineItem { message: rm, state: client_core::SendState::Sent, local: false, error: None };
            ui.set_thread_root(model::msg_item(&mut cx, &rt, String::new(), false));
            let replies: Vec<_> = c.thread_replies(&tc, &root).iter().map(|t| model::msg_item(&mut cx, t, String::new(), false)).collect();
            ui.set_thread_messages(model(replies));
            g.atts = atts;
        }
    }
    // Inline image previews download in the background, then refresh once.
    for a in want {
        if g.img_inflight.insert(a.id.clone()) {
            let (wk, c2, sh2) = (ui.as_weak(), c.clone(), sh.clone());
            c.handle().spawn(async move {
                let ok = c2.download_attachment(&a).await.is_ok();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = wk.upgrade() {
                        if ok {
                            refresh(&ui, &c2, &sh2);
                        }
                    }
                });
            });
        }
    }
}

/// `@channel`, `@here`, `@everyone` (channels and groups) and user groups for the mention autocomplete (M-17, M-18).
fn special_mentions(c: &Client, kind: &proto::types::ConversationKind, q: &str) -> Vec<PersonItem> {
    let mk = |handle: &str, name: &str, sub: &str| PersonItem { id: handle.into(), name: name.into(), initials: "@".into(), color: color_of(handle), sub: sub.into(), sel: false, tag: Default::default() };
    let mut v = vec![];
    if !matches!(kind, proto::types::ConversationKind::Dm | proto::types::ConversationKind::SelfNote) {
        for (h, d) in [("channel", "Notify everyone in this conversation"), ("here", "Notify people who are online"), ("everyone", "Notify everyone in this conversation")] {
            if q.is_empty() || h.starts_with(q) {
                v.push(mk(h, &format!("@{h}"), d));
            }
        }
    }
    for g in c.groups() {
        if q.is_empty() || g.handle.to_lowercase().starts_with(q) {
            v.push(mk(&g.handle, &format!("@{}", g.handle), &format!("Group: {} ({} people)", g.name, g.member_ids.len())));
        }
    }
    v
}

fn conv_title_of_id(c: &Client, id: &str) -> String {
    c.conversations().into_iter().find(|x| x.conversation.id == id).map(|cs| c.conversation_title(&cs)).unwrap_or_default()
}

/// The bot on the other side of a direct message, if it is one.
fn bot_of_dm(c: &Client, cs: &proto::types::ConversationSummary, me: &str) -> Option<String> {
    if cs.conversation.kind != proto::types::ConversationKind::Dm {
        return None;
    }
    cs.conversation.members.iter().find(|m| m.user_id != me).filter(|m| c.user(&m.user_id).map(|u| u.is_bot).unwrap_or(false)).map(|m| m.user_id.clone())
}

fn home_rows(p: &proto::automation::BotProfile) -> (Vec<RowItem>, Vec<proto::automation::Form>) {
    use proto::automation::HomeBlock;
    let mut forms = vec![];
    let mut rows = vec![];
    if !p.description.is_empty() {
        rows.push(row(1, "", "", String::new(), p.description.clone(), String::new()));
    }
    for b in &p.home.blocks {
        match b {
            HomeBlock::Header { text } => rows.push(row(0, "", "", String::new(), text.clone(), String::new())),
            HomeBlock::Text { text } => rows.push(row(1, "", "", String::new(), text.clone(), String::new())),
            HomeBlock::Divider {} => rows.push(row(2, "", "", String::new(), String::new(), String::new())),
            HomeBlock::Button { id, label, style } => rows.push(row(3, id, "", String::new(), label.clone(), style.clone().unwrap_or_default())),
            HomeBlock::Form { form } => {
                rows.push(row(4, &form.id, "", form.title.clone().unwrap_or_else(|| "Form".into()), form.description.clone().unwrap_or_default(), String::new()));
                forms.push(form.clone());
            }
        }
    }
    (rows, forms)
}

fn load_home(ui: &AppWindow, c: &Client, sh: &Sh) {
    let cur = sh.lock().unwrap().current.clone();
    let Some(cs) = c.conversations().into_iter().find(|x| x.conversation.id == cur) else { return };
    let Some(bot) = bot_of_dm(c, &cs, &c.my_id().unwrap_or_default()) else { return };
    let (wk, c2, sh2) = (ui.as_weak(), c.clone(), sh.clone());
    c.handle().spawn(async move {
        let Ok(p) = c2.bot_profile(&bot).await else { return };
        let (rows, forms) = home_rows(&p);
        {
            let mut g = sh2.lock().unwrap();
            g.home_bot = bot;
            g.home_forms = forms;
        }
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = wk.upgrade() {
                ui.set_home(model(rows));
            }
        });
    });
}

fn show_form(ui: &AppWindow, st: forms::FormState, fallback_title: &str) {
    let target = &st.form;
    ui.set_form_title(target.title.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| fallback_title.to_string()).into());
    ui.set_form_desc(target.description.clone().unwrap_or_default().into());
    ui.set_form_submit_label(target.submit_label.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| "Submit".into()).into());
    ui.set_form_error("".into());
    ui.set_form_fields(model(st.items()));
    ui.set_form_open(true);
}

fn draft_rows(c: &Client, off: i32, hour24: bool) -> Vec<RowItem> {
    use proto::automation::ScheduledState;
    let title_of = |conv: &str| c.conversations().into_iter().find(|x| x.conversation.id == conv).map(|cs| c.conversation_title(&cs)).unwrap_or_else(|| "Conversation".into());
    let stamp = |ms: i64| format!("{} {}", datefmt::format_date(ms, off), datefmt::format_time(ms, off, hour24));
    let mut out: Vec<RowItem> = vec![];
    let sched = c.scheduled();
    let mut first = true;
    for m in sched.iter().filter(|m| m.state == ScheduledState::Pending) {
        let mut r = row(0, &m.id, &m.conv_id, format!("To {}", title_of(&m.conv_id)), model::plain(&m.body), format!("Sends {}", stamp(m.due_ms)));
        if first {
            r.header = "Scheduled".into();
            first = false;
        }
        out.push(r);
    }
    let mut first = true;
    for d in c.all_drafts().into_iter().filter(|d| !d.body.trim().is_empty() && d.thread_root.is_none()) {
        let mut r = row(1, "", &d.conv_id, format!("Draft in {}", title_of(&d.conv_id)), model::plain(&d.body), format!("Edited {}", stamp(d.updated_ms)));
        if first {
            r.header = "Drafts".into();
            first = false;
        }
        out.push(r);
    }
    let mut first = true;
    for m in sched.iter().filter(|m| m.state != ScheduledState::Pending).take(20) {
        let (status, bad) = match m.state {
            ScheduledState::Sent => (format!("Sent {}", stamp(m.due_ms)), false),
            ScheduledState::Cancelled => ("Cancelled".to_string(), false),
            _ => (format!("Not sent: {}", m.error.clone().unwrap_or_default()), true),
        };
        let mut r = row(2, &m.id, &m.conv_id, format!("To {}", title_of(&m.conv_id)), model::plain(&m.body), status);
        r.flag = bad;
        if first {
            r.header = "Sent".into();
            first = false;
        }
        out.push(r);
    }
    out
}

fn select_conv(ui: &AppWindow, c: &Client, sh: &Sh, id: &str, push_history: bool) {
    {
        let mut g = sh.lock().unwrap();
        // Keep the unsent text as a draft of the conversation we are leaving.
        if !g.current.is_empty() && g.current != id {
            c.save_draft(&g.current, None, ui.get_composer().as_str());
        }
        if g.current != id {
            ui.set_composer(c.draft(id).into());
            ui.set_editing_id("".into());
        }
        g.read_mark = c.conversations().into_iter().find(|x| x.conversation.id == id).filter(|x| x.unread > 0).map(|x| x.last_read_seq).unwrap_or(i64::MAX);
        g.current = id.to_string();
        g.thread = None;
        g.limit = 60;
        g.msg_count = 0;
        g.scroll_after = None;
        g.loading_older = false;
        if push_history {
            let pos = g.hist_pos;
            if g.hist.get(pos).map(|s| s.as_str()) != Some(id) {
                let keep = if g.hist.is_empty() { 0 } else { pos + 1 };
                g.hist.truncate(keep);
                g.hist.push(id.to_string());
                g.hist_pos = g.hist.len() - 1;
            }
        }
    }
    ui.set_current_conv(id.into());
    ui.set_thread_id("".into());
    ui.set_info_open(false);
    ui.set_info_page("".into());
    ui.set_editing_id("".into());
    ui.set_emoji_sugg(model(vec![]));
    ui.set_quote_id("".into());
    ui.set_quote_text("".into());
    ui.set_tab(0);
    ui.set_screen("".into());
    ui.set_show_chat(true);
    ui.set_mentions(model(vec![]));
    ui.set_selected_msg("".into());
    ui.set_messages(model(vec![]));
    refresh(ui, c, sh);
    ui.invoke_scroll_bottom();
    let (c2, id2) = (c.clone(), id.to_string());
    c.handle().spawn(async move {
        let _ = c2.mark_read(&id2).await;
    });
}

/// Loads the neighbourhood of `msg_id` and scrolls to it.
fn show_message(ui: &AppWindow, c: &Client, sh: &Sh, conv: &str, msg_id: &str) {
    select_conv(ui, c, sh, conv, true);
    let (wk, c2, sh2, conv, msg_id) = (ui.as_weak(), c.clone(), sh.clone(), conv.to_string(), msg_id.to_string());
    c.handle().spawn(async move {
        let _ = c2.load_around(&conv, &msg_id).await;
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = wk.upgrade() else { return };
            let all: Vec<_> = c2.timeline(&conv, 5000, None).into_iter().filter(|t| t.message.thread_root.is_none()).collect();
            if let Some(i) = all.iter().position(|t| t.message.id == msg_id) {
                let mut g = sh2.lock().unwrap();
                g.limit = all.len() - i + 20;
                g.scroll_after = Some((i as f32 / all.len().max(1) as f32).clamp(0.0, 1.0));
            }
            refresh(&ui, &c2, &sh2);
        });
    });
}

fn open_thread(ui: &AppWindow, c: &Client, sh: &Sh, conv: &str, root: &str) {
    sh.lock().unwrap().thread = Some((conv.to_string(), root.to_string()));
    ui.set_thread_id(root.into());
    ui.set_thread_composer("".into());
    refresh(ui, c, sh);
    let (wk, c2, sh2, conv, root) = (ui.as_weak(), c.clone(), sh.clone(), conv.to_string(), root.to_string());
    c.handle().spawn(async move {
        let _ = c2.open_thread(&conv, &root).await;
        let _ = c2.mark_thread_read(&root).await;
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = wk.upgrade() {
                refresh(&ui, &c2, &sh2);
            }
        });
    });
}

/// Returns to the login screen after a self-service sign-out; a refused one (server says forbidden) changes nothing.
fn finish_sign_out(wk: Weak<AppWindow>, ok: bool) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = wk.upgrade() {
            if !ok {
                toast(&ui, "Sign-out is not allowed for this account.");
                return;
            }
            ui.set_signed_in(false);
            ui.set_login_error("".into());
            ui.set_conversations(model(vec![]));
            ui.set_messages(model(vec![]));
            ui.set_current_conv("".into());
            ui.set_current_title("".into());
        }
    });
}

fn load_sessions(ui: &AppWindow, c: &Client) {
    let (wk, c2) = (ui.as_weak(), c.clone());
    c.handle().spawn(async move {
        let off = tz_offset();
        let rows: Vec<RowItem> = c2
            .sessions()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|s| !s.revoked)
            .map(|s| {
                let mut r = row(0, &s.id, "", format!("{} ({})", s.device_name, s.client_version), String::new(), format!("{}  |  last used {}", s.ip, datefmt::format_date(s.last_seen_ms, off)));
                r.flag = s.current;
                r
            })
            .collect();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = wk.upgrade() {
                ui.set_sessions(model(rows));
            }
        });
    });
}

fn do_login(ui: &AppWindow, c: &Client, sh: &Sh, server: String, user: String, pass: String, link: String, pin: Option<String>) {
    let (wk, c, sh) = (ui.as_weak(), c.clone(), sh.clone());
    c.clone().handle().spawn(async move {
        let r = if link.trim().is_empty() {
            c.login(server.trim(), user.trim(), &pass, pin.as_deref()).await
        } else {
            c.login_with_link(link.trim(), if server.trim().is_empty() { None } else { Some(server.trim()) }, pin.as_deref()).await
        };
        if r.is_ok() {
            c.start();
        }
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = wk.upgrade() {
                ui.set_busy(false);
                match r {
                    Ok(_) => {
                        {
                            let mut g = sh.lock().unwrap();
                            g.prefs.last_server = server.clone();
                            g.prefs.last_username = user.clone();
                            save_prefs(&g);
                        }
                        ui.set_f_pass("".into());
                        ui.set_signed_in(true);
                        refresh(&ui, &c, &sh);
                        sync_push(&c, &sh);
                    }
                    Err(e) => ui.set_login_error(login_error_text(&e).into()),
                }
            }
        });
    });
}

/// The exact reason a sign-in failed, in words a person can act on.
fn login_error_text(e: &client_core::ClientError) -> String {
    match e {
        client_core::ClientError::Network(m) => format!("Cannot reach the server: {m}. Check the server address and your connection."),
        other => other.to_string(),
    }
}

/// If `text` contains a SuperChat login link (superchat:// or the older atomic-chat://), switch the login screen to
/// link mode and take the server address from the link.
fn adopt_login_text(ui: &AppWindow, text: &str) {
    if let Some(l) = proto::rules::extract_login_link(text) {
        ui.set_use_link(true);
        ui.set_f_link(l.clone().into());
        if let Some(srv) = proto::rules::parse_login_link(&l).and_then(|p| p.server) {
            ui.set_f_server(srv.into());
        }
        ui.set_login_error("".into());
    }
}

fn toast(ui: &AppWindow, text: &str) {
    ui.set_toast(text.into());
    let wk = ui.as_weak();
    slint::Timer::single_shot(Duration::from_millis(2500), move || {
        if let Some(ui) = wk.upgrade() {
            ui.set_toast("".into());
        }
    });
}

pub fn run() -> Result<(), slint::PlatformError> {
    let mut hooks = Hooks::desktop();
    // A login link given on the command line (the browser's "Open in SuperChat" button, a superchat:// click).
    let link = std::env::args().skip(1).find_map(|a| proto::rules::extract_login_link(&a));
    if let Some(l) = &link {
        if handoff::send(&hooks.data_dir, l) {
            return Ok(()); // the running instance signs in
        }
    }
    let _ = std::fs::create_dir_all(&hooks.data_dir);
    let slot = Arc::new(std::sync::Mutex::new(link));
    handoff::listen(&hooks.data_dir, slot.clone());
    hooks.take_login_link = Box::new(move || slot.lock().ok().and_then(|mut g| g.take()));
    run_with(hooks)
}

pub fn run_with(hooks: Hooks) -> Result<(), slint::PlatformError> {
    let _ = std::fs::create_dir_all(&hooks.data_dir);
    let client = shared_client_with(&hooks.data_dir, hooks.allow_insecure_http, &hooks.device_name, hooks.secrets.clone());
    let ui = AppWindow::new()?;
    let mut prefs = Prefs::load(&hooks.data_dir);
    // Migrate the old single-number zoom file of earlier builds.
    if let Some(s) = std::fs::read_to_string(hooks.data_dir.join("ui-scale")).ok().and_then(|s| s.trim().parse::<f32>().ok()) {
        if prefs.zoom == 100 {
            prefs.zoom = (s * 100.0).round().clamp(50.0, 300.0) as u16;
        }
        let _ = std::fs::remove_file(hooks.data_dir.join("ui-scale"));
    }
    let sh: Sh = Arc::new(Mutex::new(St { prefs, data_dir: hooks.data_dir.clone(), system_scale: (hooks.system_scale)(), limit: 60, ..Default::default() }));
    let hooks = Arc::new(hooks);
    let osh = sh.clone();
    ui.set_is_android(hooks.is_android);
    ui.set_can_record(hooks.media.can_record());
    ui.set_can_scan(hooks.scan_qr.is_some());
    nav::register(&ui);
    {
        let wk = ui.as_weak();
        ui.on_back_pressed(move || wk.upgrade().map(|ui| nav::handle_back(&ui)).unwrap_or(false));
        let h = hooks.clone();
        let wk = ui.as_weak();
        ui.on_scan_qr(move || {
            if let Some(ui) = wk.upgrade() {
                ui.set_login_error("".into());
            }
            match &h.scan_qr {
                Some(f) => f(),
                None => {
                    if let Some(ui) = wk.upgrade() {
                        ui.set_login_error("Scanning is not available here. Paste the link instead.".into());
                    }
                }
            }
        });
        let h = hooks.clone();
        let wk = ui.as_weak();
        ui.on_scan_image(move || {
            if let Some(ui) = wk.upgrade() {
                ui.set_login_error("".into());
            }
            if let Some(f) = &h.scan_image {
                f();
            }
        });
    }
    ui.set_close_tray(sh.lock().unwrap().prefs.close_to_tray);
    ui.set_version(format!("SuperChat {}", client_core::CLIENT_VERSION).into());
    ui.set_emoji_list(model(model::emoji_items(emoji::all())));
    ui.set_quick_emoji(model(model::emoji_items(["\u{1F44D}", "\u{2764}\u{FE0F}", "\u{1F602}", "\u{1F62E}", "\u{1F622}", "\u{1F64F}"].iter().map(|s| s.to_string()).collect())));
    {
        let g = sh.lock().unwrap();
        apply_prefs(&ui, &g);
        ui.set_f_server(g.prefs.last_server.clone().into());
        ui.set_f_user(g.prefs.last_username.clone().into());
    }
    if std::env::args().any(|a| a == "--minimised") {
        ui.window().set_minimized(true);
    }

    ui.set_signed_in(client.signed_in());
    if client.signed_in() {
        client.start();
        refresh(&ui, &client, &sh);
        sync_push(&client, &sh);
    }

    // Test and demo helper: sign in from the environment and open a screen (used by the
    // screenshot script; nothing here is reachable from the UI).
    if !client.signed_in() {
        if let (Ok(s), Ok(u), Ok(p)) = (proto::env_var("ATOMIC_AUTOLOGIN_SERVER"), proto::env_var("ATOMIC_AUTOLOGIN_USER"), proto::env_var("ATOMIC_AUTOLOGIN_PASS")) {
            ui.set_busy(true);
            do_login(&ui, &client, &sh, s, u, p, String::new(), proto::env_var("ATOMIC_AUTOLOGIN_PIN").ok());
        }
    }
    if let Ok(what) = proto::env_var("ATOMIC_UI_OPEN") {
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        let t = slint::Timer::default();
        let what2 = what.clone();
        t.start(slint::TimerMode::Repeated, Duration::from_millis(700), move || {
            let Some(ui) = wk.upgrade() else { return };
            if !ui.get_signed_in() || c.conversations().is_empty() || ui.get_conversations().row_count() == 0 {
                return;
            }
            // Runs once: stop by flagging through the prefs-less property.
            if ui.get_status().as_str() == "\u{0}done" {
                return;
            }
            let mut parts = what2.split(':');
            let screen = parts.next().unwrap_or("");
            let conv = parts.next().unwrap_or("");
            let third = what2.split(':').nth(2).unwrap_or("");
            let find = |name: &str| c.conversations().into_iter().find(|x| c.conversation_title(x).eq_ignore_ascii_case(name));
            let cs = find(conv).or_else(|| find(third)).or_else(|| c.conversations().into_iter().next());
            if let (Some(cs), false) = (&cs, screen.starts_with("lock") || screen == "tab") {
                select_conv(&ui, &c, &sh, &cs.conversation.id, true);
            }
            if screen.starts_with("lock") {
                // App Lock demo: PIN 123456, the named conversation locked.
                if let Some(cs) = &cs {
                    let _ = c.applock().set_pin("123456", "123456", client_core::applock::wall_ms());
                    let _ = c.applock().set_conv_locked(&cs.conversation.id, true);
                    if screen != "lockopen" {
                        c.applock().lock();
                    }
                    lock::changed(&ui, &c, &sh);
                    if screen == "lockpin" {
                        ui.invoke_lock_tapped();
                    }
                }
                ui.set_status("\u{0}done".into());
                return;
            }
            match screen {
                "settings" => ui.invoke_open_settings(),
                // Test hook for screenshots: Settings scrolled to the bottom (sign-out controls).
                "settings-bottom" => {
                    ui.invoke_open_settings();
                    ui.set_settings_scroll_y(if ui.get_sign_out_allowed() { -1000.0 } else { -600.0 });
                }
                "settings-page" => {
                    ui.invoke_open_settings();
                    ui.set_settings_page(conv.into());
                }
                "sheet" => ui.set_menu_kind(conv.into()),
                "info" | "details" => ui.invoke_open_details(conv.into()),
                "tab" => ui.invoke_go_home_tab(conv.into()),
                "addpeople" => ui.invoke_add_people(),
                "newmessage" => ui.invoke_new_message(),
                "help" => ui.set_menu_kind("help".into()),
                "labels" => {
                    ui.set_label_msg("".into());
                    ui.set_label_open(true);
                }
                "labelpick" => {
                    let n = ui.get_messages().row_count();
                    if let Some(m) = (0..n).rev().filter_map(|i| ui.get_messages().row_data(i)).find(|m| !m.deleted && !m.system) {
                        ui.invoke_label_picker(m.id);
                    }
                }
                "savedemo" => {
                    let n = ui.get_messages().row_count();
                    for (k, m) in (0..n).rev().filter_map(|i| ui.get_messages().row_data(i)).filter(|m| !m.deleted && !m.system).take(3).enumerate() {
                        let cc = c.clone();
                        let id = m.id.to_string();
                        let first = k == 0;
                        c.handle().spawn(async move {
                            let _ = cc.save_message(&id, None).await;
                            if first {
                                if let Ok(l) = cc.create_label("Follow up", "#e8590c").await {
                                    if let Some(l) = l.first() {
                                        let _ = cc.set_saved_labels(&id, &[l.id.clone()]).await;
                                    }
                                }
                            }
                        });
                    }
                    ui.set_main_tab("later".into());
                }
                "menu" => {
                    let n = ui.get_messages().row_count();
                    if let Some(m) = (0..n).rev().filter_map(|i| ui.get_messages().row_data(i)).find(|m| !m.deleted && !m.system && !m.mine) {
                        ui.set_sel_msg(m.clone());
                        ui.set_menu_msg(m.id.clone());
                        ui.set_menu_x(520.0);
                        ui.set_menu_y(260.0);
                        ui.set_menu_kind("msg".into());
                    }
                }
                "convmenu" => {
                    ui.set_menu_x(40.0);
                    ui.set_menu_y(260.0);
                    ui.invoke_open_conv_menu(ui.get_current_conv(), ui.get_current_title(), false);
                }
                "editdemo" => ui.invoke_up_edit(),
                "searchq" => {
                    ui.invoke_open_search();
                    ui.set_search_text(parts.next().unwrap_or("").into());
                    ui.invoke_search(ui.get_search_text());
                }
                "searchmenu" => {
                    ui.invoke_open_search();
                    ui.set_menu_x(380.0);
                    ui.set_menu_y(120.0);
                    ui.set_menu_kind(conv.into());
                }
                "msgsheet" => {
                    use slint::Model;
                    let n = ui.get_messages().row_count();
                    if let Some(m) = (0..n).rev().filter_map(|i| ui.get_messages().row_data(i)).find(|m| !m.deleted && !m.mine).or_else(|| ui.get_messages().row_data(n.saturating_sub(1))) {
                        ui.set_sel_msg(m.clone());
                        ui.set_menu_msg(m.id.clone());
                        ui.set_menu_kind("msg".into());
                    }
                }
                "search" => ui.invoke_open_search(),
                "threads" => ui.invoke_open_threads(),
                "switcher" => ui.invoke_open_switcher(),
                "people" => ui.invoke_open_people(),
                "thread" => {
                    let cur = sh.lock().unwrap().current.clone();
                    if let Some((root, _)) = c.reply_counts(&cur).into_iter().find(|(_, n)| *n > 0) {
                        open_thread(&ui, &c, &sh, &cur, &root);
                    }
                }
                "drafts" => ui.invoke_open_drafts(),
                "schedule" => {
                    ui.set_composer("Quarterly numbers are attached".into());
                    ui.set_schedule_open(true);
                }
                "home" => {
                    ui.set_tab(3);
                    ui.invoke_tab_changed(3);
                }
                "form" => {
                    let cur = sh.lock().unwrap().current.clone();
                    if let Some(t) = c.timeline(&cur, 50, None).into_iter().find(|t| t.message.form.is_some()) {
                        ui.invoke_form_action(t.message.id.clone().into(), "".into());
                    }
                }
                "bookmark" => ui.set_bookmark_open(true),
                "files" => ui.invoke_tab_changed(1),
                "pins" => ui.invoke_tab_changed(2),
                _ => {}
            }
            if screen == "files" {
                ui.set_tab(1);
            } else if screen == "pins" {
                ui.set_tab(2);
            } else if screen == "bookmarks" {
                ui.set_tab(4);
            }
            if screen == "search" {
                ui.set_search_text(parts.next().unwrap_or("").into());
                ui.invoke_search(ui.get_search_text());
            }
            ui.set_status("\u{0}done".into());
        });
        std::mem::forget(t);
    }

    // ---- tray icon (E-08), close to tray, file drop (F-02) -------------------------------------------------------------
    #[cfg(not(target_os = "android"))]
    let tray = if hooks.is_android { None } else { SuperChatTray::new().ok() };
    #[cfg(not(target_os = "android"))]
    {
        let has_tray = tray.is_some();
        if let Some(t) = &tray {
            let wk = ui.as_weak();
            let show = move || {
                if let Some(ui) = wk.upgrade() {
                    let _ = ui.show();
                    ui.window().set_minimized(false);
                }
            };
            t.on_open_app(move || show());
            t.on_quit_app(|| {
                let _ = slint::quit_event_loop();
            });
        }
        let sh2 = osh.clone();
        ui.window().on_close_requested(move || {
            if !has_tray || !sh2.lock().unwrap().prefs.close_to_tray {
                // Without a tray (or when the user prefers it) closing the window quits; a tray icon alone would keep the loop alive.
                let _ = slint::quit_event_loop();
            }
            slint::CloseRequestResponse::HideWindow
        });
        use slint::winit_030::{winit, EventResult, WinitWindowAccessor};
        let wk = ui.as_weak();
        ui.window().on_winit_window_event(move |_, ev| {
            use winit::event::WindowEvent as WE;
            match ev {
                WE::CursorMoved { .. } | WE::MouseInput { .. } | WE::MouseWheel { .. } | WE::KeyboardInput { .. } | WE::Touch(_) => lock::note_input(),
                WE::Focused(f) => lock::note_background(!*f),
                _ => {}
            }
            if let winit::event::WindowEvent::DroppedFile(p) = ev {
                if let Some(ui) = wk.upgrade() {
                    ui.invoke_attach(p.to_string_lossy().to_string().into());
                }
            }
            EventResult::Propagate
        });
    }

    // ---- client events -> UI refresh -------------------------------------------------------------
    {
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        let is_android = hooks.is_android;
        let mut rx = client.subscribe();
        client.handle().spawn(async move {
            loop {
                let ev = match rx.recv().await {
                    Ok(e) => e,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => ClientEvent::Changed { conv_ids: vec![], users: true, me: true },
                    Err(_) => break,
                };
                match ev {
                    ClientEvent::Typing { conv_id, user_id, .. } => {
                        let name = c.display_name(&user_id);
                        sh.lock().unwrap().typing = Some((conv_id, name, Instant::now()));
                    }
                    ClientEvent::Incoming { message, conversation, sender_name } => {
                        let (notif, preview, cur) = {
                            let g = sh.lock().unwrap();
                            (g.prefs.notifications, g.prefs.notification_preview, g.current.clone())
                        };
                        let me = c.me();
                        let dnd = me.as_ref().map(|m| m.presence_pref == "dnd").unwrap_or(false);
                        let my_id = c.my_id().unwrap_or_default();
                        let pref = c.pref(&message.conv_id);
                        let allowed = proto::rules::should_notify(Some(&pref), now_ms(), me.as_ref().map(|m| m.presence_pref.as_str()).unwrap_or("auto"), message.mentions.contains(&my_id));
                        if !is_android && notif && !dnd && allowed && (message.conv_id != cur || !c.signed_in()) {
                            // A locked conversation never shows its name or text, whether or not the app is unlocked.
                            let (title, body) = if c.is_conv_locked(&message.conv_id) {
                                ("SuperChat".to_string(), "New message".to_string())
                            } else {
                                let title = conversation.as_ref().map(|cs| c.conversation_title(cs)).unwrap_or_else(|| sender_name.clone());
                                (title, if preview { format!("{}: {}", sender_name, model::plain(&message.body)) } else { "New message".into() })
                            };
                            platform::notify(&title, &body);
                        }
                    }
                    ClientEvent::SignedOut { message, .. } => {
                        let wk = wk.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = wk.upgrade() {
                                ui.set_signed_in(false);
                                ui.set_login_error(message.into());
                                ui.set_messages(model(vec![]));
                                ui.set_conversations(model(vec![]));
                                ui.set_current_conv("".into());
                                ui.set_current_title("".into());
                            }
                        });
                    }
                    ClientEvent::Changed { .. } | ClientEvent::SendFailed { .. } | ClientEvent::Connection(_) | ClientEvent::Presence { .. } => {
                        let (wk, c, sh) = (wk.clone(), c.clone(), sh.clone());
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = wk.upgrade() {
                                if ui.get_signed_in() {
                                    refresh(&ui, &c, &sh);
                                }
                            }
                        });
                    }
                    _ => {}
                }
            }
        });
    }

    // ---- status line, typing indicator, system font scale, open-from-notification ------------------
    let timer = slint::Timer::default();
    {
        let (wk, c, sh, h2) = (ui.as_weak(), client.clone(), osh.clone(), hooks.clone());
        let mut lock_bg = false;
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(500), move || {
            let Some(ui) = wk.upgrade() else { return };
            if ui.get_signed_in() {
                lock::tick(&ui, &c, &sh, &mut lock_bg);
            }
            {
                let mut g = sh.lock().unwrap();
                let s = (h2.system_scale)();
                if (s - g.system_scale).abs() > 0.001 {
                    g.system_scale = s;
                    ui.global::<Theme>().set_scale(scale_of(&g));
                }
            }
            if !ui.get_playing_id().is_empty() && !h2.media.playing() {
                ui.set_playing_id("".into());
            }
            if let Some(sharing) = (h2.take_shared)() {
                if ui.get_signed_in() {
                    ui.set_share_text(if sharing.0.is_empty() { format!("{} file(s)", sharing.1.len()) } else { sharing.0.clone() }.into());
                    sh.lock().unwrap().shared = Some(sharing);
                    ui.set_menu_msg("share".into());
                    ui.set_menu_kind("share".into());
                }
            }
            if let Some(err) = (h2.take_login_error)() {
                if !ui.get_signed_in() {
                    ui.set_use_link(true);
                    ui.set_login_error(err.into());
                } else {
                    toast(&ui, &err);
                }
            }
            if let Some(link) = (h2.take_login_link)() {
                if !ui.get_signed_in() && !ui.get_busy() {
                    if proto::rules::parse_login_link(&link).is_some() {
                        ui.set_use_link(true);
                        ui.set_f_link(link.clone().into());
                        ui.invoke_login(ui.get_f_server(), "".into(), "".into(), link.into());
                    } else {
                        ui.set_login_error("That is not a SuperChat login link.".into());
                    }
                } else if ui.get_signed_in() {
                    toast(&ui, "Already signed in.");
                }
            }
            if let Some(conv) = (h2.take_open_conv)() {
                if ui.get_signed_in() && !c.is_hidden(&conv) {
                    select_conv(&ui, &c, &sh, &conv, true);
                }
            }
            if ui.get_status().as_str() != "\u{0}done" {
                ui.set_status(
                    match c.connection() {
                        ConnState::Online => "Connected".to_string(),
                        ConnState::Connecting => "Connecting...".to_string(),
                        ConnState::Offline { retry_in_secs } => format!("Offline. Messages you send are kept and sent when you are back. Retrying in {retry_in_secs}s"),
                        ConnState::SignedOut => String::new(),
                    }
                    .into(),
                );
            }
            let g = sh.lock().unwrap();
            let t = match &g.typing {
                Some((conv, name, at)) if *conv == g.current && at.elapsed() < Duration::from_secs(4) => format!("{name} is typing..."),
                _ => String::new(),
            };
            ui.set_typing_text(t.into());
        });
    }

    lock::install(&ui, &client, &osh);
    shell::install(&ui, &client, &osh);

    // ---- login screen helpers: a link pasted anywhere switches to link mode and supplies the server -------------
    {
        let wk = ui.as_weak();
        ui.on_server_edited(move |text| {
            if let Some(ui) = wk.upgrade() {
                adopt_login_text(&ui, text.as_str());
            }
        });
        let (wk, h2) = (ui.as_weak(), hooks.clone());
        ui.on_paste_clip(move || {
            if let (Some(ui), Some(f)) = (wk.upgrade(), h2.clipboard.as_ref()) {
                if let Some(t) = f() {
                    adopt_login_text(&ui, &t);
                    if ui.get_f_link().is_empty() {
                        ui.set_f_link(t.trim().to_string().into());
                    }
                } else {
                    ui.set_login_error("The clipboard is empty.".into());
                }
            }
        });
    }

    // ---- sign in (with certificate fingerprint confirmation) ----------------------------------------
    {
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_login(move |server, user, pass, link| {
            let Some(ui) = wk.upgrade() else { return };
            ui.set_busy(true);
            ui.set_login_error("".into());
            let (mut server, user, pass, mut link) = (server.to_string(), user.to_string(), pass.to_string(), link.to_string());
            // A link pasted into the server field (or surrounded by other text) is recognised here too.
            if let Some(l) = proto::rules::extract_login_link(&link).or_else(|| proto::rules::extract_login_link(&server)) {
                if let Some(srv) = proto::rules::parse_login_link(&l).and_then(|p| p.server) {
                    server = srv;
                }
                link = l;
                ui.set_use_link(true);
                ui.set_f_link(link.clone().into());
                ui.set_f_server(server.clone().into());
            }
            let probe_target = if link.trim().is_empty() { server.clone() } else { proto::rules::parse_login_link(&link).and_then(|p| p.server).unwrap_or(server.clone()) };
            let (wk, c, sh) = (wk.clone(), c.clone(), sh.clone());
            c.clone().handle().spawn(async move {
                // https servers with a certificate no public authority signed need the user's OK.
                let probe = if probe_target.trim().starts_with("http://") || probe_target.trim().is_empty() { None } else { c.probe_certificate(probe_target.trim()).await.ok() };
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = wk.upgrade() else { return };
                    match probe {
                        Some(p) if !p.publicly_trusted => {
                            ui.set_tofu_fp(p.fingerprint.clone().into());
                            ui.set_tofu_host(probe_target.trim().trim_start_matches("https://").trim_end_matches('/').to_string().into());
                            ui.set_tofu_open(true);
                            ui.set_busy(false);
                            sh.lock().unwrap().pending_login = Some((server, user, pass, link, p.fingerprint));
                        }
                        _ => do_login(&ui, &c, &sh, server, user, pass, link, None),
                    }
                });
            });
        });
    }
    {
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_tofu_accept(move || {
            let Some(ui) = wk.upgrade() else { return };
            ui.set_tofu_open(false);
            let p = sh.lock().unwrap().pending_login.take();
            if let Some((server, user, pass, link, fp)) = p {
                ui.set_busy(true);
                do_login(&ui, &c, &sh, server, user, pass, link, Some(fp));
            }
        });
        let (wk, sh) = (ui.as_weak(), osh.clone());
        ui.on_tofu_cancel(move || {
            sh.lock().unwrap().pending_login = None;
            if let Some(ui) = wk.upgrade() {
                ui.set_tofu_open(false);
                ui.set_login_error("Sign in cancelled: the server certificate was not trusted.".into());
            }
        });
    }

    // ---- conversations, history, navigation ------------------------------------------------------------
    {
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_select_conv(move |id| {
            if let Some(ui) = wk.upgrade() {
                select_conv(&ui, &c, &sh, &id, true);
            }
        });
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_nav_back(move || {
            let id = {
                let mut g = sh.lock().unwrap();
                if g.hist_pos == 0 {
                    return;
                }
                g.hist_pos -= 1;
                g.hist[g.hist_pos].clone()
            };
            if let Some(ui) = wk.upgrade() {
                select_conv(&ui, &c, &sh, &id, false);
            }
        });
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_nav_forward(move || {
            let id = {
                let mut g = sh.lock().unwrap();
                if g.hist_pos + 1 >= g.hist.len() {
                    return;
                }
                g.hist_pos += 1;
                g.hist[g.hist_pos].clone()
            };
            if let Some(ui) = wk.upgrade() {
                select_conv(&ui, &c, &sh, &id, false);
            }
        });
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_tab_changed(move |t| {
            let Some(ui) = wk.upgrade() else { return };
            if t == 1 {
                let cur = sh.lock().unwrap().current.clone();
                let (wk, c2, sh2) = (ui.as_weak(), c.clone(), sh.clone());
                c.handle().spawn(async move {
                    let _ = c2.list_files(Some(&cur), None, None, None).await;
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = wk.upgrade() {
                            refresh(&ui, &c2, &sh2);
                        }
                    });
                });
            }
            if t == 3 {
                load_home(&ui, &c, &sh);
            }
            refresh(&ui, &c, &sh);
        });
    }
    {
        // Older history: grow the window, fetch from the server when the cache runs out.
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_load_older(move || {
            let Some(ui) = wk.upgrade() else { return };
            let (cur, cached) = {
                let mut g = sh.lock().unwrap();
                if g.loading_older || g.current.is_empty() {
                    return;
                }
                g.loading_older = true;
                g.keep_scroll = Some((ui.get_list_y(), ui.get_list_h()));
                g.limit += 50;
                (g.current.clone(), c.timeline(&g.current, g.limit, None).len())
            };
            let (wk, c2, sh2) = (ui.as_weak(), c.clone(), sh.clone());
            c.handle().spawn(async move {
                let limit = sh2.lock().unwrap().limit;
                if cached < limit && c2.has_more_before(&cur) {
                    let _ = c2.load_older(&cur, None).await;
                }
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = wk.upgrade() {
                        refresh(&ui, &c2, &sh2);
                    }
                    // Allow the next fetch only after the layout settled.
                    let sh3 = sh2.clone();
                    slint::Timer::single_shot(Duration::from_millis(400), move || sh3.lock().unwrap().loading_older = false);
                });
            });
            let _ = ui;
        });
        let (wk, sh) = (ui.as_weak(), osh.clone());
        ui.on_jump_unread(move || {
            let (idx, n) = {
                let g = sh.lock().unwrap();
                (g.first_unread, g.msg_count)
            };
            if let (Some(ui), Some(i)) = (wk.upgrade(), idx) {
                ui.invoke_scroll_to_fraction((i as f32 / n.max(1) as f32).clamp(0.0, 1.0));
            }
        });
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_jump_date(move |text| {
            let Some(ui) = wk.upgrade() else { return };
            let Some(ts) = datefmt::parse_date(text.trim(), tz_offset()) else {
                toast(&ui, "Type a date like 17/11/2026.");
                return;
            };
            let cur = sh.lock().unwrap().current.clone();
            let (wk, c2, sh2) = (ui.as_weak(), c.clone(), sh.clone());
            c.handle().spawn(async move {
                let m = c2.jump_to_date(&cur, ts).await.ok().flatten();
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = wk.upgrade() else { return };
                    match m {
                        Some(m) => show_message(&ui, &c2, &sh2, &cur, &m.id),
                        None => toast(&ui, "Nothing was sent on or after that date."),
                    }
                });
            });
        });
    }

    // ---- composing --------------------------------------------------------------------------------------------
    {
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_send(move |text| {
            let t = text.trim().to_string();
            let cur = sh.lock().unwrap().current.clone();
            if t.is_empty() || cur.is_empty() {
                return;
            }
            let quote = wk.upgrade().map(|u| u.get_quote_id().to_string()).filter(|q| !q.is_empty());
            if let Err(e) = c.send_text(&cur, &t, SendOptions { thread_root: None, reply_to: quote }) {
                if let Some(ui) = wk.upgrade() {
                    toast(&ui, &e.to_string());
                }
                return;
            }
            c.save_draft(&cur, None, "");
            if let Some(ui) = wk.upgrade() {
                ui.set_quote_id("".into());
                ui.set_quote_text("".into());
                ui.set_mentions(model(vec![]));
                sh.lock().unwrap().scroll_after = None;
                refresh(&ui, &c, &sh);
                ui.invoke_scroll_bottom();
            }
        });
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_thread_send(move |text| {
            let t = text.trim().to_string();
            let Some((conv, root)) = sh.lock().unwrap().thread.clone() else { return };
            if t.is_empty() {
                return;
            }
            if let Err(e) = c.send_thread_reply(&conv, &root, &t) {
                if let Some(ui) = wk.upgrade() {
                    toast(&ui, &e.to_string());
                }
            }
            if let Some(ui) = wk.upgrade() {
                // "Also send to #channel": the same text goes to the channel too, quoting the thread's first message.
                if ui.get_thread_also() {
                    let _ = c.send_text(&conv, &t, SendOptions { thread_root: None, reply_to: Some(root.clone()) });
                    ui.set_thread_also(false);
                }
                refresh(&ui, &c, &sh);
            }
        });
        let c = client.clone();
        ui.on_save_edit(move |id, text| {
            let (c, id, text) = (c.clone(), id.to_string(), text.trim().to_string());
            if text.is_empty() {
                return;
            }
            c.clone().handle().spawn(async move {
                let _ = c.edit_message(&id, &text).await;
            });
        });
        let c = client.clone();
        ui.on_begin_edit(move |id| c.store().message(&id).ok().flatten().map(|m| m.body).unwrap_or_default().into());
        let c = client.clone();
        ui.on_delete_msg(move |id| {
            let (c, id) = (c.clone(), id.to_string());
            c.clone().handle().spawn(async move {
                let _ = c.delete_message(&id).await;
            });
        });
        let c = client.clone();
        ui.on_react(move |id, emoji| {
            let (c, id, emoji) = (c.clone(), id.to_string(), emoji.to_string());
            let me = c.my_id().unwrap_or_default();
            let mine = c.store().message(&id).ok().flatten().map(|m| m.reactions.iter().any(|r| r.emoji == emoji && r.user_ids.contains(&me))).unwrap_or(false);
            c.clone().handle().spawn(async move {
                let _ = c.react(&id, &emoji, !mine).await;
            });
        });
        let (c, sh) = (client.clone(), osh.clone());
        ui.on_retry(move |id| {
            if let Some(cid) = c.store().message(&id).ok().flatten().and_then(|m| m.client_id).or_else(|| {
                let cur = sh.lock().unwrap().current.clone();
                c.timeline(&cur, 300, None).into_iter().find(|t| t.message.id == id.as_str()).and_then(|t| t.message.client_id)
            }) {
                let _ = c.retry_send(&cid);
            }
        });
    }
    {
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_attach(move |path| {
            let cur = sh.lock().unwrap().current.clone();
            let p = PathBuf::from(path.trim());
            if !cur.is_empty() && p.is_file() {
                if let Err(e) = c.send_files(&cur, "", &[p], SendOptions::default()) {
                    if let Some(ui) = wk.upgrade() {
                        toast(&ui, &e.to_string());
                    }
                }
            } else if let Some(ui) = wk.upgrade() {
                toast(&ui, "That file was not found.");
            }
        });
        let wk = ui.as_weak();
        ui.on_browse_file(move || {
            let wk = wk.clone();
            std::thread::spawn(move || {
                if let Some(p) = platform::pick_file() {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = wk.upgrade() {
                            ui.set_attach_path(p.display().to_string().into());
                        }
                    });
                }
            });
        });
        let (c, sh) = (client.clone(), osh.clone());
        ui.on_typing({
            let last = Arc::new(Mutex::new(Instant::now() - Duration::from_secs(10)));
            move || {
                let mut l = last.lock().unwrap();
                if l.elapsed() > Duration::from_secs(3) {
                    *l = Instant::now();
                    let cur = sh.lock().unwrap().current.clone();
                    if !cur.is_empty() {
                        c.typing(&cur);
                    }
                }
            }
        });
        // @mention autocomplete and draft saving.
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_composer_edited(move |t| {
            let Some(ui) = wk.upgrade() else { return };
            let cur = sh.lock().unwrap().current.clone();
            c.save_draft(&cur, None, &t);
            let q = markup::mention_query(&t, t.chars().count());
            let list = match (q, c.conversations().into_iter().find(|x| x.conversation.id == cur)) {
                (Some(q), Some(cs)) => {
                    let me = c.my_id().unwrap_or_default();
                    let q = q.to_lowercase();
                    cs.conversation
                        .members
                        .iter()
                        .filter(|m| m.user_id != me)
                        .filter_map(|m| c.user(&m.user_id))
                        .filter(|u| !u.disabled && (q.is_empty() || u.username.to_lowercase().starts_with(&q) || u.display_name.to_lowercase().contains(&q)))
                        .take(6)
                        .map(|u| {
                            let mut p = model::person(&u);
                            p.id = u.username.clone().into();
                            p
                        })
                        .collect::<Vec<_>>()
                        .into_iter()
                        .chain(special_mentions(&c, &cs.conversation.kind, &q))
                        .collect()
                }
                _ => vec![],
            };
            ui.set_mentions(model(list));
            ui.set_emoji_sugg(model(match shortcodes::query(&t) {
                Some(q) => model::emoji_items(shortcodes::matches(&q)),
                None => vec![],
            }));
        });
        let wk = ui.as_weak();
        ui.on_pick_mention(move |username| {
            let Some(ui) = wk.upgrade() else { return };
            let t = ui.get_composer().to_string();
            if let Some(at) = t.rfind('@') {
                ui.set_composer(format!("{}@{} ", &t[..at], username).into());
            }
            ui.set_mentions(model(vec![]));
        });
        // Formatting buttons act on the last word (or add a prefix) because the text field exposes no selection.
        let wk = ui.as_weak();
        ui.on_format(move |marker| {
            let Some(ui) = wk.upgrade() else { return };
            let t = ui.get_composer().to_string();
            let m = marker.as_str();
            let out = if m == "> " || m == "- " || m == "1. " {
                if t.starts_with(m) { t[m.len()..].to_string() } else { format!("{m}{t}") }
            } else if m == "```\n" {
                format!("{t}```\n\n```")
            } else {
                let chars: Vec<char> = t.chars().collect();
                let end = chars.len();
                let start = chars.iter().rposition(|c| c.is_whitespace()).map(|i| i + 1).unwrap_or(0);
                if start >= end || chars[end - 1].is_whitespace() {
                    format!("{t}{m}{m}")
                } else {
                    markup::wrap_selection(&t, start, end, m).0
                }
            };
            ui.set_composer(out.into());
        });
        let (wk, c) = (ui.as_weak(), client.clone());
        ui.on_emoji_picked(move |e| {
            let Some(ui) = wk.upgrade() else { return };
            let target = ui.get_picker_target().to_string();
            if target.is_empty() {
                ui.set_composer(format!("{}{}", ui.get_composer(), e).into());
            } else {
                ui.invoke_react(target.into(), e);
            }
            let _ = &c;
        });
    }

    // ---- message actions ---------------------------------------------------------------------------------------
    {
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_open_thread(move |id| {
            let Some(ui) = wk.upgrade() else { return };
            let conv = c.store().message(&id).ok().flatten().map(|m| m.conv_id).unwrap_or_else(|| sh.lock().unwrap().current.clone());
            open_thread(&ui, &c, &sh, &conv, &id);
        });
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_close_thread(move || {
            sh.lock().unwrap().thread = None;
            if let Some(ui) = wk.upgrade() {
                ui.set_thread_id("".into());
                refresh(&ui, &c, &sh);
            }
        });
        let (c, sh) = (client.clone(), osh.clone());
        ui.on_pin(move |id| {
            let cur = sh.lock().unwrap().current.clone();
            let pinned = c.pins(&cur).iter().any(|p| p.message_id == id.as_str());
            let (c, id) = (c.clone(), id.to_string());
            c.clone().handle().spawn(async move {
                let _ = c.pin_message(&cur, &id, !pinned).await;
            });
        });
        let (wk, c) = (ui.as_weak(), client.clone());
        ui.on_copy_link(move |id| {
            let Some(ui) = wk.upgrade() else { return };
            if let Some(m) = c.store().message(&id).ok().flatten() {
                let ok = platform::copy_text(&c.permalink(&m));
                toast(&ui, if ok { "Link copied." } else { "No clipboard tool found (install wl-clipboard or xclip)." });
            }
        });
        let (wk, c) = (ui.as_weak(), client.clone());
        let shd = osh.clone();
        ui.on_forward_to(move |id, conv| {
            if id.as_str() == "share" {
                let Some((text, files)) = shd.lock().unwrap().shared.take() else { return };
                if !text.trim().is_empty() {
                    let _ = c.send_text(&conv, text.trim(), SendOptions::default());
                }
                if !files.is_empty() {
                    let _ = c.send_files(&conv, "", &files, SendOptions::default());
                }
                if let Some(ui) = wk.upgrade() {
                    select_conv(&ui, &c, &shd, &conv, true);
                    toast(&ui, "Shared");
                }
                return;
            }
            let (wk, c2, id, conv) = (wk.clone(), c.clone(), id.to_string(), conv.to_string());
            c.handle().spawn(async move {
                let r = c2.forward_message(&id, &[conv], None).await;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = wk.upgrade() {
                        toast(&ui, if r.is_ok() { "Message forwarded." } else { "Could not forward the message." });
                    }
                });
            });
        });
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_open_file(move |id| {
            let att = sh.lock().unwrap().atts.get(id.as_str()).cloned();
            let Some(a) = att else { return };
            let (wk, c2) = (wk.clone(), c.clone());
            c.handle().spawn(async move {
                match c2.download_attachment(&a).await {
                    Ok(p) => platform::open_path(&p),
                    Err(e) => {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = wk.upgrade() {
                                toast(&ui, &format!("Could not download the file: {e}"));
                            }
                        });
                    }
                }
            });
        });
        let (wk, c) = (ui.as_weak(), client.clone());
        ui.on_open_link(move |url| {
            let u = url.to_string();
            if let Some(ui) = wk.upgrade() {
                if u.starts_with("superchat://") || u.starts_with("atomic-chat://") {
                    let (wk, c2) = (ui.as_weak(), c.clone());
                    c.handle().spawn(async move {
                        let m = c2.open_permalink(&u).await;
                        let _ = slint::invoke_from_event_loop(move || {
                            if let (Some(ui), Ok(m)) = (wk.upgrade(), m) {
                                let sh = ui.get_current_conv();
                                let _ = sh;
                                ui.invoke_row_clicked("pin".into(), m.id.clone().into(), m.conv_id.clone().into());
                            }
                        });
                    });
                    return;
                }
            }
            platform::open_url(&u);
        });
    }

    // ---- voice messages (M-30), audio playback, pasted images (F-02) ------------------------------------------------
    {
        let (wk, c, sh, h2) = (ui.as_weak(), client.clone(), osh.clone(), hooks.clone());
        ui.on_play_audio(move |id| {
            let Some(ui) = wk.upgrade() else { return };
            let id = id.to_string();
            if ui.get_playing_id().as_str() == id {
                h2.media.stop();
                ui.set_playing_id("".into());
                return;
            }
            let Some(a) = sh.lock().unwrap().atts.get(&id).cloned() else { return };
            ui.set_playing_id(id.clone().into());
            let (wk, c2, media) = (ui.as_weak(), c.clone(), h2.media.clone());
            c.handle().spawn(async move {
                let path = match c2.cached_attachment_path(&a) {
                    Some(p) => Some(p),
                    None => c2.download_attachment(&a).await.ok().and_then(|_| c2.cached_attachment_path(&a)),
                };
                let ok = path.map(|p| media.play(&p)).unwrap_or(false);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = wk.upgrade() {
                        if !ok {
                            ui.set_playing_id("".into());
                            toast(&ui, "No audio player was found. The file is saved in the downloads folder.");
                        }
                    }
                });
            });
        });
        let (wk, c, sh, h2) = (ui.as_weak(), client.clone(), osh.clone(), hooks.clone());
        ui.on_record_toggle(move || {
            let Some(ui) = wk.upgrade() else { return };
            let mut g = sh.lock().unwrap();
            if !g.recording {
                if h2.media.record_start() {
                    g.recording = true;
                    ui.set_recording(true);
                } else {
                    toast(&ui, "Recording is not available (allow the microphone, or install arecord).");
                }
                return;
            }
            g.recording = false;
            ui.set_recording(false);
            let cur = g.current.clone();
            drop(g);
            match h2.media.record_stop() {
                Some(p) if !cur.is_empty() => {
                    if let Err(e) = c.send_files(&cur, "", &[p], SendOptions::default()) {
                        toast(&ui, &e.to_string());
                    } else {
                        toast(&ui, "Voice message sent");
                    }
                }
                _ => toast(&ui, "Nothing was recorded."),
            }
        });
        let (wk, sh) = (ui.as_weak(), osh.clone());
        ui.on_paste_image(move || {
            let Some(ui) = wk.upgrade() else { return };
            let dir = sh.lock().unwrap().data_dir.join("pasted");
            let _ = std::fs::create_dir_all(&dir);
            match platform::clipboard_image(&dir) {
                Some(p) => ui.invoke_attach(p.to_string_lossy().to_string().into()),
                None => toast(&ui, "There is no image on the clipboard."),
            }
        });
    }

    // ---- bot forms (A-03), App Home (A-04) -------------------------------------------------------------------------
    {
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_form_action(move |msg_id, button| {
            let Some(ui) = wk.upgrade() else { return };
            let Some(m) = c.store().message(&msg_id).ok().flatten() else { return };
            let Some(form) = m.form.clone() else { return };
            if button.is_empty() {
                let st = forms::FormState::new(forms::Src::Message(msg_id.to_string()), None, form);
                show_form(&ui, st.clone(), "Form");
                sh.lock().unwrap().form = Some(st);
                return;
            }
            let Some(b) = form.buttons.iter().find(|b| b.id == button.as_str()).cloned() else { return };
            if let Some(sub) = b.form.as_deref() {
                let st = forms::FormState::new(forms::Src::Message(msg_id.to_string()), Some(b.id.clone()), sub.clone());
                show_form(&ui, st.clone(), &b.label);
                sh.lock().unwrap().form = Some(st);
            } else {
                let (wk, c2, id) = (ui.as_weak(), c.clone(), msg_id.to_string());
                c.handle().spawn(async move {
                    let r = c2.submit_form(&id, Some(&b.id), Default::default()).await;
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = wk.upgrade() {
                            toast(&ui, &match r {
                                Ok(_) => "Sent".to_string(),
                                Err(e) => e.to_string(),
                            });
                        }
                    });
                });
            }
        });
        let (wk, sh) = (ui.as_weak(), osh.clone());
        ui.on_form_text(move |i, t| {
            let _ = &wk;
            if let Some(f) = sh.lock().unwrap().form.as_mut() {
                f.set_text(i as usize, &t);
            }
        });
        let (wk, sh) = (ui.as_weak(), osh.clone());
        ui.on_form_toggle(move |i, opt| {
            let Some(ui) = wk.upgrade() else { return };
            let mut g = sh.lock().unwrap();
            if let Some(f) = g.form.as_mut() {
                f.toggle(i as usize, &opt);
                ui.set_form_fields(model(f.items()));
            }
        });
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_form_submit(move |_| {
            let Some(ui) = wk.upgrade() else { return };
            let Some(st) = sh.lock().unwrap().form.clone() else { return };
            let values = st.submit_values();
            let (wk, c2) = (ui.as_weak(), c.clone());
            c.handle().spawn(async move {
                let r = match &st.src {
                    forms::Src::Message(id) => c2.submit_form(id, st.button.as_deref(), values).await.map(|_| ()),
                    forms::Src::Home { bot, form_id } => c2.home_submit(bot, Some(form_id), st.button.as_deref(), values).await,
                };
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = wk.upgrade() else { return };
                    match r {
                        Ok(()) => {
                            ui.set_form_open(false);
                            toast(&ui, "Response sent");
                        }
                        Err(e) => ui.set_form_error(e.to_string().into()),
                    }
                });
            });
        });
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_home_action(move |kind, id| {
            let Some(ui) = wk.upgrade() else { return };
            let (bot, form) = {
                let g = sh.lock().unwrap();
                (g.home_bot.clone(), g.home_forms.iter().find(|f| f.id == id.as_str()).cloned())
            };
            if kind.as_str() == "form" {
                if let Some(f) = form {
                    let st = forms::FormState::new(forms::Src::Home { bot, form_id: f.id.clone() }, None, f);
                    show_form(&ui, st.clone(), "Form");
                    sh.lock().unwrap().form = Some(st);
                }
                return;
            }
            let (wk, c2, id) = (ui.as_weak(), c.clone(), id.to_string());
            c.handle().spawn(async move {
                let r = c2.home_submit(&bot, None, Some(&id), Default::default()).await;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = wk.upgrade() {
                        toast(&ui, &match r {
                            Ok(()) => "Sent".to_string(),
                            Err(e) => e.to_string(),
                        });
                    }
                });
            });
        });
    }

    // ---- scheduled messages (M-23), drafts and sent (M-24), bookmarks (C-07), saved searches (SR-06) ------------------
    {
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_schedule(move |when| {
            let Some(ui) = wk.upgrade() else { return };
            let cur = sh.lock().unwrap().current.clone();
            let body = ui.get_composer().trim().to_string();
            if body.is_empty() {
                ui.set_schedule_error("Type the message first, then choose when to send it.".into());
                return;
            }
            let due = match sched::due_ms(&when, now_ms(), tz_offset()) {
                Ok(d) => d,
                Err(e) => {
                    ui.set_schedule_error(e.into());
                    return;
                }
            };
            let (wk, c2, sh2) = (ui.as_weak(), c.clone(), sh.clone());
            c.handle().spawn(async move {
                let r = c2.schedule_message(&cur, &body, due, None).await;
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = wk.upgrade() else { return };
                    match r {
                        Ok(item) => {
                            ui.set_composer("".into());
                            c2.save_draft(&cur, None, "");
                            ui.set_schedule_open(false);
                            let g = sh2.lock().unwrap();
                            let off = tz_offset();
                            toast(&ui, &format!("Scheduled for {} {}", datefmt::format_date(item.due_ms, off), datefmt::format_time(item.due_ms, off, g.prefs.hour24)));
                            drop(g);
                            refresh(&ui, &c2, &sh2);
                        }
                        Err(e) => ui.set_schedule_error(e.to_string().into()),
                    }
                });
            });
        });
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_open_drafts(move || {
            let Some(ui) = wk.upgrade() else { return };
            let hour24 = sh.lock().unwrap().prefs.hour24;
            ui.set_drafts_rows(model(draft_rows(&c, tz_offset(), hour24)));
            ui.set_screen("scheduled".into());
            let (wk, c2, sh2) = (ui.as_weak(), c.clone(), sh.clone());
            c.handle().spawn(async move {
                let _ = c2.refresh_scheduled().await;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = wk.upgrade() {
                        let hour24 = sh2.lock().unwrap().prefs.hour24;
                        ui.set_drafts_rows(model(draft_rows(&c2, tz_offset(), hour24)));
                    }
                });
            });
        });
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_cancel_scheduled(move |id| {
            let (wk, c2, sh2, id) = (wk.clone(), c.clone(), sh.clone(), id.to_string());
            c.handle().spawn(async move {
                let r = c2.cancel_scheduled(&id).await;
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = wk.upgrade() else { return };
                    if let Err(e) = r {
                        toast(&ui, &e.to_string());
                    }
                    let hour24 = sh2.lock().unwrap().prefs.hour24;
                    ui.set_drafts_rows(model(draft_rows(&c2, tz_offset(), hour24)));
                    refresh(&ui, &c2, &sh2);
                });
            });
        });
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_add_bookmark(move |title, url| {
            let Some(ui) = wk.upgrade() else { return };
            let cur = sh.lock().unwrap().current.clone();
            let (title, url) = (title.trim().to_string(), url.trim().to_string());
            if url.is_empty() || !(url.starts_with("http://") || url.starts_with("https://")) {
                toast(&ui, "Enter a link that starts with http:// or https://");
                return;
            }
            let title = if title.is_empty() { url.clone() } else { title };
            ui.set_bookmark_open(false);
            let (wk, c2) = (ui.as_weak(), c.clone());
            c.handle().spawn(async move {
                let r = c2.add_bookmark(&cur, &proto::types::BookmarkRequest { title, url: Some(url), attachment_id: None, position: None }).await;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = wk.upgrade() {
                        toast(&ui, &match r {
                            Ok(_) => "Bookmark added".to_string(),
                            Err(e) => e.to_string(),
                        });
                    }
                });
            });
        });
        let (wk, sh) = (ui.as_weak(), osh.clone());
        ui.on_save_search(move |q| {
            let Some(ui) = wk.upgrade() else { return };
            let q = q.trim().to_string();
            let mut g = sh.lock().unwrap();
            let msg = if let Some(p) = g.prefs.saved_searches.iter().position(|x| *x == q) {
                g.prefs.saved_searches.remove(p);
                "Saved search removed"
            } else {
                g.prefs.saved_searches.insert(0, q);
                g.prefs.saved_searches.truncate(20);
                "Search saved"
            };
            save_prefs(&g);
            ui.set_saved(model(g.prefs.saved_searches.iter().map(|q| row(0, "", "", q.clone(), String::new(), String::new())).collect()));
            drop(g);
            toast(&ui, msg);
        });
    }

    // ---- search, switcher, threads list, people ------------------------------------------------------------------
    {
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_open_search(move || {
            let Some(ui) = wk.upgrade() else { return };
            let recents: Vec<RowItem> = sh.lock().unwrap().prefs.recent_searches.iter().map(|q| row(0, "", "", q.clone(), String::new(), String::new())).collect();
            ui.set_recents(model(recents));
            ui.set_saved(model(sh.lock().unwrap().prefs.saved_searches.iter().map(|q| row(0, "", "", q.clone(), String::new(), String::new())).collect()));
            ui.set_ai_answer("".into());
            ui.set_ai_note("".into());
            ui.set_search_tab(0);
            // The admin may have switched AI on or off since we signed in.
            let (wk2, c2) = (ui.as_weak(), c.clone());
            c.handle().spawn(async move {
                let _ = c2.refresh_server_meta().await;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = wk2.upgrade() {
                        ui.set_ai_enabled(c2.ai_enabled());
                        if !c2.ai_enabled() {
                            ui.set_ai_on(false);
                        }
                    }
                });
            });
            ui.set_results(model(vec![]));
            ui.set_search_text("".into());
            ui.set_screen("search".into());
            let _ = &c;
        });
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_search(move |q| {
            let Some(ui) = wk.upgrade() else { return };
            let q = q.trim().to_string();
            if q.is_empty() && ui.get_search_tab() == 1 {
                // The global Files view: recent files in every conversation you can see.
                let (wk2, c2, sh2) = (ui.as_weak(), c.clone(), sh.clone());
                c.handle().spawn(async move {
                    let off = tz_offset();
                    let files = c2.list_files(None, None, None, None).await.unwrap_or_default();
                    let mut rows: Vec<RowItem> = vec![];
                    {
                        let mut g = sh2.lock().unwrap();
                        for fh in files {
                            rows.push(row(1, &fh.attachment.id, &fh.conv_id, format!("{}  in {}  |  {}", c2.display_name(&fh.sender_id), conv_title_of_id(&c2, &fh.conv_id), datefmt::format_date(fh.ts, off)), fh.attachment.name.clone(), String::new()));
                            g.atts.insert(fh.attachment.id.clone(), fh.attachment);
                        }
                    }
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = wk2.upgrade() {
                            ui.set_results(model(rows));
                        }
                    });
                });
                return;
            }
            if q.is_empty() {
                ui.set_results(model(vec![]));
                return;
            }
            let gen = {
                let mut g = sh.lock().unwrap();
                g.search_gen += 1;
                g.search_gen
            };
            let tab_now = ui.get_search_tab();
            let ai = ui.get_ai_on() && ui.get_ai_enabled();
            if ai {
                ui.set_ai_busy(true);
            }
            let (wk, c2, sh2) = (ui.as_weak(), c.clone(), sh.clone());
            c.handle().spawn(async move {
                tokio::time::sleep(Duration::from_millis(if ai { 0 } else { 250 })).await;
                if sh2.lock().unwrap().search_gen != gen {
                    return;
                }
                let off = tz_offset();
                let f = searchq::parse(&q, off);
                let (r, answer, note) = if ai {
                    match c2.ai_search(&q, true).await {
                        Ok(a) => {
                            let i = &a.interpreted;
                            let mut note = format!("Searched for \"{}\"", i.q);
                            if let Some(from) = &i.from {
                                note.push_str(&format!(" from {from}"));
                            }
                            (Ok(a.results), a.answer.unwrap_or_default(), note)
                        }
                        Err(e) => {
                            let msg = e.to_string();
                            (Err(e), String::new(), msg)
                        }
                    }
                } else {
                    (c2.search_filtered(&f).await, String::new(), String::new())
                };
                let mut rows: Vec<RowItem> = vec![];
                if let Ok(r) = r {
                    for h in &r.hits {
                        let title = format!("{}  in {}  |  {}", c2.display_name(&h.message.sender_id), c2.conversations().into_iter().find(|x| x.conversation.id == h.message.conv_id).map(|cs| c2.conversation_title(&cs)).unwrap_or_default(), datefmt::format_date(h.message.ts, off));
                        let flat = |t: &str| model::plain(t).split_whitespace().collect::<Vec<_>>().join(" ");
                        let mut it = row(0, &h.message.id, &h.message.conv_id, title, flat(&h.snippet.replace("[[", "").replace("]]", "")), String::new());
                        // The server marks the match as [[...]]; show it highlighted.
                        if let (Some(a), Some(b)) = (h.snippet.find("[["), h.snippet.find("]]")) {
                            if a < b {
                                it.pre = flat(&h.snippet[..a]).into();
                                it.hit = h.snippet[a + 2..b].to_string().into();
                                it.post = flat(&h.snippet[b + 2..]).into();
                                if h.snippet[..a].ends_with(' ') {
                                    it.pre = format!("{} ", it.pre).into();
                                }
                                if h.snippet[b + 2..].starts_with(' ') {
                                    it.post = format!(" {}", it.post).into();
                                }
                            }
                        }
                        rows.push(it);
                    }
                    let start = rows.len();
                    for fh in &r.files {
                        rows.push(row(1, &fh.attachment.id, &fh.conv_id, format!("{}  |  {}", c2.display_name(&fh.sender_id), datefmt::format_date(fh.ts, off)), fh.attachment.name.clone(), String::new()));
                    }
                    if rows.len() > start {
                        rows[start].header = "Files".into();
                    }
                    let start = rows.len();
                    for u in &r.people {
                        rows.push(row(2, &u.id, "", format!("@{}", u.username), u.display_name.clone(), String::new()));
                    }
                    if rows.len() > start {
                        rows[start].header = "People".into();
                    }
                    let start = rows.len();
                    for cs in &r.channels {
                        let t = c2.conversation_title(cs);
                        rows.push(row(3, &cs.conversation.id, &cs.conversation.id, "Channel".into(), t, String::new()));
                    }
                    if rows.len() > start {
                        rows[start].header = "Channels".into();
                    }
                    let mut g = sh2.lock().unwrap();
                    for fh in r.files {
                        g.atts.insert(fh.attachment.id.clone(), fh.attachment);
                    }
                    drop(g);
                    let tab = tab_now;
                    rows.retain(|r| r.kind == tab);
                    for r in rows.iter_mut() {
                        r.header = Default::default();
                    }
                }
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = wk.upgrade() {
                        ui.set_results(model(rows));
                        ui.set_ai_busy(false);
                        ui.set_ai_answer(answer.into());
                        ui.set_ai_note(note.into());
                    }
                });
            });
        });
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_result_clicked(move |kind, id, conv| {
            let Some(ui) = wk.upgrade() else { return };
            {
                let q = ui.get_search_text().to_string();
                let mut g = sh.lock().unwrap();
                g.prefs.remember_search(&q);
                save_prefs(&g);
            }
            ui.set_screen("".into());
            match kind {
                0 => show_message(&ui, &c, &sh, &conv, &id),
                1 => ui.invoke_open_file(id),
                2 => ui.invoke_start_dm(id),
                _ => select_conv(&ui, &c, &sh, &id, true),
            }
        });
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_row_clicked(move |kind, id, conv| {
            let Some(ui) = wk.upgrade() else { return };
            ui.set_screen("".into());
            match kind.as_str() {
                "conv" => select_conv(&ui, &c, &sh, &id, true),
                "thread" => {
                    select_conv(&ui, &c, &sh, &conv, true);
                    open_thread(&ui, &c, &sh, &conv, &id);
                }
                _ => show_message(&ui, &c, &sh, &conv, &id),
            }
        });
        let (wk, c) = (ui.as_weak(), client.clone());
        ui.on_open_switcher(move || {
            let Some(ui) = wk.upgrade() else { return };
            if !ui.get_signed_in() {
                return;
            }
            ui.set_switcher_text("".into());
            ui.invoke_switcher_edited("".into());
            ui.set_screen("switcher".into());
            let _ = &c;
        });
        let (wk, c) = (ui.as_weak(), client.clone());
        ui.on_switcher_edited(move |t| {
            let Some(ui) = wk.upgrade() else { return };
            let q = t.to_lowercase();
            let rows: Vec<RowItem> = c
                .sorted_conversations(false, false, false)
                .into_iter()
                .map(|cs| (c.conversation_title(&cs), cs.conversation.id))
                .filter(|(title, _)| q.is_empty() || title.to_lowercase().contains(&q))
                .take(8)
                .map(|(title, id)| row(3, &id, &id, title.clone(), title, String::new()))
                .collect();
            ui.set_switcher(model(rows));
        });
        let (wk, c) = (ui.as_weak(), client.clone());
        ui.on_open_threads(move || {
            let Some(ui) = wk.upgrade() else { return };
            let show = {
                let c = c.clone();
                move |ui: &AppWindow| {
                    let off = tz_offset();
                    let rows: Vec<RowItem> = c
                        .threads()
                        .into_iter()
                        .map(|t| {
                            let conv = c.conversations().into_iter().find(|x| x.conversation.id == t.conv_id).map(|cs| c.conversation_title(&cs)).unwrap_or_default();
                            let mut r = row(0, &t.root.id, &t.conv_id, format!("{conv}  |  {}", c.display_name(&t.root.sender_id)), model::plain(&t.root.body), format!("{} replies  |  last {}", t.reply_count, datefmt::format_date(t.last_reply_ts, off)));
                            r.badge = t.unread as i32;
                            r
                        })
                        .collect();
                    ui.set_threads(model(rows));
                }
            };
            show(&ui);
            ui.set_screen("threads".into());
            let (wk, c2) = (ui.as_weak(), c.clone());
            c.handle().spawn(async move {
                let _ = c2.refresh_threads().await;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = wk.upgrade() {
                        show(&ui);
                    }
                });
            });
        });
        let (wk, c) = (ui.as_weak(), client.clone());
        ui.on_open_people(move || {
            if let Some(ui) = wk.upgrade() {
                ui.set_people_text("".into());
                ui.set_people(model(people_list(&c, "")));
                ui.set_screen("people".into());
            }
        });
        let (wk, c) = (ui.as_weak(), client.clone());
        ui.on_people_filter(move |q| {
            if let Some(ui) = wk.upgrade() {
                ui.set_people(model(people_list(&c, q.as_str())));
            }
        });
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_create_channel(move |name| {
            let name = name.trim().to_string();
            if name.is_empty() {
                return;
            }
            let (wk2, c2, sh2) = (wk.clone(), c.clone(), sh.clone());
            c.handle().spawn(async move {
                let r = c2.create_channel(&name).await;
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = wk2.upgrade() else { return };
                    match r {
                        Ok(id) => {
                            ui.set_new_channel_open(false);
                            ui.set_screen("".into());
                            select_conv(&ui, &c2, &sh2, &id, true);
                        }
                        Err(e) => toast(&ui, &format!("Could not create the channel: {e}")),
                    }
                });
            });
        });
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_start_dm(move |uid| {
            let (wk, c2, sh2, uid) = (wk.clone(), c.clone(), sh.clone(), uid.to_string());
            c.handle().spawn(async move {
                if let Ok(conv) = c2.open_dm(&uid).await {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = wk.upgrade() {
                            select_conv(&ui, &c2, &sh2, &conv, true);
                        }
                    });
                }
            });
        });
    }

    // ---- organisation: favourites, mute, notify level, sections, unread ----------------------------------------
    {
        let (wk, c) = (ui.as_weak(), client.clone());
        let spawn_pref = move |f: Box<dyn FnOnce(Client) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send>| {
            let c = c.clone();
            let h = c.handle().clone();
            h.spawn(f(c));
        };
        let sp = Arc::new(spawn_pref);
        let (c, s) = (client.clone(), sp.clone());
        ui.on_toggle_fav(move |id| {
            let fav = !c.pref(&id).favourite;
            let id = id.to_string();
            s(Box::new(move |c| Box::pin(async move {
                let _ = c.set_favourite(&id, fav).await;
            })));
        });
        let s = sp.clone();
        ui.on_mute(move |id, choice| {
            let (id, choice) = (id.to_string(), choice.to_string());
            s(Box::new(move |c| Box::pin(async move {
                let _ = c.mute(&id, &choice, tz_offset()).await;
            })));
        });
        let s = sp.clone();
        ui.on_notify_level(move |id, lvl| {
            let (id, lvl) = (id.to_string(), lvl.to_string());
            s(Box::new(move |c| Box::pin(async move {
                let _ = c.set_pref(&id, &ConvPrefRequest { notify: Some(lvl), ..Default::default() }).await;
            })));
        });
        let s = sp.clone();
        ui.on_mark_unread(move |id| {
            let id = id.to_string();
            s(Box::new(move |c| Box::pin(async move {
                let _ = c.mark_unread(&id).await;
            })));
        });
        let s = sp.clone();
        ui.on_set_section(move |id, sid| {
            let (id, sid) = (id.to_string(), sid.to_string());
            s(Box::new(move |c| Box::pin(async move {
                let _ = c.set_pref(&id, &ConvPrefRequest { section_id: Some(sid), ..Default::default() }).await;
            })));
        });
        let s = sp.clone();
        let wk2 = wk.clone();
        let c2 = client.clone();
        ui.on_new_section(move |name| {
            let name = name.trim().to_string();
            if name.is_empty() {
                return;
            }
            let (wk3, c3) = (wk2.clone(), c2.clone());
            s(Box::new(move |c| Box::pin(async move {
                let _ = c.create_section(&name).await;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = wk3.upgrade() {
                        ui.set_sections(model(c3.sections().into_iter().map(|s| row(0, &s.id, "", s.name, String::new(), String::new())).collect()));
                    }
                });
            })));
        });
        // The sections list for the "Move to section" menu.
        let (wk, c) = (ui.as_weak(), client.clone());
        let t = slint::Timer::default();
        t.start(slint::TimerMode::Repeated, Duration::from_secs(2), move || {
            if let Some(ui) = wk.upgrade() {
                if ui.get_signed_in() && ui.get_menu_kind().as_str() == "conv" || ui.get_screen().as_str() == "settings" {
                    ui.set_sections(model(c.sections().into_iter().map(|s| row(0, &s.id, "", s.name, String::new(), String::new())).collect()));
                }
            }
        });
        std::mem::forget(t);
    }

    // ---- settings --------------------------------------------------------------------------------------------------
    {
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_open_settings(move || {
            let Some(ui) = wk.upgrade() else { return };
            if let Some(me) = c.me() {
                ui.set_edit_name(me.user.display_name.into());
            }
            ui.set_sections(model(c.sections().into_iter().map(|s| row(0, &s.id, "", s.name, String::new(), String::new())).collect()));
            ui.set_settings_page("".into());
            ui.set_screen("settings".into());
            load_sessions(&ui, &c);
            let _ = &sh;
        });
        let (wk, c, sh) = (ui.as_weak(), client.clone(), osh.clone());
        ui.on_setting_changed(move |kind| {
            let Some(ui) = wk.upgrade() else { return };
            let mut g = sh.lock().unwrap();
            let mut push_changed = false;
            match kind.as_str() {
                "theme" => g.prefs.theme = ["system", "light", "dark"][ui.get_theme_mode().clamp(0, 2) as usize].into(),
                "accent" => g.prefs.accent = ui.global::<Theme>().get_accent_index() as u8,
                "font" => g.prefs.font_scale = [100, 115, 130][ui.get_font_step().clamp(0, 2) as usize],
                "compact" => g.prefs.compact = ui.global::<Theme>().get_compact(),
                "motion" => g.prefs.reduce_motion = ui.global::<Theme>().get_reduce_motion(),
                "hour24" => g.prefs.hour24 = ui.get_hour24(),
                "notif" => {
                    g.prefs.notifications = ui.get_notif();
                    g.prefs.notification_preview = ui.get_notif_preview();
                }
                "push" => {
                    g.prefs.push_choice = ui.get_push_choice().to_string();
                    g.prefs.ntfy_url = ui.get_ntfy_url().to_string();
                    push_changed = true;
                }
                "tray" => g.prefs.close_to_tray = ui.get_close_tray(),
                "launch" => {
                    g.prefs.launch_on_login = ui.get_launch_login();
                    g.prefs.start_minimised = ui.get_start_min();
                    platform::set_autostart(g.prefs.launch_on_login, g.prefs.start_minimised);
                }
                "sort" => {
                    g.prefs.sort = if ui.get_sort_alpha() { "alpha".into() } else { "activity".into() };
                    g.prefs.unread_only = ui.get_unread_only();
                    g.prefs.hide_muted = ui.get_hide_muted();
                }
                "privacy" => {
                    let hide = ui.get_hide_last_active();
                    let c2 = c.clone();
                    c.handle().spawn(async move {
                        let _ = c2.set_hide_last_active(hide).await;
                    });
                }
                _ => {}
            }
            save_prefs(&g);
            apply_prefs(&ui, &g);
            drop(g);
            if push_changed {
                sync_push(&c, &sh);
            }
            refresh(&ui, &c, &sh);
        });
        let (wk, sh) = (ui.as_weak(), osh.clone());
        ui.on_zoom_step(move |d| {
            let Some(ui) = wk.upgrade() else { return };
            let mut g = sh.lock().unwrap();
            g.prefs.zoom = if d == 0 { 100 } else { (g.prefs.zoom as i32 + d).clamp(50, 300) as u16 };
            save_prefs(&g);
            ui.set_zoom(g.prefs.zoom as i32);
            ui.global::<Theme>().set_scale(scale_of(&g));
        });
        let (wk, c) = (ui.as_weak(), client.clone());
        ui.on_set_presence(move |pref| {
            let (wk, c2) = (wk.clone(), c.clone());
            let pref = pref.to_string();
            c.handle().spawn(async move {
                let r = c2.set_presence_pref(&pref).await;
                let _ = slint::invoke_from_event_loop(move || {
                    if let (Some(ui), Ok(me)) = (wk.upgrade(), r) {
                        ui.set_presence_pref(me.presence_pref.into());
                    }
                });
            });
        });
        let (wk, c) = (ui.as_weak(), client.clone());
        ui.on_save_profile(move |name| {
            let name = name.trim().to_string();
            if name.is_empty() {
                return;
            }
            let (wk, c2) = (wk.clone(), c.clone());
            c.handle().spawn(async move {
                let r = c2.update_profile(proto::types::UpdateMeRequest { display_name: Some(name), ..Default::default() }).await;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = wk.upgrade() {
                        toast(&ui, if r.is_ok() { "Profile saved." } else { "Could not save your profile." });
                    }
                });
            });
        });
        let (wk, c) = (ui.as_weak(), client.clone());
        ui.on_set_avatar(move |path| {
            let p = PathBuf::from(path.trim());
            let (wk, c2) = (wk.clone(), c.clone());
            c.handle().spawn(async move {
                let ok = p.is_file() && c2.set_avatar(&p).await.is_ok();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = wk.upgrade() {
                        toast(&ui, if ok { "Profile photo updated." } else { "Could not set that photo." });
                    }
                });
            });
        });
        let (wk, c) = (ui.as_weak(), client.clone());
        ui.on_revoke_session(move |id| {
            if !c.can_sign_out() {
                return;
            }
            let (wk, c2, id) = (wk.clone(), c.clone(), id.to_string());
            c.handle().spawn(async move {
                let _ = c2.revoke_session(&id).await;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = wk.upgrade() {
                        load_sessions(&ui, &c2);
                    }
                });
            });
        });
        let (wk, c) = (ui.as_weak(), client.clone());
        ui.on_revoke_others(move || {
            if !c.can_sign_out() {
                return;
            }
            let (wk, c2) = (wk.clone(), c.clone());
            c.handle().spawn(async move {
                let _ = c2.revoke_other_sessions().await;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = wk.upgrade() {
                        load_sessions(&ui, &c2);
                    }
                });
            });
        });
        let (wk, c) = (ui.as_weak(), client.clone());
        ui.on_sign_out(move || {
            if !c.can_sign_out() {
                return;
            }
            let (wk, c) = (wk.clone(), c.clone());
            c.clone().handle().spawn(async move {
                let r = c.logout().await;
                finish_sign_out(wk, r.is_ok());
            });
        });
        let (wk, c) = (ui.as_weak(), client.clone());
        ui.on_sign_out_all(move || {
            if !c.can_sign_out() {
                return;
            }
            let (wk, c) = (wk.clone(), c.clone());
            c.clone().handle().spawn(async move {
                let r = c.logout_all().await;
                finish_sign_out(wk, r.is_ok());
            });
        });
    }
    let _keep: Weak<AppWindow> = ui.as_weak();
    ui.run()
}
