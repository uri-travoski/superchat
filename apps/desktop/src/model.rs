//! Turns client-core data into Slint models. Pure functions of the cache; no UI handles here.

use crate::{ActivityRow, BlockItem, BookmarkItem, ConvItem, EmojiItem, FileItem, LabelItem, MsgItem, PersonItem, ReactItem, RowItem, SavedRow, SideRow};
use client_core::logic::{datefmt, initials, markup};
use client_core::{Client, SendState, TimelineItem};
use proto::types::{Attachment, ConversationKind, ConversationSummary, Message, MessageKind};
use slint::{ModelRc, SharedString, VecModel};
use std::collections::{HashMap, HashSet};

pub const ACCENTS: [(&str, u8, u8, u8); 4] = [("#0dbd8b", 0x0d, 0xbd, 0x8b), ("#0467dd", 0x04, 0x67, 0xdd), ("#7b61ff", 0x7b, 0x61, 0xff), ("#e8590c", 0xe8, 0x59, 0x0c)];

#[derive(Clone)]
pub struct View {
    pub hour24: bool,
    pub accent: usize,
    pub off: i32,
    pub now: i64,
}

pub fn color_of(id: &str) -> slint::Color {
    let (r, g, b) = initials::color_rgb(id);
    slint::Color::from_rgb_u8(r, g, b)
}

pub fn model<T: Clone + 'static>(v: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(v))
}

thread_local! {
    static EMOJI_IMAGES: std::cell::RefCell<HashMap<String, Option<slint::Image>>> = Default::default();
}

fn strip_vs(e: &str) -> String {
    e.chars().filter(|c| *c != '\u{FE0F}').collect()
}

/// Colour picture of an emoji (the text renderers draw colour fonts as outlines). None for emoji we do not bundle.
pub fn emoji_image(e: &str) -> Option<slint::Image> {
    let key = strip_vs(e);
    EMOJI_IMAGES.with(|c| {
        c.borrow_mut()
            .entry(key.clone())
            .or_insert_with(|| {
                let bytes = crate::emoji_data::EMOJI_PNG.iter().find(|(k, _)| strip_vs(k) == key)?.1;
                let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png).ok()?.to_rgba8();
                let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(img.as_raw(), img.width(), img.height());
                Some(slint::Image::from_rgba8(buf))
            })
            .clone()
    })
}

pub fn emoji_items(list: Vec<String>) -> Vec<EmojiItem> {
    list.into_iter()
        .map(|e| {
            let img = emoji_image(&e);
            EmojiItem { ch: e.into(), has_img: img.is_some(), img: img.unwrap_or_default() }
        })
        .collect()
}

pub fn plain(body: &str) -> String {
    markup::to_plain(body)
}

fn preview_of(m: &Message) -> String {
    if m.deleted {
        return "Message deleted".into();
    }
    let body = plain(&m.body);
    if body.is_empty() {
        m.attachments.first().map(|a| format!("File: {}", a.name)).unwrap_or_default()
    } else {
        body
    }
}

/// Conversation rows grouped as Favourites, one block per section, then the rest.
pub fn conv_items(c: &Client, sort_alpha: bool, unread_only: bool, hide_muted: bool, v: &View) -> Vec<ConvItem> {
    let list = c.sorted_conversations(sort_alpha, unread_only, hide_muted);
    let sections = c.sections();
    let mut buckets: Vec<(String, Vec<ConversationSummary>)> = vec![("Favourites".into(), vec![])];
    for s in &sections {
        buckets.push((s.name.clone(), vec![]));
    }
    buckets.push((if sections.is_empty() { String::new() } else { "Conversations".into() }, vec![]));
    let last = buckets.len() - 1;
    for cs in list {
        let p = c.pref(&cs.conversation.id);
        let idx = if p.favourite {
            0
        } else if let Some(sid) = &p.section_id {
            sections.iter().position(|s| &s.id == sid).map(|i| i + 1).unwrap_or(last)
        } else {
            last
        };
        buckets[idx].1.push(cs);
    }
    let any_header = buckets[..last].iter().any(|(_, l)| !l.is_empty());
    let mut out = vec![];
    for (name, list) in buckets {
        for (i, cs) in list.into_iter().enumerate() {
            let title = c.conversation_title(&cs);
            let last_msg = c.timeline(&cs.conversation.id, 1, None).pop();
            let (preview, ts) = match &last_msg {
                Some(t) => (preview_of(&t.message), t.message.ts),
                None => (String::new(), 0),
            };
            let p = c.pref(&cs.conversation.id);
            out.push(ConvItem {
                id: cs.conversation.id.clone().into(),
                initials: initials::initials(&title).into(),
                color: color_of(&cs.conversation.id),
                title: title.into(),
                unread: if cs.unread > 0 { cs.unread as i32 } else if cs.marked_unread { 1 } else { 0 },
                mentions: cs.mentions as i32,
                preview: preview.into(),
                stamp: if ts > 0 { datefmt::list_stamp(ts, v.now, v.off, v.hour24).into() } else { SharedString::new() },
                kind: match cs.conversation.kind {
                    ConversationKind::Dm => "dm",
                    ConversationKind::SelfNote => "self",
                    ConversationKind::GroupDm => "group",
                    ConversationKind::Channel => "channel",
                }
                .into(),
                favourite: p.favourite,
                muted: p.is_muted(v.now),
                archived: cs.conversation.archived,
                header: if i == 0 && any_header { name.clone().into() } else { SharedString::new() },
            });
        }
    }
    out
}

fn file_kind(a: &Attachment) -> i32 {
    let t = a.content_type.as_str();
    if t.starts_with("image/") {
        1
    } else if t.starts_with("video/") {
        2
    } else if t.starts_with("audio/") {
        3
    } else {
        0
    }
}

/// 83000 ms -> "1:23".
pub fn clock_text(ms: u32) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

/// A video poster frame (small JPEG, base64) as a Slint image.
fn poster_image(b64: &str) -> Option<slint::Image> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(img.as_raw(), img.width(), img.height());
    Some(slint::Image::from_rgba8(buf))
}

pub fn human_size(n: i64) -> String {
    let n = n.max(0) as f64;
    if n < 1024.0 {
        format!("{n:.0} B")
    } else if n < 1024.0 * 1024.0 {
        format!("{:.0} KB", n / 1024.0)
    } else {
        format!("{:.1} MB", n / 1024.0 / 1024.0)
    }
}

/// Everything `msg_item` needs besides the message: lets the thread panel reuse it.
pub struct Ctx<'a> {
    pub c: &'a Client,
    pub me: String,
    pub v: &'a View,
    pub atts: &'a mut HashMap<String, Attachment>,
    /// Image attachments that need downloading for an inline preview.
    pub want_images: &'a mut Vec<Attachment>,
    pub pinned: HashSet<String>,
    pub replies: HashMap<String, i64>,
    pub can_pin: bool,
    pub saved: HashSet<String>,
}

pub fn msg_item(cx: &mut Ctx, t: &TimelineItem, day: String, divider: bool) -> MsgItem {
    let m = &t.message;
    let name = cx.c.display_name(&m.sender_id);
    let accent = ACCENTS[cx.v.accent.min(3)].0;
    let blocks: Vec<BlockItem> = if m.deleted {
        vec![]
    } else if m.kind == MessageKind::System {
        vec![BlockItem { kind: 0, styled: slint::StyledText::from_plain_text(&plain(&m.body)), text: plain(&m.body).into() }]
    } else {
        markup::ui_blocks(&m.body, accent)
            .into_iter()
            .map(|b| BlockItem {
                kind: b.kind as i32,
                styled: slint::StyledText::from_markdown(&b.markdown).unwrap_or_else(|_| slint::StyledText::from_plain_text(&b.plain)),
                text: b.plain.into(),
            })
            .collect()
    };
    let files: Vec<FileItem> = m
        .attachments
        .iter()
        .map(|a| {
            cx.atts.insert(a.id.clone(), a.clone());
            let kind = file_kind(a);
            let mut img = slint::Image::default();
            let mut has = false;
            if kind == 1 && !a.removed {
                match cx.c.cached_attachment_path(a) {
                    Some(p) => {
                        if let Ok(i) = slint::Image::load_from_path(&p) {
                            img = i;
                            has = true;
                        }
                    }
                    None if a.size < 12 * 1024 * 1024 => cx.want_images.push(a.clone()),
                    None => {}
                }
            }
            let (mut poster, mut has_poster) = (slint::Image::default(), false);
            if let Some(b64) = a.meta.as_ref().and_then(|m| m.poster_b64.as_deref()) {
                if let Some(i) = poster_image(b64) {
                    poster = i;
                    has_poster = true;
                }
            }
            let dur = a.meta.as_ref().and_then(|m| m.duration_ms).unwrap_or(0);
            let ext = a.name.rsplit_once('.').map(|(_, e)| e.to_uppercase()).filter(|e| e.len() <= 4).unwrap_or_default();
            FileItem {
                id: a.id.clone().into(),
                name: a.name.clone().into(),
                size: human_size(a.size).into(),
                kind,
                img,
                has_img: has,
                removed: a.removed,
                peaks: model(a.meta.as_ref().map(|m| m.peaks.iter().map(|p| *p as i32).collect::<Vec<_>>()).unwrap_or_default()),
                dur_text: if dur > 0 { clock_text(dur).into() } else { SharedString::new() },
                has_poster,
                poster,
                ext: ext.into(),
            }
        })
        .collect();
    let mut by_emoji: Vec<ReactItem> = m
        .reactions
        .iter()
        .filter(|r| !r.user_ids.is_empty())
        .map(|r| {
            let img = emoji_image(&r.emoji);
            ReactItem { emoji: r.emoji.clone().into(), count: r.user_ids.len() as i32, mine: r.user_ids.iter().any(|u| *u == cx.me), has_img: img.is_some(), img: img.unwrap_or_default() }
        })
        .collect();
    by_emoji.truncate(12);
    let quote = m
        .reply_to
        .as_ref()
        .and_then(|id| cx.c.store().message(id).ok().flatten())
        .map(|q| format!("{}: {}", cx.c.display_name(&q.sender_id), preview_of(&q)))
        .unwrap_or_default();
    let system = m.kind == MessageKind::System;
    let (reply_people, reply_time) = if cx.replies.get(&m.id).copied().unwrap_or(0) > 0 {
        let rs = cx.c.thread_replies(&m.conv_id, &m.id);
        let mut seen: Vec<String> = vec![];
        for r in rs.iter().rev() {
            if !seen.contains(&r.message.sender_id) {
                seen.push(r.message.sender_id.clone());
            }
            if seen.len() == 3 {
                break;
            }
        }
        let people: Vec<PersonItem> = seen
            .iter()
            .map(|id| {
                let n = cx.c.display_name(id);
                PersonItem { id: id.clone().into(), name: n.clone().into(), initials: initials::initials(&n).into(), color: color_of(id), sub: SharedString::new(), sel: false, tag: SharedString::new() }
            })
            .collect();
        let last = rs.last().map(|r| datefmt::list_stamp(r.message.ts, cx.v.now, cx.v.off, cx.v.hour24)).unwrap_or_default();
        (people, last)
    } else {
        (vec![], String::new())
    };
    // F-01: a message with files that is still in the outbox shows its upload progress, or the error with Retry / Discard.
    let upload: (i32, String, String) = match (t.local, m.client_id.as_deref()) {
        (true, Some(cid)) if !m.attachments.is_empty() => match (t.state, cx.c.upload_progress(cid)) {
            (SendState::Failed, _) => (-1, String::new(), t.error.clone().unwrap_or_else(|| "Could not upload".into())),
            (_, Some(u)) => {
                let pct = if u.total == 0 { 0 } else { (u.sent * 100 / u.total).min(100) as i32 };
                let mb = |b: u64| format!("{:.1}", b as f64 / 1_048_576.0);
                let which = if u.file_count > 1 { format!(" ({} of {})", u.file_index + 1, u.file_count) } else { String::new() };
                (pct, format!("Uploading {}{} - {}% ({} of {} MB)", u.file_name, which, pct, mb(u.sent), mb(u.total)), String::new())
            }
            (_, None) => (0, "Waiting to upload...".into(), String::new()),
        },
        _ => (-1, String::new(), String::new()),
    };
    let forwarded = m.forwarded.as_ref().map(|f| format!("Forwarded from {}", cx.c.display_name(&f.sender_id))).unwrap_or_default();
    MsgItem {
        id: m.id.clone().into(),
        initials: initials::initials(&name).into(),
        color: color_of(&m.sender_id),
        sender: name.into(),
        time: datefmt::format_time(m.ts, cx.v.off, cx.v.hour24).into(),
        mine: m.sender_id == cx.me,
        edited: m.is_edited(),
        deleted: m.deleted,
        can_edit: !t.local && cx.c.can_edit(m),
        state: match t.state {
            SendState::Sending => 1,
            SendState::Failed => 2,
            SendState::Delivered => 3,
            SendState::Sent => 0,
        },
        day: day.into(),
        blocks: model(blocks),
        files: model(files),
        reactions: model(by_emoji),
        replies: cx.replies.get(&m.id).copied().unwrap_or(0) as i32,
        pinned: cx.pinned.contains(&m.id),
        forwarded: forwarded.into(),
        quote: quote.into(),
        divider,
        can_pin: cx.can_pin && !t.local && !m.deleted,
        is_app: cx.c.user(&m.sender_id).map(|u| u.is_bot).unwrap_or(false),
        has_form: m.form.is_some() && !m.deleted,
        form_title: m.form.as_ref().and_then(|f| f.title.clone()).unwrap_or_default().into(),
        form_desc: m.form.as_ref().and_then(|f| f.description.clone()).unwrap_or_default().into(),
        form_fields: m.form.as_ref().map(|f| !f.fields.is_empty()).unwrap_or(false),
        form_buttons: model(m.form.as_ref().map(crate::forms::buttons).unwrap_or_default()),
        responded: cx.c.i_responded(m),
        grouped: false,
        system,
        saved: cx.saved.contains(&m.id),
        reply_people: model(reply_people),
        reply_time: reply_time.into(),
        reply_unread: false,
        sender_id: m.sender_id.clone().into(),
        upload_pct: upload.0,
        upload_text: upload.1.into(),
        upload_error: upload.2.into(),
    }
}

/// Top-level messages of a conversation (thread replies live in the thread panel).
/// Returns the items and the index of the first unread message, if any.
pub fn msg_items(cx: &mut Ctx, conv: &str, limit: usize, last_read: i64) -> (Vec<MsgItem>, Option<usize>) {
    let mut last_day = i64::MIN;
    let mut first_unread = None;
    let mut out = vec![];
    let mut prev: Option<(String, i64, bool)> = None;
    for t in cx.c.timeline(conv, limit, None).into_iter().filter(|t| t.message.thread_root.is_none() || t.message.thread_root.as_deref() == Some("")) {
        let m = &t.message;
        let day = datefmt::local_day(m.ts, cx.v.off);
        let label = if day != last_day { datefmt::day_label(m.ts, cx.v.now, cx.v.off) } else { String::new() };
        last_day = day;
        let divider = first_unread.is_none() && !t.local && m.seq > last_read && m.sender_id != cx.me;
        if divider {
            first_unread = Some(out.len());
        }
        let mut it = msg_item(cx, &t, label.clone(), divider);
        let system = it.system;
        it.grouped = !system && label.is_empty() && !divider && prev.as_ref().map(|(s, ts, sys)| !*sys && *s == m.sender_id && (m.ts - *ts).abs() < 5 * 60_000).unwrap_or(false);
        prev = Some((m.sender_id.clone(), m.ts, system));
        out.push(it);
    }
    (out, first_unread)
}

pub fn person(u: &proto::types::User) -> PersonItem {
    PersonItem { id: u.id.clone().into(), name: u.display_name.clone().into(), initials: initials::initials(&u.display_name).into(), color: color_of(&u.id), sub: format!("@{}", u.username).into(), sel: false, tag: SharedString::new() }
}

pub fn person_blank() -> PersonItem {
    PersonItem { id: SharedString::new(), name: SharedString::new(), initials: SharedString::new(), color: slint::Color::from_rgb_u8(0, 0, 0), sub: SharedString::new(), sel: false, tag: SharedString::new() }
}

pub fn row(kind: i32, id: &str, conv: &str, title: String, text: String, sub: String) -> RowItem {
    RowItem {
        kind,
        id: id.into(),
        conv: conv.into(),
        title: title.clone().into(),
        sub: sub.into(),
        text: text.clone().into(),
        badge: 0,
        flag: false,
        initials: initials::initials(&title).into(),
        color: color_of(id),
        header: SharedString::new(),
        pre: text.clone().into(),
        hit: SharedString::new(),
        post: SharedString::new(),
    }
}

pub fn bookmarks(c: &Client, conv: &str) -> Vec<BookmarkItem> {
    c.bookmarks(conv).into_iter().filter_map(|b| b.url.clone().map(|u| BookmarkItem { id: b.id.into(), title: b.title.into(), url: u.into() })).collect()
}


// ------------------------------------------------------------------------------------------------
// 1.2: sidebar, DM list, Activity, Later

fn presence_of(c: &Client, cs: &ConversationSummary, me: &str) -> i32 {
    if cs.conversation.kind != ConversationKind::Dm {
        return 0;
    }
    cs.conversation
        .members
        .iter()
        .find(|m| m.user_id != me)
        .and_then(|m| c.user(&m.user_id))
        .and_then(|u| u.presence)
        .map(|p| match p {
            proto::types::Presence::Online => 1,
            proto::types::Presence::Away => 2,
            proto::types::Presence::Dnd => 3,
            proto::types::Presence::Offline => 0,
        })
        .unwrap_or(0)
}

fn conv_row(c: &Client, cs: &ConversationSummary, me: &str, v: &View) -> SideRow {
    let title = c.conversation_title(cs);
    let p = c.pref(&cs.conversation.id);
    let last = c.timeline(&cs.conversation.id, 1, None).pop();
    let (preview, ts) = match &last {
        Some(t) => {
            let who = if t.message.sender_id == me { "You".to_string() } else { c.display_name(&t.message.sender_id) };
            let body = preview_of(&t.message);
            (if cs.conversation.kind == ConversationKind::Dm || t.message.kind == MessageKind::System { body } else { format!("{who}: {body}") }, t.message.ts)
        }
        None => (String::new(), 0),
    };
    SideRow {
        kind: 1,
        id: cs.conversation.id.clone().into(),
        title: title.clone().into(),
        icon: SharedString::new(),
        initials: initials::initials(&title).into(),
        color: color_of(&cs.conversation.id),
        unread: if cs.unread > 0 { cs.unread as i32 } else if cs.marked_unread { 1 } else { 0 },
        mentions: cs.mentions as i32,
        muted: p.is_muted(v.now),
        favourite: p.favourite,
        collapsed: false,
        count: 0,
        conv_kind: match cs.conversation.kind {
            ConversationKind::Dm => "dm",
            ConversationKind::SelfNote => "self",
            ConversationKind::GroupDm => "group",
            ConversationKind::Channel => "channel",
        }
        .into(),
        presence: presence_of(c, cs, me),
        locked: false,
        draft: !c.draft(&cs.conversation.id).trim().is_empty(),
        plus: SharedString::new(),
        preview: preview.into(),
        stamp: if ts > 0 { datefmt::list_stamp(ts, v.now, v.off, v.hour24).into() } else { SharedString::new() },
        user_id: match cs.conversation.kind {
            ConversationKind::Dm => cs.conversation.members.iter().map(|m| m.user_id.clone()).find(|u| *u != me).unwrap_or_default().into(),
            ConversationKind::SelfNote => me.to_string().into(),
            _ => SharedString::new(),
        },
    }
}

fn blank_row(kind: i32, id: &str, title: &str) -> SideRow {
    SideRow {
        kind,
        id: id.into(),
        title: title.into(),
        icon: SharedString::new(),
        initials: SharedString::new(),
        color: slint::Color::from_rgb_u8(0, 0, 0),
        unread: 0,
        mentions: 0,
        muted: false,
        favourite: false,
        collapsed: false,
        count: 0,
        conv_kind: SharedString::new(),
        presence: 0,
        locked: false,
        draft: false,
        plus: SharedString::new(),
        preview: SharedString::new(),
        stamp: SharedString::new(),
        user_id: SharedString::new(),
    }
}

pub struct SideOpts<'a> {
    pub sort_alpha: bool,
    pub unread_only: bool,
    pub hide_muted: bool,
    pub collapsed: &'a [String],
    pub drafts: usize,
}

/// The Home sidebar: Threads, Drafts & sent, Later, then Favourites, custom sections, Channels and Direct messages.
/// A collapsed section still shows its unread conversations (like Slack), so nothing important hides.
pub fn side_rows(c: &Client, o: &SideOpts, v: &View) -> Vec<SideRow> {
    let me = c.my_id().unwrap_or_default();
    let mut out = vec![];
    // Shortcuts: Threads, Drafts & sent and Later, a collapsible section like the others. When collapsed, a link with
    // something new (unread threads) stays visible so nothing important hides.
    let short_collapsed = o.collapsed.iter().any(|x| x == "sec:shortcuts");
    let mut threads = blank_row(2, "threads", "Threads");
    threads.icon = "thread".into();
    threads.unread = c.thread_unread_total() as i32;
    let mut drafts = blank_row(2, "drafts", "Drafts & sent");
    drafts.icon = "edit".into();
    drafts.count = (o.drafts + c.scheduled_pending(None).len()) as i32;
    let mut later = blank_row(2, "later", "Later");
    later.icon = "bookmark".into();
    later.count = c.saved_items().len() as i32;
    let mut head = blank_row(0, "sec:shortcuts", "Shortcuts");
    head.collapsed = short_collapsed;
    head.unread = threads.unread;
    out.push(head);
    for r in [threads, drafts, later] {
        if short_collapsed && r.unread == 0 {
            continue;
        }
        out.push(r);
    }

    let list = c.sorted_conversations(o.sort_alpha, o.unread_only, o.hide_muted);
    let sections = c.sections();
    // bucket index: 0 favourites, 1..=n custom, n+1 channels, n+2 direct messages
    let n = sections.len();
    let mut buckets: Vec<Vec<ConversationSummary>> = vec![vec![]; n + 3];
    let mut archived: Vec<ConversationSummary> = vec![];
    for cs in list {
        if cs.conversation.archived {
            archived.push(cs);
            continue;
        }
        let p = c.pref(&cs.conversation.id);
        let idx = if p.favourite {
            0
        } else if let Some(i) = p.section_id.as_ref().and_then(|sid| sections.iter().position(|s| &s.id == sid)) {
            i + 1
        } else if cs.conversation.kind == ConversationKind::Channel {
            n + 1
        } else {
            n + 2
        };
        buckets[idx].push(cs);
    }
    let push_section = |out: &mut Vec<SideRow>, key: String, title: &str, plus: &str, collapsed: bool, list: &[ConversationSummary], hint: &str| {
        let mut h = blank_row(0, &key, title);
        h.collapsed = collapsed;
        h.plus = plus.into();
        h.count = list.len() as i32;
        h.unread = list.iter().map(|c| c.unread as i32).sum();
        out.push(h);
        let mut shown = 0;
        for cs in list {
            if collapsed && cs.unread == 0 && !cs.marked_unread {
                continue;
            }
            shown += 1;
            out.push(conv_row(c, cs, &me, v));
        }
        if !collapsed && shown == 0 && !hint.is_empty() {
            out.push(blank_row(3, "", hint));
        }
    };
    let is = |k: &str| o.collapsed.iter().any(|x| x == k);
    if !buckets[0].is_empty() {
        push_section(&mut out, "sec:fav".into(), "Favourites", "", is("sec:fav"), &buckets[0], "");
    }
    for (i, sec) in sections.iter().enumerate() {
        push_section(&mut out, format!("sec:{}", sec.id), &sec.name, "", sec.collapsed, &buckets[i + 1], "Move conversations here from their menu.");
    }
    push_section(&mut out, "sec:channels".into(), "Channels", "channel", is("sec:channels"), &buckets[n + 1], "No channels yet. An admin can add you to channels.");
    push_section(&mut out, "sec:dms".into(), "Direct messages", "dm", is("sec:dms"), &buckets[n + 2], "Start a conversation with the + button.");
    if !archived.is_empty() {
        // Archived conversations are read-only; the section starts collapsed (its key in the list means "expanded").
        push_section(&mut out, "sec:archived".into(), "Archived", "", !is("sec:archived"), &archived, "");
    }
    // Phone: room under the last row so the New message button never covers a badge.
    out.push(blank_row(4, "spacer", ""));
    out
}

/// Direct messages, group conversations and notes to self, newest first.
pub fn dm_rows(c: &Client, v: &View) -> Vec<SideRow> {
    let me = c.my_id().unwrap_or_default();
    c.sorted_conversations(false, false, false)
        .into_iter()
        .filter(|cs| cs.conversation.kind != ConversationKind::Channel)
        .map(|cs| conv_row(c, &cs, &me, v))
        .collect()
}

/// Activity entries: filter 0 all, 1 mentions, 2 threads, 3 reactions.
pub fn activity_rows(c: &Client, filter: i32, v: &View) -> Vec<ActivityRow> {
    c.activity()
        .into_iter()
        .filter(|a| match filter {
            1 => a.kind == "mention",
            2 => a.kind == "thread_reply",
            3 => a.kind == "reaction",
            _ => true,
        })
        .filter_map(|a| {
            let m = c.store().message(&a.message_id).ok().flatten();
            let actor = c.display_name(&a.actor_id);
            let conv_title = c.conversations().into_iter().find(|x| x.conversation.id == a.conv_id).map(|cs| {
                let t = c.conversation_title(&cs);
                if cs.conversation.kind == ConversationKind::Channel { format!("#{t}") } else { t }
            })?;
            let (kind, title) = match a.kind.as_str() {
                "mention" => (0, format!("{actor} mentioned you in {conv_title}")),
                "reaction" => (1, format!("{actor} reacted {} to your message in {conv_title}", a.emoji.clone().unwrap_or_default())),
                _ => (2, format!("{actor} replied in a thread in {conv_title}")),
            };
            let img = a.emoji.as_deref().and_then(emoji_image);
            Some(ActivityRow {
                id: a.id.clone().into(),
                kind,
                actor: actor.clone().into(),
                initials: initials::initials(&actor).into(),
                color: color_of(&a.actor_id),
                title: title.into(),
                snippet: m.map(|m| preview_of(&m)).unwrap_or_default().into(),
                conv: a.conv_id.clone().into(),
                msg: a.message_id.clone().into(),
                root: a.thread_root.clone().unwrap_or_default().into(),
                time: datefmt::list_stamp(a.ts, v.now, v.off, v.hour24).into(),
                unread: a.unread,
                emoji: a.emoji.clone().unwrap_or_default().into(),
                has_img: img.is_some(),
                img: img.unwrap_or_default(),
                user_id: a.actor_id.clone().into(),
            })
        })
        .collect()
}

pub const LABEL_COLORS: [&str; 6] = ["#0dbd8b", "#0467dd", "#7b61ff", "#e8590c", "#d6336c", "#868e96"];

pub fn parse_color(hex: &str) -> slint::Color {
    let h = hex.trim_start_matches('#');
    let n = u32::from_str_radix(h, 16).unwrap_or(0x868e96);
    slint::Color::from_rgb_u8((n >> 16) as u8, (n >> 8) as u8, n as u8)
}

pub fn label_items(c: &Client, on: &[String]) -> Vec<LabelItem> {
    c.labels().into_iter().map(|l| LabelItem { on: on.contains(&l.id), id: l.id.into(), name: l.name.into(), tint: parse_color(&l.color) }).collect()
}

/// Saved messages (newest first) with their label chips; `label` filters to one label id.
pub fn saved_rows(c: &Client, label: &str, v: &View) -> Vec<SavedRow> {
    let labels = c.labels();
    c.saved_items()
        .into_iter()
        .filter(|sv| label.is_empty() || sv.item.label_ids.iter().any(|l| l == label))
        .map(|sv| {
            let conv_title = c.conversations().into_iter().find(|x| x.conversation.id == sv.item.conv_id).map(|cs| {
                let t = c.conversation_title(&cs);
                if cs.conversation.kind == ConversationKind::Channel { format!("#{t}") } else { t }
            });
            let (sender, text, deleted, ts) = match &sv.message {
                Some(m) => (c.display_name(&m.sender_id), preview_of(m), m.deleted, m.ts),
                None => (String::new(), String::new(), true, sv.item.saved_ms),
            };
            let chips: Vec<LabelItem> = sv
                .item
                .label_ids
                .iter()
                .filter_map(|id| labels.iter().find(|l| &l.id == id))
                .map(|l| LabelItem { id: l.id.clone().into(), name: l.name.clone().into(), tint: parse_color(&l.color), on: true })
                .collect();
            SavedRow {
                id: sv.item.message_id.clone().into(),
                conv: sv.item.conv_id.clone().into(),
                conv_title: conv_title.unwrap_or_else(|| "Conversation".into()).into(),
                initials: initials::initials(&sender).into(),
                color: color_of(sv.message.as_ref().map(|m| m.sender_id.as_str()).unwrap_or("")),
                sender: sender.into(),
                time: datefmt::list_stamp(ts, v.now, v.off, v.hour24).into(),
                text: text.into(),
                deleted,
                labels: model(chips),
                user_id: sv.message.as_ref().map(|m| m.sender_id.clone()).unwrap_or_default().into(),
            }
        })
        .collect()
}
