//! X-04: local SQLite cache of conversations, messages, drafts and the send outbox.
//! The server is the source of truth; this is only a cache plus the not-yet-sent outbox.

use crate::error::{ClientError, Result};
use proto::*;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS kv(k TEXT PRIMARY KEY, v TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS users(id TEXT PRIMARY KEY, json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS convs(id TEXT PRIMARY KEY, json TEXT NOT NULL, sort_ts INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS messages(
  id TEXT PRIMARY KEY,
  conv_id TEXT NOT NULL,
  seq INTEGER NOT NULL,
  ts INTEGER NOT NULL,
  sender_id TEXT NOT NULL,
  client_id TEXT,
  body TEXT NOT NULL DEFAULT '',
  json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS messages_conv ON messages(conv_id, seq);
CREATE INDEX IF NOT EXISTS messages_client ON messages(client_id);
CREATE TABLE IF NOT EXISTS outbox(
  client_id TEXT PRIMARY KEY,
  conv_id TEXT NOT NULL,
  body TEXT NOT NULL,
  mentions TEXT NOT NULL DEFAULT '[]',
  thread_root TEXT,
  reply_to TEXT,
  files TEXT NOT NULL DEFAULT '[]',
  state TEXT NOT NULL DEFAULT 'pending',
  attempts INTEGER NOT NULL DEFAULT 0,
  last_error TEXT,
  created_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS drafts(conv_id TEXT PRIMARY KEY, body TEXT NOT NULL, updated_ms INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS thread_drafts(conv_id TEXT NOT NULL, thread_root TEXT NOT NULL, body TEXT NOT NULL, updated_ms INTEGER NOT NULL, PRIMARY KEY(conv_id, thread_root));
CREATE TABLE IF NOT EXISTS threads(root_id TEXT PRIMARY KEY, conv_id TEXT NOT NULL, json TEXT NOT NULL, sort_ts INTEGER NOT NULL DEFAULT 0);
CREATE VIRTUAL TABLE IF NOT EXISTS msg_fts USING fts5(body, content='messages', content_rowid='rowid');
CREATE TRIGGER IF NOT EXISTS msg_ai AFTER INSERT ON messages BEGIN
  INSERT INTO msg_fts(rowid, body) VALUES (new.rowid, new.body);
END;
CREATE TRIGGER IF NOT EXISTS msg_ad AFTER DELETE ON messages BEGIN
  INSERT INTO msg_fts(msg_fts, rowid, body) VALUES('delete', old.rowid, old.body);
END;
CREATE TRIGGER IF NOT EXISTS msg_au AFTER UPDATE OF body ON messages BEGIN
  INSERT INTO msg_fts(msg_fts, rowid, body) VALUES('delete', old.rowid, old.body);
  INSERT INTO msg_fts(rowid, body) VALUES (new.rowid, new.body);
END;
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SendState {
    Sending,
    Failed,
    Sent,
    Delivered,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimelineItem {
    pub message: Message,
    pub state: SendState,
    /// True for messages that exist only in the local outbox so far.
    pub local: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutFile {
    pub path: String,
    pub name: String,
    pub content_type: String,
    pub size: u64,
    #[serde(default)]
    pub uploaded_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OutboxItem {
    pub client_id: String,
    pub conv_id: String,
    pub body: String,
    pub mentions: Vec<String>,
    pub thread_root: Option<String>,
    pub reply_to: Option<String>,
    pub files: Vec<OutFile>,
    pub state: String,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub created_ms: i64,
}

/// What an applied event changed, so the UI and notifier know what to refresh.
#[derive(Debug, Default, Clone)]
pub struct Applied {
    pub conv_ids: Vec<String>,
    pub users_changed: bool,
    pub me_changed: bool,
    /// New messages from other people (for notifications).
    pub incoming: Vec<(Message, Option<ConversationSummary>)>,
    /// Conversations referenced by an event but not in the cache; fetch them.
    pub unknown_convs: Vec<String>,
    pub removed_convs: Vec<String>,
    pub attachment_removed: Vec<String>,
    /// Prefs, sections, groups, pins, bookmarks, threads or drafts changed.
    pub org_changed: bool,
    /// The server restored data for this account: discard the cache and bootstrap again.
    pub resync: bool,
    /// A mention, reaction or thread reply arrived: the Activity feed should be fetched again.
    pub activity_dirty: bool,
}

impl Applied {
    fn touch(&mut self, c: &str) {
        if !self.conv_ids.iter().any(|x| x == c) {
            self.conv_ids.push(c.to_string());
        }
    }
    pub fn merge(&mut self, o: Applied) {
        for c in o.conv_ids {
            self.touch(&c);
        }
        self.users_changed |= o.users_changed;
        self.me_changed |= o.me_changed;
        self.incoming.extend(o.incoming);
        self.unknown_convs.extend(o.unknown_convs);
        self.removed_convs.extend(o.removed_convs);
        self.attachment_removed.extend(o.attachment_removed);
        self.org_changed |= o.org_changed;
        self.resync |= o.resync;
        self.activity_dirty |= o.activity_dirty;
    }
}

pub struct Store {
    conn: Mutex<Connection>,
    path: PathBuf,
}

fn open_conn(path: &Path) -> Result<Connection> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let c = Connection::open(path)?;
    c.pragma_update(None, "journal_mode", "WAL")?;
    c.pragma_update(None, "synchronous", "NORMAL")?;
    c.busy_timeout(std::time::Duration::from_secs(5))?;
    c.execute_batch(SCHEMA)?;
    Ok(c)
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        Ok(Store { conn: Mutex::new(open_conn(path)?), path: path.to_path_buf() })
    }

    fn c(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// X-05: erase everything (cache, outbox, drafts, cursor) and start from an empty cache.
    pub fn wipe(&self) -> Result<()> {
        let mut g = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        // Replace the connection with an in-memory one so the files can be deleted.
        let old = std::mem::replace(&mut *g, Connection::open_in_memory()?);
        drop(old);
        for ext in ["", "-wal", "-shm"] {
            let p = PathBuf::from(format!("{}{}", self.path.display(), ext));
            let _ = std::fs::remove_file(p);
        }
        *g = open_conn(&self.path)?;
        Ok(())
    }

    pub fn kv_get(&self, k: &str) -> Result<Option<String>> {
        Ok(self.c().query_row("SELECT v FROM kv WHERE k=?1", [k], |r| r.get(0)).optional()?)
    }

    pub fn kv_set(&self, k: &str, v: &str) -> Result<()> {
        self.c().execute(
            "INSERT INTO kv(k,v) VALUES(?1,?2) ON CONFLICT(k) DO UPDATE SET v=excluded.v",
            params![k, v],
        )?;
        Ok(())
    }

    pub fn merge_scheduled(&self, item: &ScheduledMessage) -> Result<()> {
        merge_scheduled_tx(&self.c(), item)
    }

    pub fn put_message_public(&self, m: &Message) -> Result<()> {
        put_message(&self.c(), m)
    }

    pub fn kv_del(&self, k: &str) -> Result<()> {
        self.c().execute("DELETE FROM kv WHERE k=?1", [k])?;
        Ok(())
    }

    pub fn cursor(&self) -> i64 {
        self.kv_get("cursor").ok().flatten().and_then(|v| v.parse().ok()).unwrap_or(0)
    }

    pub fn has_snapshot(&self) -> bool {
        self.kv_get("bootstrapped").ok().flatten().is_some()
    }

    pub fn me(&self) -> Option<Me> {
        self.kv_get("me").ok().flatten().and_then(|s| serde_json::from_str(&s).ok())
    }

    pub fn set_me(&self, me: &Me) -> Result<()> {
        self.kv_set("me", &serde_json::to_string(me)?)
    }

    // ------------------------------------------------------------- snapshot

    /// Replaces the cached server state with a fresh snapshot (cursor expired / first login).
    /// The outbox and drafts survive.
    pub fn apply_bootstrap(&self, b: &Bootstrap) -> Result<()> {
        let mut g = self.c();
        let tx = g.transaction()?;
        tx.execute("DELETE FROM users", [])?;
        tx.execute("DELETE FROM convs", [])?;
        tx.execute("DELETE FROM messages", [])?;
        tx.execute("DELETE FROM threads", [])?;
        for k in ["prefs", "sections", "groups", "saved", "labels", "activity_read_ms"] {
            tx.execute("DELETE FROM kv WHERE k=?1", [k])?;
        }
        tx.execute("DELETE FROM kv WHERE k LIKE 'pins:%' OR k LIKE 'bookmarks:%'", [])?;
        kv_put(&tx, "prefs", &serde_json::to_string(&b.prefs)?)?;
        kv_put(&tx, "sections", &serde_json::to_string(&b.sections)?)?;
        kv_put(&tx, "groups", &serde_json::to_string(&b.groups)?)?;
        kv_put(&tx, "saved", &serde_json::to_string(&b.saved)?)?;
        kv_put(&tx, "labels", &serde_json::to_string(&b.labels)?)?;
        kv_put(&tx, "activity_read_ms", &b.activity_read_ms.to_string())?;
        for (conv, pins) in &b.pins {
            kv_put(&tx, &format!("pins:{conv}"), &serde_json::to_string(pins)?)?;
        }
        for (conv, bm) in &b.bookmarks {
            kv_put(&tx, &format!("bookmarks:{conv}"), &serde_json::to_string(bm)?)?;
        }
        for t in &b.threads {
            put_thread(&tx, t)?;
        }
        for d in &b.drafts {
            put_draft_item(&tx, d)?;
        }
        for u in &b.users {
            tx.execute("INSERT OR REPLACE INTO users(id,json) VALUES(?1,?2)", params![u.id, serde_json::to_string(u)?])?;
        }
        for cs in &b.conversations {
            put_conv(&tx, cs)?;
            tx.execute("DELETE FROM kv WHERE k=?1", [format!("more:{}", cs.conversation.id)])?;
        }
        for (conv, msgs) in &b.messages {
            for m in msgs {
                put_message(&tx, m)?;
            }
            let more = b.has_more.get(conv).copied().unwrap_or(false);
            tx.execute(
                "INSERT INTO kv(k,v) VALUES(?1,?2) ON CONFLICT(k) DO UPDATE SET v=excluded.v",
                params![format!("more:{conv}"), if more { "1" } else { "0" }],
            )?;
        }
        for (conv, more) in &b.has_more {
            if !b.messages.contains_key(conv) {
                tx.execute(
                    "INSERT INTO kv(k,v) VALUES(?1,?2) ON CONFLICT(k) DO UPDATE SET v=excluded.v",
                    params![format!("more:{conv}"), if *more { "1" } else { "0" }],
                )?;
                tx.execute("INSERT INTO kv(k,v) VALUES(?1,'1') ON CONFLICT(k) DO UPDATE SET v='1'", [format!("sparse:{conv}")])?;
            }
        }
        tx.execute("INSERT INTO kv(k,v) VALUES('me',?1) ON CONFLICT(k) DO UPDATE SET v=excluded.v", [serde_json::to_string(&b.me)?])?;
        tx.execute("INSERT INTO kv(k,v) VALUES('cursor',?1) ON CONFLICT(k) DO UPDATE SET v=excluded.v", [b.cursor.to_string()])?;
        tx.execute("INSERT INTO kv(k,v) VALUES('bootstrapped','1') ON CONFLICT(k) DO UPDATE SET v='1'", [])?;
        // Drop outbox rows whose message already arrived.
        tx.execute(
            "DELETE FROM outbox WHERE client_id IN (SELECT client_id FROM messages WHERE client_id IS NOT NULL)",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }

    // ------------------------------------------------------------- events

    /// Applies one event idempotently and advances the cursor. Events at or below the stored
    /// cursor are skipped (duplicates / replays).
    pub fn apply_event(&self, ev: &Event, my_id: &str) -> Result<Applied> {
        let mut g = self.c();
        let stored: i64 = g
            .query_row("SELECT v FROM kv WHERE k='cursor'", [], |r| r.get::<_, String>(0))
            .optional()?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut out = Applied::default();
        if !should_apply(stored, ev.cursor) {
            return Ok(out);
        }
        let tx = g.transaction()?;
        match &ev.event {
            EventData::MessageNew { message } => {
                let existed: bool = tx
                    .query_row("SELECT 1 FROM messages WHERE id=?1", [&message.id], |_| Ok(()))
                    .optional()?
                    .is_some();
                put_message(&tx, message)?;
                if let Some(cid) = &message.client_id {
                    tx.execute("DELETE FROM outbox WHERE client_id=?1", [cid])?;
                }
                out.touch(&message.conv_id);
                match get_conv(&tx, &message.conv_id)? {
                    Some(mut cs) => {
                        if message.seq > cs.conversation.last_seq {
                            cs.conversation.last_seq = message.seq;
                        }
                        if cs.conversation.last_message_ts.map(|t| message.ts > t).unwrap_or(true) {
                            cs.conversation.last_message_ts = Some(message.ts);
                        }
                        let mine = message.sender_id == my_id;
                        if !existed && !mine && message.seq > cs.last_read_seq && message.kind != MessageKind::System {
                            cs.unread += 1;
                            if message.mentions.iter().any(|m| m == my_id) {
                                cs.mentions += 1;
                            }
                        }
                        put_conv(&tx, &cs)?;
                        if !existed && !mine && message.kind != MessageKind::System {
                            out.incoming.push((message.clone(), Some(cs)));
                        }
                    }
                    None => {
                        out.unknown_convs.push(message.conv_id.clone());
                        if !existed && message.sender_id != my_id && message.kind != MessageKind::System {
                            out.incoming.push((message.clone(), None));
                        }
                    }
                }
            }
            EventData::MessageEdit { message } | EventData::MessageDelete { message } => {
                put_message(&tx, message)?;
                out.touch(&message.conv_id);
            }
            EventData::MessageHide { conv_id, message_ids } => {
                for id in message_ids {
                    tx.execute("DELETE FROM messages WHERE id=?1", [id])?;
                }
                out.touch(conv_id);
                // The visible tail may have changed.
                if let Some(mut cs) = get_conv(&tx, conv_id)? {
                    let last: Option<(i64, i64)> = tx
                        .query_row("SELECT seq, ts FROM messages WHERE conv_id=?1 ORDER BY seq DESC LIMIT 1", [conv_id], |r| Ok((r.get(0)?, r.get(1)?)))
                        .optional()?;
                    cs.conversation.last_message_ts = last.map(|l| l.1);
                    put_conv(&tx, &cs)?;
                }
            }
            EventData::ReactionUpdate { conv_id, message_id, reactions } => {
                if let Some(mut m) = get_message(&tx, message_id)? {
                    m.reactions = reactions.clone();
                    put_message(&tx, &m)?;
                }
                out.touch(conv_id);
            }
            EventData::MessageDelivered { conv_id, message_id } => {
                if let Some(mut m) = get_message(&tx, message_id)? {
                    m.delivered = true;
                    put_message(&tx, &m)?;
                }
                out.touch(conv_id);
            }
            EventData::ConversationUpsert { conversation } => {
                put_conv(&tx, conversation)?;
                out.touch(&conversation.conversation.id);
            }
            EventData::ConversationRemove { conv_id } => {
                tx.execute("DELETE FROM convs WHERE id=?1", [conv_id])?;
                tx.execute("DELETE FROM messages WHERE conv_id=?1", [conv_id])?;
                tx.execute("DELETE FROM drafts WHERE conv_id=?1", [conv_id])?;
                tx.execute("DELETE FROM thread_drafts WHERE conv_id=?1", [conv_id])?;
                tx.execute("DELETE FROM threads WHERE conv_id=?1", [conv_id])?;
                tx.execute("DELETE FROM outbox WHERE conv_id=?1", [conv_id])?;
                out.touch(conv_id);
                out.removed_convs.push(conv_id.clone());
            }
            EventData::ReadUpdate { conv_id, last_read_seq, marked_unread, unread, mentions } => {
                if let Some(mut cs) = get_conv(&tx, conv_id)? {
                    cs.last_read_seq = *last_read_seq;
                    cs.marked_unread = *marked_unread;
                    cs.unread = *unread;
                    cs.mentions = *mentions;
                    put_conv(&tx, &cs)?;
                }
                out.touch(conv_id);
            }
            EventData::UserUpsert { user } => {
                tx.execute("INSERT OR REPLACE INTO users(id,json) VALUES(?1,?2)", params![user.id, serde_json::to_string(user)?])?;
                out.users_changed = true;
            }
            EventData::AttachmentRemoved { attachment_id } => {
                let like = format!("%\"id\":\"{attachment_id}\"%");
                let ids: Vec<String> = {
                    let mut s = tx.prepare("SELECT id FROM messages WHERE json LIKE ?1")?;
                    let v = s.query_map([like], |r| r.get::<_, String>(0))?.collect::<std::result::Result<Vec<_>, _>>()?;
                    v
                };
                for id in ids {
                    if let Some(mut m) = get_message(&tx, &id)? {
                        for a in &mut m.attachments {
                            if a.id == *attachment_id {
                                a.removed = true;
                            }
                        }
                        out.touch(&m.conv_id);
                        put_message(&tx, &m)?;
                    }
                }
                out.attachment_removed.push(attachment_id.clone());
            }
            EventData::ThreadUpdate { thread } => {
                if thread.following {
                    put_thread(&tx, thread)?;
                } else {
                    tx.execute("DELETE FROM threads WHERE root_id=?1", [&thread.root.id])?;
                }
                out.org_changed = true;
            }
            EventData::PinsUpdate { conv_id, pins } => {
                kv_put(&tx, &format!("pins:{conv_id}"), &serde_json::to_string(pins)?)?;
                out.org_changed = true;
            }
            EventData::BookmarksUpdate { conv_id, bookmarks } => {
                kv_put(&tx, &format!("bookmarks:{conv_id}"), &serde_json::to_string(bookmarks)?)?;
                out.org_changed = true;
            }
            EventData::GroupsUpdate { groups } => {
                kv_put(&tx, "groups", &serde_json::to_string(groups)?)?;
                out.org_changed = true;
            }
            EventData::PrefUpdate { pref } => {
                let mut all: Vec<ConvPref> = kv_json(&tx, "prefs");
                all.retain(|p| p.conv_id != pref.conv_id);
                all.push(pref.clone());
                kv_put(&tx, "prefs", &serde_json::to_string(&all)?)?;
                out.org_changed = true;
            }
            EventData::SectionsUpdate { sections } => {
                kv_put(&tx, "sections", &serde_json::to_string(sections)?)?;
                out.org_changed = true;
            }
            EventData::DraftUpdate { draft } => {
                put_draft_item(&tx, draft)?;
                out.org_changed = true;
            }
            EventData::ScheduledUpdate { item } => {
                merge_scheduled_tx(&tx, item)?;
                out.org_changed = true;
            }
            EventData::SavedUpdate { item } => {
                let mut all: Vec<SavedItem> = kv_json(&tx, "saved");
                all.retain(|x| x.message_id != item.message_id);
                all.insert(0, item.clone());
                all.sort_by(|a, b| b.saved_ms.cmp(&a.saved_ms));
                kv_put(&tx, "saved", &serde_json::to_string(&all)?)?;
                out.org_changed = true;
            }
            EventData::SavedRemove { message_id } => {
                let mut all: Vec<SavedItem> = kv_json(&tx, "saved");
                all.retain(|x| x.message_id != *message_id);
                kv_put(&tx, "saved", &serde_json::to_string(&all)?)?;
                out.org_changed = true;
            }
            EventData::LabelsUpdate { labels } => {
                kv_put(&tx, "labels", &serde_json::to_string(labels)?)?;
                out.org_changed = true;
            }
            EventData::ActivityRead { read_ms } => {
                kv_put(&tx, "activity_read_ms", &read_ms.to_string())?;
                out.org_changed = true;
            }
            EventData::Resync {} => {
                out.resync = true;
            }
            EventData::MeUpdate { me } => {
                tx.execute("INSERT INTO kv(k,v) VALUES('me',?1) ON CONFLICT(k) DO UPDATE SET v=excluded.v", [serde_json::to_string(me)?])?;
                tx.execute("INSERT OR REPLACE INTO users(id,json) VALUES(?1,?2)", params![me.user.id, serde_json::to_string(&me.user)?])?;
                out.me_changed = true;
                out.users_changed = true;
            }
        }
        match &ev.event {
            EventData::MessageNew { message } if message.sender_id != my_id && (message.mentions.iter().any(|m| m == my_id) || message.thread_root.is_some()) => out.activity_dirty = true,
            EventData::ReactionUpdate { .. } | EventData::ThreadUpdate { .. } => out.activity_dirty = true,
            _ => {}
        }
        tx.execute("INSERT INTO kv(k,v) VALUES('cursor',?1) ON CONFLICT(k) DO UPDATE SET v=excluded.v", [ev.cursor.to_string()])?;
        tx.commit()?;
        Ok(out)
    }

    // ------------------------------------------------------------- reads

    pub fn conversations(&self) -> Result<Vec<ConversationSummary>> {
        let g = self.c();
        let mut s = g.prepare("SELECT json FROM convs ORDER BY sort_ts DESC")?;
        let rows = s.query_map([], |r| r.get::<_, String>(0))?.collect::<std::result::Result<Vec<_>, _>>()?;
        rows.iter().map(|j| serde_json::from_str(j).map_err(Into::into)).collect()
    }

    pub fn conversation(&self, id: &str) -> Result<Option<ConversationSummary>> {
        get_conv(&self.c(), id)
    }

    pub fn put_conversation(&self, cs: &ConversationSummary) -> Result<()> {
        put_conv(&self.c(), cs)
    }

    pub fn users(&self) -> Result<Vec<User>> {
        let g = self.c();
        let mut s = g.prepare("SELECT json FROM users")?;
        let rows = s.query_map([], |r| r.get::<_, String>(0))?.collect::<std::result::Result<Vec<_>, _>>()?;
        let mut v: Vec<User> = rows.iter().filter_map(|j| serde_json::from_str(j).ok()).collect();
        v.sort_by_key(|u| u.display_name.to_lowercase());
        Ok(v)
    }

    pub fn user(&self, id: &str) -> Result<Option<User>> {
        Ok(self
            .c()
            .query_row("SELECT json FROM users WHERE id=?1", [id], |r| r.get::<_, String>(0))
            .optional()?
            .and_then(|j| serde_json::from_str(&j).ok()))
    }

    pub fn put_users(&self, users: &[User]) -> Result<()> {
        let g = self.c();
        for u in users {
            g.execute("INSERT OR REPLACE INTO users(id,json) VALUES(?1,?2)", params![u.id, serde_json::to_string(u)?])?;
        }
        Ok(())
    }

    pub fn message(&self, id: &str) -> Result<Option<Message>> {
        get_message(&self.c(), id)
    }

    pub fn put_messages(&self, msgs: &[Message]) -> Result<()> {
        let mut g = self.c();
        let tx = g.transaction()?;
        for m in msgs {
            put_message(&tx, m)?;
            if let Some(cid) = &m.client_id {
                tx.execute("DELETE FROM outbox WHERE client_id=?1", [cid])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Latest `limit` messages (older than `before_seq` if given), ascending, with the outbox
    /// appended when showing the tail.
    pub fn timeline(&self, conv_id: &str, limit: usize, before_seq: Option<i64>, my_id: &str) -> Result<Vec<TimelineItem>> {
        let g = self.c();
        let rows: Vec<String> = {
            let mut s = g.prepare(
                "SELECT json FROM messages WHERE conv_id=?1 AND seq<?2 AND json_extract(json,'$.thread_root') IS NULL ORDER BY seq DESC LIMIT ?3",
            )?;
            let v = s
                .query_map(params![conv_id, before_seq.unwrap_or(i64::MAX), limit as i64], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            v
        };
        let mut items: Vec<TimelineItem> = rows
            .iter()
            .rev()
            .filter_map(|j| serde_json::from_str::<Message>(j).ok())
            .map(|m| {
                let state = if m.sender_id == my_id && m.delivered { SendState::Delivered } else { SendState::Sent };
                TimelineItem { message: m, state, local: false, error: None }
            })
            .collect();
        if before_seq.is_none() {
            let mut s = g.prepare("SELECT client_id,conv_id,body,mentions,thread_root,reply_to,files,state,attempts,last_error,created_ms FROM outbox WHERE conv_id=?1 ORDER BY created_ms")?;
            let ob = s
                .query_map([conv_id], outbox_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            for (i, o) in ob.into_iter().filter(|o| o.thread_root.is_none()).enumerate() {
                let failed = o.state == "failed";
                items.push(TimelineItem {
                    message: outbox_message(&o, my_id, i as i64),
                    state: if failed { SendState::Failed } else { SendState::Sending },
                    local: true,
                    error: o.last_error.clone(),
                });
            }
        }
        Ok(items)
    }

    pub fn oldest_seq(&self, conv_id: &str) -> Option<i64> {
        self.c()
            .query_row("SELECT MIN(seq) FROM messages WHERE conv_id=?1", [conv_id], |r| r.get::<_, Option<i64>>(0))
            .ok()
            .flatten()
    }

    pub fn newest_seq(&self, conv_id: &str) -> Option<i64> {
        self.c()
            .query_row("SELECT MAX(seq) FROM messages WHERE conv_id=?1", [conv_id], |r| r.get::<_, Option<i64>>(0))
            .ok()
            .flatten()
    }

    pub fn has_more_before(&self, conv_id: &str) -> bool {
        self.kv_get(&format!("more:{conv_id}")).ok().flatten().map(|v| v == "1").unwrap_or(false)
    }

    pub fn set_has_more_before(&self, conv_id: &str, more: bool) -> Result<()> {
        self.kv_set(&format!("more:{conv_id}"), if more { "1" } else { "0" })
    }

    /// True when someone other than `author` has a cached message after `seq` (edit rule mirror).
    pub fn other_posted_after(&self, conv_id: &str, seq: i64, author: &str) -> bool {
        self.c()
            .query_row(
                "SELECT 1 FROM messages WHERE conv_id=?1 AND seq>?2 AND sender_id!=?3 LIMIT 1",
                params![conv_id, seq, author],
                |_| Ok(()),
            )
            .optional()
            .ok()
            .flatten()
            .is_some()
    }

    pub fn unread_total(&self) -> i64 {
        self.conversations().map(|v| v.iter().map(|c| c.unread).sum()).unwrap_or(0)
    }

    /// Instant offline search over the cached messages (SR-01), newest first.
    pub fn search_local(&self, q: &str, limit: usize) -> Result<Vec<Message>> {
        let words: Vec<String> = q
            .split_whitespace()
            .map(|w| w.chars().filter(|c| c.is_alphanumeric() || *c == '_' || *c == '\'').collect::<String>())
            .filter(|w| !w.is_empty())
            .collect();
        if words.is_empty() {
            return Ok(vec![]);
        }
        let n = words.len();
        let fts = words
            .iter()
            .enumerate()
            .map(|(i, w)| if i + 1 == n { format!("\"{w}\"*") } else { format!("\"{w}\"") })
            .collect::<Vec<_>>()
            .join(" ");
        let g = self.c();
        let mut s = g.prepare(
            "SELECT m.json FROM msg_fts f JOIN messages m ON m.rowid=f.rowid WHERE msg_fts MATCH ?1 ORDER BY m.ts DESC LIMIT ?2",
        )?;
        let rows = s
            .query_map(params![fts, limit as i64], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows.iter().filter_map(|j| serde_json::from_str::<Message>(j).ok()).filter(|m| !m.deleted).collect())
    }

    // ------------------------------------------------------------- organisation

    pub fn prefs(&self) -> Vec<ConvPref> {
        kv_json(&self.c(), "prefs")
    }

    pub fn pref(&self, conv_id: &str) -> Option<ConvPref> {
        self.prefs().into_iter().find(|p| p.conv_id == conv_id)
    }

    pub fn sections(&self) -> Vec<Section> {
        let mut v: Vec<Section> = kv_json(&self.c(), "sections");
        v.sort_by_key(|s| s.position);
        v
    }

    pub fn saved(&self) -> Vec<SavedItem> {
        kv_json(&self.c(), "saved")
    }

    pub fn put_saved(&self, v: &[SavedItem]) -> Result<()> {
        kv_put(&self.c(), "saved", &serde_json::to_string(v)?)
    }

    pub fn labels(&self) -> Vec<Label> {
        let mut v: Vec<Label> = kv_json(&self.c(), "labels");
        v.sort_by_key(|l| l.position);
        v
    }

    pub fn put_labels(&self, v: &[Label]) -> Result<()> {
        kv_put(&self.c(), "labels", &serde_json::to_string(v)?)
    }

    pub fn activity_read_ms(&self) -> i64 {
        self.kv_get("activity_read_ms").ok().flatten().and_then(|v| v.parse().ok()).unwrap_or(0)
    }

    pub fn set_activity_read_ms(&self, ms: i64) -> Result<()> {
        self.kv_set("activity_read_ms", &ms.to_string())
    }

    pub fn activity(&self) -> Vec<ActivityItem> {
        kv_json(&self.c(), "activity")
    }

    pub fn put_activity(&self, v: &[ActivityItem]) -> Result<()> {
        kv_put(&self.c(), "activity", &serde_json::to_string(v)?)
    }

    pub fn groups(&self) -> Vec<UserGroup> {
        kv_json(&self.c(), "groups")
    }

    pub fn pins(&self, conv_id: &str) -> Vec<Pin> {
        kv_json(&self.c(), &format!("pins:{conv_id}"))
    }

    pub fn bookmarks(&self, conv_id: &str) -> Vec<Bookmark> {
        kv_json(&self.c(), &format!("bookmarks:{conv_id}"))
    }

    pub fn put_pins(&self, conv_id: &str, pins: &[Pin]) -> Result<()> {
        kv_put(&self.c(), &format!("pins:{conv_id}"), &serde_json::to_string(pins)?)
    }

    pub fn put_bookmarks(&self, conv_id: &str, b: &[Bookmark]) -> Result<()> {
        kv_put(&self.c(), &format!("bookmarks:{conv_id}"), &serde_json::to_string(b)?)
    }

    pub fn put_prefs(&self, prefs: &[ConvPref]) -> Result<()> {
        kv_put(&self.c(), "prefs", &serde_json::to_string(prefs)?)
    }

    pub fn put_sections(&self, s: &[Section]) -> Result<()> {
        kv_put(&self.c(), "sections", &serde_json::to_string(s)?)
    }

    pub fn threads(&self) -> Vec<ThreadSummary> {
        let g = self.c();
        let Ok(mut s) = g.prepare("SELECT json FROM threads ORDER BY sort_ts DESC") else { return vec![] };
        let rows = s.query_map([], |r| r.get::<_, String>(0)).map(|it| it.filter_map(|r| r.ok()).collect::<Vec<_>>()).unwrap_or_default();
        rows.iter().filter_map(|j| serde_json::from_str(j).ok()).collect()
    }

    pub fn thread(&self, root_id: &str) -> Option<ThreadSummary> {
        self.c().query_row("SELECT json FROM threads WHERE root_id=?1", [root_id], |r| r.get::<_, String>(0)).optional().ok().flatten().and_then(|j| serde_json::from_str(&j).ok())
    }

    pub fn put_thread(&self, t: &ThreadSummary) -> Result<()> {
        if t.following {
            put_thread(&self.c(), t)
        } else {
            self.c().execute("DELETE FROM threads WHERE root_id=?1", [&t.root.id])?;
            Ok(())
        }
    }

    /// Cached replies of a thread, ascending, with unsent outbox replies appended.
    pub fn thread_replies(&self, conv_id: &str, root_id: &str, my_id: &str) -> Result<Vec<TimelineItem>> {
        let g = self.c();
        let rows: Vec<String> = {
            let mut s = g.prepare("SELECT json FROM messages WHERE conv_id=?1 AND json_extract(json,'$.thread_root')=?2 ORDER BY seq")?;
            let v = s.query_map(params![conv_id, root_id], |r| r.get::<_, String>(0))?.collect::<std::result::Result<Vec<_>, _>>()?;
            v
        };
        let mut items: Vec<TimelineItem> = rows
            .iter()
            .filter_map(|j| serde_json::from_str::<Message>(j).ok())
            .map(|m| {
                let state = if m.sender_id == my_id && m.delivered { SendState::Delivered } else { SendState::Sent };
                TimelineItem { message: m, state, local: false, error: None }
            })
            .collect();
        let mut s = g.prepare("SELECT client_id,conv_id,body,mentions,thread_root,reply_to,files,state,attempts,last_error,created_ms FROM outbox WHERE conv_id=?1 AND thread_root=?2 ORDER BY created_ms")?;
        let ob = s.query_map(params![conv_id, root_id], outbox_row)?.collect::<std::result::Result<Vec<_>, _>>()?;
        for (i, o) in ob.into_iter().enumerate() {
            let failed = o.state == "failed";
            items.push(TimelineItem { message: outbox_message(&o, my_id, i as i64), state: if failed { SendState::Failed } else { SendState::Sending }, local: true, error: o.last_error.clone() });
        }
        Ok(items)
    }

    /// Number of cached replies per thread root in a conversation (for "N replies" badges).
    pub fn reply_counts(&self, conv_id: &str) -> std::collections::HashMap<String, i64> {
        let g = self.c();
        let mut out = std::collections::HashMap::new();
        if let Ok(mut s) = g.prepare("SELECT json_extract(json,'$.thread_root'), COUNT(*) FROM messages WHERE conv_id=?1 AND json_extract(json,'$.thread_root') IS NOT NULL AND json_extract(json,'$.deleted')=0 GROUP BY 1") {
            if let Ok(rows) = s.query_map([conv_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))) {
                for r in rows.flatten() {
                    out.insert(r.0, r.1);
                }
            }
        }
        out
    }

    /// Cached messages with a file attached, newest first (Files tab, offline).
    pub fn cached_files(&self, conv_id: Option<&str>, limit: usize) -> Vec<(Message, Attachment)> {
        let g = self.c();
        let sql = "SELECT json FROM messages WHERE (?1 IS NULL OR conv_id=?1) AND json_array_length(json,'$.attachments')>0 AND json_extract(json,'$.deleted')=0 ORDER BY ts DESC LIMIT ?2";
        let Ok(mut s) = g.prepare(sql) else { return vec![] };
        let rows = s.query_map(params![conv_id, limit as i64], |r| r.get::<_, String>(0)).map(|it| it.filter_map(|r| r.ok()).collect::<Vec<_>>()).unwrap_or_default();
        let mut out = vec![];
        for j in rows {
            if let Ok(m) = serde_json::from_str::<Message>(&j) {
                for a in m.attachments.clone() {
                    out.push((m.clone(), a));
                }
            }
        }
        out
    }

    /// All drafts (conversation and thread drafts), newest first (M-23).
    pub fn all_drafts(&self) -> Vec<DraftItem> {
        let g = self.c();
        let mut out = vec![];
        if let Ok(mut s) = g.prepare("SELECT conv_id,body,updated_ms FROM drafts") {
            if let Ok(rows) = s.query_map([], |r| Ok(DraftItem { conv_id: r.get(0)?, thread_root: None, body: r.get(1)?, updated_ms: r.get(2)? })) {
                out.extend(rows.flatten());
            }
        }
        if let Ok(mut s) = g.prepare("SELECT conv_id,thread_root,body,updated_ms FROM thread_drafts") {
            if let Ok(rows) = s.query_map([], |r| Ok(DraftItem { conv_id: r.get(0)?, thread_root: Some(r.get(1)?), body: r.get(2)?, updated_ms: r.get(3)? })) {
                out.extend(rows.flatten());
            }
        }
        out.sort_by_key(|d| std::cmp::Reverse(d.updated_ms));
        out
    }

    pub fn set_thread_draft(&self, conv_id: &str, thread_root: &str, body: &str, now: i64) -> Result<()> {
        put_draft_item(&self.c(), &DraftItem { conv_id: conv_id.to_string(), thread_root: Some(thread_root.to_string()), body: body.to_string(), updated_ms: now })
    }

    pub fn thread_draft(&self, conv_id: &str, thread_root: &str) -> Option<String> {
        self.c().query_row("SELECT body FROM thread_drafts WHERE conv_id=?1 AND thread_root=?2", params![conv_id, thread_root], |r| r.get(0)).optional().ok().flatten()
    }

    /// Cached message around a sequence number (permalink jump) - ascending.
    pub fn messages_around(&self, conv_id: &str, seq: i64, radius: i64) -> Vec<Message> {
        let g = self.c();
        let Ok(mut s) = g.prepare("SELECT json FROM messages WHERE conv_id=?1 AND seq BETWEEN ?2 AND ?3 ORDER BY seq") else { return vec![] };
        let rows = s.query_map(params![conv_id, seq - radius, seq + radius], |r| r.get::<_, String>(0)).map(|it| it.filter_map(|r| r.ok()).collect::<Vec<_>>()).unwrap_or_default();
        rows.iter().filter_map(|j| serde_json::from_str(j).ok()).collect()
    }

    // ------------------------------------------------------------- drafts

    pub fn set_draft(&self, conv_id: &str, body: &str, now: i64) -> Result<()> {
        let g = self.c();
        if body.is_empty() {
            g.execute("DELETE FROM drafts WHERE conv_id=?1", [conv_id])?;
        } else {
            g.execute(
                "INSERT INTO drafts(conv_id,body,updated_ms) VALUES(?1,?2,?3) ON CONFLICT(conv_id) DO UPDATE SET body=excluded.body, updated_ms=excluded.updated_ms",
                params![conv_id, body, now],
            )?;
        }
        Ok(())
    }

    pub fn draft(&self, conv_id: &str) -> Option<String> {
        self.c().query_row("SELECT body FROM drafts WHERE conv_id=?1", [conv_id], |r| r.get(0)).optional().ok().flatten()
    }

    // ------------------------------------------------------------- outbox

    pub fn outbox_add(&self, o: &OutboxItem) -> Result<()> {
        self.c().execute(
            "INSERT INTO outbox(client_id,conv_id,body,mentions,thread_root,reply_to,files,state,attempts,last_error,created_ms)
             VALUES(?1,?2,?3,?4,?5,?6,?7,'pending',0,NULL,?8)",
            params![o.client_id, o.conv_id, o.body, serde_json::to_string(&o.mentions)?, o.thread_root, o.reply_to, serde_json::to_string(&o.files)?, o.created_ms],
        )?;
        Ok(())
    }

    /// Pending (not failed) outbox items in send order.
    pub fn outbox_pending(&self) -> Result<Vec<OutboxItem>> {
        let g = self.c();
        let mut s = g.prepare("SELECT client_id,conv_id,body,mentions,thread_root,reply_to,files,state,attempts,last_error,created_ms FROM outbox WHERE state='pending' ORDER BY created_ms, rowid")?;
        let v = s.query_map([], outbox_row)?.collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(v)
    }

    pub fn outbox_all(&self) -> Result<Vec<OutboxItem>> {
        let g = self.c();
        let mut s = g.prepare("SELECT client_id,conv_id,body,mentions,thread_root,reply_to,files,state,attempts,last_error,created_ms FROM outbox ORDER BY created_ms, rowid")?;
        let v = s.query_map([], outbox_row)?.collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(v)
    }

    pub fn outbox_update_files(&self, client_id: &str, files: &[OutFile]) -> Result<()> {
        self.c().execute("UPDATE outbox SET files=?2 WHERE client_id=?1", params![client_id, serde_json::to_string(files)?])?;
        Ok(())
    }

    pub fn outbox_attempt(&self, client_id: &str, err: Option<&str>) -> Result<()> {
        self.c().execute(
            "UPDATE outbox SET attempts=attempts+1, last_error=?2 WHERE client_id=?1",
            params![client_id, err],
        )?;
        Ok(())
    }

    pub fn outbox_fail(&self, client_id: &str, err: &str) -> Result<()> {
        self.c().execute("UPDATE outbox SET state='failed', last_error=?2 WHERE client_id=?1", params![client_id, err])?;
        Ok(())
    }

    pub fn outbox_retry(&self, client_id: &str) -> Result<()> {
        self.c().execute("UPDATE outbox SET state='pending', last_error=NULL WHERE client_id=?1", [client_id])?;
        Ok(())
    }

    pub fn outbox_remove(&self, client_id: &str) -> Result<()> {
        self.c().execute("DELETE FROM outbox WHERE client_id=?1", [client_id])?;
        Ok(())
    }
}

fn outbox_row(r: &rusqlite::Row) -> rusqlite::Result<OutboxItem> {
    Ok(OutboxItem {
        client_id: r.get(0)?,
        conv_id: r.get(1)?,
        body: r.get(2)?,
        mentions: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_default(),
        thread_root: r.get(4)?,
        reply_to: r.get(5)?,
        files: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default(),
        state: r.get(7)?,
        attempts: r.get(8)?,
        last_error: r.get(9)?,
        created_ms: r.get(10)?,
    })
}

fn outbox_message(o: &OutboxItem, my_id: &str, idx: i64) -> Message {
    Message {
        id: format!("local:{}", o.client_id),
        conv_id: o.conv_id.clone(),
        seq: i64::MAX / 2 + idx,
        sender_id: my_id.to_string(),
        client_id: Some(o.client_id.clone()),
        body: o.body.clone(),
        kind: if o.body.is_empty() { MessageKind::File } else { MessageKind::Text },
        ts: o.created_ms,
        version: 1,
        edited_ms: None,
        deleted: false,
        thread_root: o.thread_root.clone(),
        reply_to: o.reply_to.clone(),
        mentions: o.mentions.clone(),
        attachments: o
            .files
            .iter()
            .map(|f| Attachment { id: format!("local:{}", f.name), name: f.name.clone(), content_type: f.content_type.clone(), size: f.size as i64, removed: false })
            .collect(),
        reactions: vec![],
        delivered: false,
        forwarded: None,
        form: None,
        responded: vec![],
    }
}

fn put_conv(c: &Connection, cs: &ConversationSummary) -> Result<()> {
    let sort = cs.conversation.last_message_ts.unwrap_or(cs.conversation.created_ms);
    c.execute(
        "INSERT INTO convs(id,json,sort_ts) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET json=excluded.json, sort_ts=excluded.sort_ts",
        params![cs.conversation.id, serde_json::to_string(cs)?, sort],
    )?;
    Ok(())
}

fn get_conv(c: &Connection, id: &str) -> Result<Option<ConversationSummary>> {
    c.query_row("SELECT json FROM convs WHERE id=?1", [id], |r| r.get::<_, String>(0))
        .optional()?
        .map(|j| serde_json::from_str(&j).map_err(ClientError::from))
        .transpose()
}

fn put_message(c: &Connection, m: &Message) -> Result<()> {
    c.execute(
        "INSERT INTO messages(id,conv_id,seq,ts,sender_id,client_id,body,json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
         ON CONFLICT(id) DO UPDATE SET seq=excluded.seq, ts=excluded.ts, body=excluded.body, json=excluded.json, client_id=excluded.client_id",
        params![m.id, m.conv_id, m.seq, m.ts, m.sender_id, m.client_id, m.body, serde_json::to_string(m)?],
    )?;
    Ok(())
}

fn get_message(c: &Connection, id: &str) -> Result<Option<Message>> {
    c.query_row("SELECT json FROM messages WHERE id=?1", [id], |r| r.get::<_, String>(0))
        .optional()?
        .map(|j| serde_json::from_str(&j).map_err(ClientError::from))
        .transpose()
}

fn merge_scheduled_tx(c: &Connection, item: &ScheduledMessage) -> Result<()> {
    let mut all: Vec<ScheduledMessage> = kv_json(c, "scheduled");
    all.retain(|m| m.id != item.id);
    all.push(item.clone());
    // Keep the list small: pending plus the 50 most recent finished items.
    let mut done: Vec<ScheduledMessage> = all.iter().filter(|m| m.state != ScheduledState::Pending).cloned().collect();
    done.sort_by_key(|m| std::cmp::Reverse(m.due_ms));
    done.truncate(50);
    all.retain(|m| m.state == ScheduledState::Pending);
    all.extend(done);
    kv_put(c, "scheduled", &serde_json::to_string(&all)?)
}

fn kv_put(c: &Connection, k: &str, v: &str) -> Result<()> {
    c.execute("INSERT INTO kv(k,v) VALUES(?1,?2) ON CONFLICT(k) DO UPDATE SET v=excluded.v", params![k, v])?;
    Ok(())
}

fn kv_json<T: serde::de::DeserializeOwned + Default>(c: &Connection, k: &str) -> T {
    c.query_row("SELECT v FROM kv WHERE k=?1", [k], |r| r.get::<_, String>(0)).optional().ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn put_thread(c: &Connection, t: &ThreadSummary) -> Result<()> {
    c.execute(
        "INSERT INTO threads(root_id,conv_id,json,sort_ts) VALUES(?1,?2,?3,?4) ON CONFLICT(root_id) DO UPDATE SET json=excluded.json, sort_ts=excluded.sort_ts",
        params![t.root.id, t.conv_id, serde_json::to_string(t)?, t.last_reply_ts.max(t.root.ts)],
    )?;
    Ok(())
}

/// Stores a server draft unless the local one is newer.
fn put_draft_item(c: &Connection, d: &DraftItem) -> Result<()> {
    match &d.thread_root {
        None => {
            let local: Option<i64> = c.query_row("SELECT updated_ms FROM drafts WHERE conv_id=?1", [&d.conv_id], |r| r.get(0)).optional()?;
            if local.map(|l| l > d.updated_ms).unwrap_or(false) {
                return Ok(());
            }
            if d.body.is_empty() {
                c.execute("DELETE FROM drafts WHERE conv_id=?1", [&d.conv_id])?;
            } else {
                c.execute(
                    "INSERT INTO drafts(conv_id,body,updated_ms) VALUES(?1,?2,?3) ON CONFLICT(conv_id) DO UPDATE SET body=excluded.body, updated_ms=excluded.updated_ms",
                    params![d.conv_id, d.body, d.updated_ms],
                )?;
            }
        }
        Some(tr) => {
            if d.body.is_empty() {
                c.execute("DELETE FROM thread_drafts WHERE conv_id=?1 AND thread_root=?2", params![d.conv_id, tr])?;
            } else {
                c.execute(
                    "INSERT INTO thread_drafts(conv_id,thread_root,body,updated_ms) VALUES(?1,?2,?3,?4) ON CONFLICT(conv_id,thread_root) DO UPDATE SET body=excluded.body, updated_ms=excluded.updated_ms",
                    params![d.conv_id, tr, d.body, d.updated_ms],
                )?;
            }
        }
    }
    Ok(())
}
