//! Phase 2 client API: search filters, threads, pins, bookmarks, preferences, sections, drafts sync,
//! forwarding, permalinks, files browser, push registration.

use super::*;
use crate::logic::searchq::SearchFilters;
use proto::rules;

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

impl Client {
    pub(super) fn changed(&self, conv_ids: Vec<String>) {
        self.inner.emit(ClientEvent::Changed { conv_ids, users: false, me: false });
    }

    // ------------------------------------------------------------------ search (SV-08, SR-*)

    /// Server-side search with filters. `in_conv` may be a conversation id or a channel name.
    pub async fn search_filtered(&self, f: &SearchFilters) -> Result<SearchResponse> {
        let mut q = vec![format!("q={}", enc(&f.text))];
        if let Some(v) = &f.from {
            q.push(format!("from={}", enc(v)));
        }
        if let Some(v) = &f.in_conv {
            let id = self.resolve_conversation(v).unwrap_or_else(|| v.clone());
            q.push(format!("in={}", enc(&id)));
        }
        if let Some(v) = f.before_ms {
            q.push(format!("before={v}"));
        }
        if let Some(v) = f.after_ms {
            q.push(format!("after={v}"));
        }
        if !f.has.is_empty() {
            q.push(format!("has={}", f.has.join(",")));
        }
        if let Some(k) = &f.kind {
            q.push(format!("type={k}"));
        }
        let r: SearchResponse = self.api()?.get(&format!("/search?{}", q.join("&"))).await?;
        Ok(self.without_hidden_hits(r))
    }

    /// Finds a conversation id by id or case-insensitive name.
    pub fn resolve_conversation(&self, name_or_id: &str) -> Option<String> {
        let n = name_or_id.trim().trim_start_matches('#');
        self.conversations().into_iter().find_map(|c| {
            if c.conversation.id == n || c.conversation.name.as_deref().map(|x| x.eq_ignore_ascii_case(n)).unwrap_or(false) || self.conversation_title(&c).eq_ignore_ascii_case(n) {
                Some(c.conversation.id)
            } else {
                None
            }
        })
    }

    /// Files tab (SR-03): server list, newest first.
    pub async fn list_files(&self, conv_id: Option<&str>, kind: Option<&str>, q: Option<&str>, before: Option<i64>) -> Result<Vec<FileHit>> {
        let mut p = vec![];
        if let Some(c) = conv_id {
            p.push(format!("conv={}", enc(c)));
        }
        if let Some(k) = kind {
            p.push(format!("type={k}"));
        }
        if let Some(t) = q {
            p.push(format!("q={}", enc(t)));
        }
        if let Some(b) = before {
            p.push(format!("before={b}"));
        }
        let mut v: Vec<FileHit> = self.api()?.get(&format!("/files?{}", p.join("&"))).await?;
        v.retain(|f| !self.is_hidden(&f.conv_id));
        Ok(v)
    }

    // ------------------------------------------------------------------ threads (M-05..07)

    pub fn threads(&self) -> Vec<ThreadSummary> {
        let mut v = self.inner.store.threads();
        v.retain(|t| !self.is_hidden(&t.conv_id));
        v
    }

    pub fn thread_unread_total(&self) -> i64 {
        self.threads().iter().map(|t| t.unread).sum()
    }

    pub fn thread_replies(&self, conv_id: &str, root_id: &str) -> Vec<TimelineItem> {
        if self.is_hidden(conv_id) {
            return vec![];
        }
        let me = self.my_id().unwrap_or_default();
        self.inner.store.thread_replies(conv_id, root_id, &me).unwrap_or_default()
    }

    pub fn reply_counts(&self, conv_id: &str) -> std::collections::HashMap<String, i64> {
        self.inner.store.reply_counts(conv_id)
    }

    /// Opens a thread: fetches root and replies and caches them.
    pub async fn open_thread(&self, conv_id: &str, root_id: &str) -> Result<ThreadView> {
        let v: ThreadView = self.api()?.get(&format!("/conversations/{conv_id}/threads/{root_id}")).await?;
        let mut msgs = vec![v.summary.root.clone()];
        msgs.extend(v.replies.clone());
        self.inner.store.put_messages(&msgs)?;
        self.changed(vec![conv_id.to_string()]);
        Ok(v)
    }

    /// Refreshes the Threads list from the server.
    pub async fn refresh_threads(&self) -> Result<Vec<ThreadSummary>> {
        let v: Vec<ThreadSummary> = self.api()?.get("/threads").await?;
        for t in &v {
            self.inner.store.put_thread(t)?;
        }
        self.changed(vec![]);
        Ok(v)
    }

    pub async fn follow_thread(&self, root_id: &str, follow: bool) -> Result<ThreadSummary> {
        let t: ThreadSummary = if follow {
            self.api()?.put(&format!("/threads/{root_id}/follow")).await?
        } else {
            self.api()?.delete(&format!("/threads/{root_id}/follow")).await?
        };
        self.inner.store.put_thread(&t)?;
        self.changed(vec![t.conv_id.clone()]);
        Ok(t)
    }

    pub async fn mark_thread_read(&self, root_id: &str) -> Result<ThreadSummary> {
        let t: ThreadSummary = self.api()?.post(&format!("/threads/{root_id}/read"), &serde_json::json!({})).await?;
        self.inner.store.put_thread(&t)?;
        self.changed(vec![t.conv_id.clone()]);
        Ok(t)
    }

    pub fn send_thread_reply(&self, conv_id: &str, root_id: &str, body: &str) -> Result<String> {
        let id = self.send_text(conv_id, body, SendOptions { thread_root: Some(root_id.to_string()), reply_to: None })?;
        let _ = self.inner.store.set_thread_draft(conv_id, root_id, "", now_ms());
        Ok(id)
    }

    // ------------------------------------------------------------------ pins & bookmarks (M-14, C-04..)

    pub fn pins(&self, conv_id: &str) -> Vec<Pin> {
        self.inner.store.pins(conv_id)
    }

    pub async fn pin_message(&self, conv_id: &str, message_id: &str, pin: bool) -> Result<Vec<Pin>> {
        let p = format!("/conversations/{conv_id}/pins/{message_id}");
        let v: Vec<Pin> = if pin { self.api()?.put(&p).await? } else { self.api()?.delete(&p).await? };
        self.inner.store.put_pins(conv_id, &v)?;
        self.changed(vec![conv_id.to_string()]);
        Ok(v)
    }

    pub fn bookmarks(&self, conv_id: &str) -> Vec<Bookmark> {
        self.inner.store.bookmarks(conv_id)
    }

    pub async fn add_bookmark(&self, conv_id: &str, req: &BookmarkRequest) -> Result<Vec<Bookmark>> {
        let v: Vec<Bookmark> = self.api()?.post(&format!("/conversations/{conv_id}/bookmarks"), req).await?;
        self.inner.store.put_bookmarks(conv_id, &v)?;
        self.changed(vec![conv_id.to_string()]);
        Ok(v)
    }

    pub async fn delete_bookmark(&self, conv_id: &str, id: &str) -> Result<Vec<Bookmark>> {
        let v: Vec<Bookmark> = self.api()?.delete(&format!("/bookmarks/{id}")).await?;
        self.inner.store.put_bookmarks(conv_id, &v)?;
        self.changed(vec![conv_id.to_string()]);
        Ok(v)
    }

    // ------------------------------------------------------------------ prefs / sections (NV-*)

    pub fn pref(&self, conv_id: &str) -> ConvPref {
        self.inner.store.pref(conv_id).unwrap_or_else(|| ConvPref::default_for(conv_id))
    }

    pub fn is_muted(&self, conv_id: &str) -> bool {
        self.pref(conv_id).is_muted(now_ms())
    }

    /// The user's custom sections in their order (the built-in order-only rows are left out).
    pub fn sections(&self) -> Vec<Section> {
        self.inner.store.sections().into_iter().filter(|s| s.builtin_key().is_none()).collect()
    }

    /// NV-04: the order of all sidebar sections as keys: `fav`, `channels`, `dms` and custom section ids. Without a
    /// saved order it is the default one (Favourites, custom sections, Channels, Direct messages).
    pub fn section_order(&self) -> Vec<String> {
        let all = self.inner.store.sections();
        let mut keys: Vec<String> = all.iter().map(|s| s.builtin_key().map(str::to_string).unwrap_or_else(|| s.id.clone())).collect();
        if !all.iter().any(|s| s.builtin_key().is_some()) {
            keys.insert(0, "fav".into());
        }
        for b in ["channels", "dms"] {
            if !keys.iter().any(|k| k == b) {
                keys.push(b.into());
            }
        }
        if !keys.iter().any(|k| k == "fav") {
            keys.insert(0, "fav".into());
        }
        keys
    }

    /// NV-04: saves the order of the sidebar sections (keys as returned by `section_order`) for all the user's devices.
    pub async fn reorder_sections(&self, keys: &[String]) -> Result<Vec<Section>> {
        let ids: Vec<String> = keys.iter().map(|k| if matches!(k.as_str(), "fav" | "channels" | "dms") { format!("builtin:{k}") } else { k.clone() }).collect();
        let v: Vec<Section> = self.api()?.post("/sections/order", &proto::SectionOrderRequest { ids }).await?;
        self.inner.store.put_sections(&v)?;
        self.changed(vec![]);
        Ok(v)
    }

    /// NV-04: saves the order of some conversations (one section's list, top to bottom). An empty list resets every
    /// manual order, so the list setting (activity or A-Z) rules again.
    pub async fn reorder_conversations(&self, conv_ids: &[String]) -> Result<()> {
        let prefs: Vec<ConvPref> = self.api()?.post("/conversations/order", &proto::ConvOrderRequest { conv_ids: conv_ids.to_vec() }).await?;
        let mut all = self.inner.store.prefs();
        if conv_ids.is_empty() {
            for p in all.iter_mut() {
                p.position = None;
            }
        }
        for p in prefs {
            all.retain(|x| x.conv_id != p.conv_id);
            all.push(p);
        }
        self.inner.store.put_prefs(&all)?;
        self.changed(conv_ids.to_vec());
        Ok(())
    }

    pub async fn set_pref(&self, conv_id: &str, req: &ConvPrefRequest) -> Result<ConvPref> {
        let p: ConvPref = self.api()?.put_json(&format!("/conversations/{conv_id}/prefs"), req).await?;
        let mut all = self.inner.store.prefs();
        all.retain(|x| x.conv_id != p.conv_id);
        all.push(p.clone());
        self.inner.store.put_prefs(&all)?;
        self.changed(vec![conv_id.to_string()]);
        Ok(p)
    }

    pub async fn set_favourite(&self, conv_id: &str, fav: bool) -> Result<ConvPref> {
        self.set_pref(conv_id, &ConvPrefRequest { favourite: Some(fav), ..Default::default() }).await
    }

    /// `choice`: "off", "1h", "8h", "tomorrow" (8 am local) or "forever" (see `rules::mute_until`).
    pub async fn mute(&self, conv_id: &str, choice: &str, utc_offset_secs: i32) -> Result<ConvPref> {
        let now = now_ms();
        let day = crate::logic::datefmt::local_day(now, utc_offset_secs);
        let tomorrow = (day + 1) * 86_400_000 + 8 * 3_600_000 - utc_offset_secs as i64 * 1000;
        let until = rules::mute_until(choice, now, tomorrow).ok_or_else(|| ClientError::Local("Unknown mute choice.".into()))?;
        self.set_pref(conv_id, &ConvPrefRequest { mute_until_ms: Some(until), ..Default::default() }).await
    }

    pub async fn create_section(&self, name: &str) -> Result<Vec<Section>> {
        let before: Vec<String> = self.sections().into_iter().map(|s| s.id).collect();
        let v: Vec<Section> = self.api()?.post("/sections", &SectionRequest { name: Some(name.into()), ..Default::default() }).await?;
        self.inner.store.put_sections(&v)?;
        // With a saved section order the new section goes before Channels (as it does without one).
        if v.iter().any(|s| s.builtin_key().is_some()) {
            if let Some(new) = v.iter().find(|s| s.builtin_key().is_none() && !before.contains(&s.id)) {
                let mut keys: Vec<String> = self.section_order().into_iter().filter(|k| *k != new.id).collect();
                let at = keys.iter().position(|k| k == "channels").unwrap_or(keys.len());
                keys.insert(at, new.id.clone());
                if let Ok(v2) = self.reorder_sections(&keys).await {
                    return Ok(v2.into_iter().filter(|s| s.builtin_key().is_none()).collect());
                }
            }
        }
        self.changed(vec![]);
        Ok(v.into_iter().filter(|s| s.builtin_key().is_none()).collect())
    }

    pub async fn update_section(&self, id: &str, req: &SectionRequest) -> Result<Vec<Section>> {
        let v: Vec<Section> = self.api()?.patch(&format!("/sections/{id}"), req).await?;
        self.inner.store.put_sections(&v)?;
        self.changed(vec![]);
        Ok(v)
    }

    pub async fn delete_section(&self, id: &str) -> Result<Vec<Section>> {
        let v: Vec<Section> = self.api()?.delete(&format!("/sections/{id}")).await?;
        self.inner.store.put_sections(&v)?;
        self.changed(vec![]);
        Ok(v)
    }

    /// Conversation list in display order (NV-01..05): favourites first, then by section, then the
    /// rest; unread-only / hide-muted filters; sort by activity or name.
    pub fn sorted_conversations(&self, sort_alpha: bool, unread_only: bool, hide_muted: bool) -> Vec<ConversationSummary> {
        let prefs: std::collections::HashMap<String, ConvPref> = self.inner.store.prefs().into_iter().map(|p| (p.conv_id.clone(), p)).collect();
        let now = now_ms();
        let mut v: Vec<ConversationSummary> = self
            .conversations()
            .into_iter()
            .filter(|c| {
                let muted = prefs.get(&c.conversation.id).map(|p| p.is_muted(now)).unwrap_or(false);
                (!unread_only || c.unread > 0 || c.marked_unread) && (!hide_muted || !muted)
            })
            .collect();
        let title = |c: &ConversationSummary| self.conversation_title(c).to_lowercase();
        if sort_alpha {
            v.sort_by_key(title);
        } else {
            v.sort_by_key(|c| std::cmp::Reverse(c.conversation.last_message_ts.unwrap_or(c.conversation.created_ms)));
        }
        // NV-04: conversations the user placed by hand keep that order (new ones, without a place, stay on top).
        v.sort_by_key(|c| prefs.get(&c.conversation.id).and_then(|p| p.position).map(|p| (1, p)).unwrap_or((0, 0)));
        v.sort_by_key(|c| !prefs.get(&c.conversation.id).map(|p| p.favourite).unwrap_or(false));
        v
    }

    // ------------------------------------------------------------------ groups

    pub fn groups(&self) -> Vec<UserGroup> {
        self.inner.store.groups()
    }

    // ------------------------------------------------------------------ drafts sync (M-23)

    /// Saves a draft locally and pushes it to the server so it follows you between devices.
    pub fn save_draft(&self, conv_id: &str, thread_root: Option<&str>, body: &str) {
        let now = now_ms();
        match thread_root {
            None => {
                let _ = self.inner.store.set_draft(conv_id, body, now);
            }
            Some(t) => {
                let _ = self.inner.store.set_thread_draft(conv_id, t, body, now);
            }
        }
        if let Ok(api) = self.api() {
            let (c, t, b) = (conv_id.to_string(), thread_root.map(str::to_string), body.to_string());
            self.inner.handle.spawn(async move {
                let _: Result<DraftItem> = api.put_json(&format!("/conversations/{c}/draft"), &DraftRequest { thread_root: t, body: b }).await;
            });
        }
    }

    pub fn thread_draft(&self, conv_id: &str, root: &str) -> String {
        self.inner.store.thread_draft(conv_id, root).unwrap_or_default()
    }

    pub fn all_drafts(&self) -> Vec<DraftItem> {
        let mut v = self.inner.store.all_drafts();
        v.retain(|d| !self.is_hidden(&d.conv_id));
        v
    }

    // ------------------------------------------------------------------ forward / permalink (M-19..22)

    pub async fn forward_message(&self, id: &str, conv_ids: &[String], comment: Option<&str>) -> Result<Vec<Message>> {
        let v: Vec<Message> = self
            .api()?
            .post(&format!("/messages/{id}/forward"), &ForwardRequest { conv_ids: conv_ids.to_vec(), comment: comment.map(str::to_string) })
            .await?;
        self.inner.store.put_messages(&v)?;
        self.changed(conv_ids.to_vec());
        Ok(v)
    }

    pub fn permalink(&self, m: &Message) -> String {
        let server = self.session().map(|s| s.server).unwrap_or_default();
        rules::make_permalink(&server, &m.conv_id, &m.id)
    }

    /// Resolves a pasted permalink into a cached/fetched message and loads its surroundings.
    pub async fn open_permalink(&self, link: &str) -> Result<Message> {
        let p = rules::parse_permalink(link).ok_or_else(|| ClientError::Local("That is not a SuperChat link.".into()))?;
        if self.is_hidden(&p.conv_id) {
            return Err(ClientError::Local("That message could not be found.".into()));
        }
        let m: Message = self.api()?.get(&format!("/messages/{}", p.message_id)).await?;
        let h: HistoryResponse = self.api()?.get(&format!("/conversations/{}/messages?around_id={}&limit=40", m.conv_id, m.id)).await?;
        self.inner.store.put_messages(&h.messages)?;
        self.changed(vec![m.conv_id.clone()]);
        Ok(m)
    }

    /// Jump to a message by id: fetches a window around it.
    pub async fn load_around(&self, conv_id: &str, message_id: &str) -> Result<Vec<Message>> {
        let h: HistoryResponse = self.api()?.get(&format!("/conversations/{conv_id}/messages?around_id={message_id}&limit=40")).await?;
        self.inner.store.put_messages(&h.messages)?;
        self.changed(vec![conv_id.to_string()]);
        Ok(h.messages)
    }

    // ------------------------------------------------------------------ push (N-01, SV-13)

    pub async fn push_registrations(&self) -> Result<Vec<PushRegistration>> {
        self.api()?.get("/me/push").await
    }

    pub async fn register_push(&self, reg: &PushRegistration) -> Result<PushRegistration> {
        self.api()?.put_json("/me/push", reg).await
    }

    pub async fn unregister_push(&self, id: &str) -> Result<()> {
        let _: serde_json::Value = self.api()?.delete(&format!("/me/push/{id}")).await?;
        Ok(())
    }

    // ------------------------------------------------------------------ sessions (SV-11)

    pub async fn revoke_session(&self, id: &str) -> Result<()> {
        let _: serde_json::Value = self.api()?.delete(&format!("/sessions/{id}")).await?;
        Ok(())
    }

    /// Privacy screen (SV-12): only meaningful when the server enabled privacy controls for me.
    pub async fn set_hide_last_active(&self, hide: bool) -> Result<Me> {
        self.update_profile(UpdateMeRequest { hide_last_active: Some(hide), ..Default::default() }).await
    }
}
