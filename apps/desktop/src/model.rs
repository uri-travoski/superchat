//! Turns client-core data into Slint models. Pure functions of the cache; no UI handles here.

use crate::{BlockItem, BookmarkItem, ConvItem, EmojiItem, FileItem, MsgItem, PersonItem, ReactItem, RowItem};
use client_core::logic::{datefmt, initials, markup};
use client_core::{Client, SendState, TimelineItem};
use proto::types::{Attachment, ConversationKind, ConversationSummary, Message};
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
}

pub fn msg_item(cx: &mut Ctx, t: &TimelineItem, day: String, divider: bool) -> MsgItem {
    let m = &t.message;
    let name = cx.c.display_name(&m.sender_id);
    let accent = ACCENTS[cx.v.accent.min(3)].0;
    let blocks: Vec<BlockItem> = if m.deleted {
        vec![]
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
            FileItem { id: a.id.clone().into(), name: a.name.clone().into(), size: human_size(a.size).into(), kind, img, has_img: has, removed: a.removed }
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
    }
}

/// Top-level messages of a conversation (thread replies live in the thread panel).
/// Returns the items and the index of the first unread message, if any.
pub fn msg_items(cx: &mut Ctx, conv: &str, limit: usize, last_read: i64) -> (Vec<MsgItem>, Option<usize>) {
    let mut last_day = i64::MIN;
    let mut first_unread = None;
    let mut out = vec![];
    for t in cx.c.timeline(conv, limit, None).into_iter().filter(|t| t.message.thread_root.is_none() || t.message.thread_root.as_deref() == Some("")) {
        let m = &t.message;
        let day = datefmt::local_day(m.ts, cx.v.off);
        let label = if day != last_day { datefmt::day_label(m.ts, cx.v.now, cx.v.off) } else { String::new() };
        last_day = day;
        let divider = first_unread.is_none() && !t.local && m.seq > last_read && m.sender_id != cx.me;
        if divider {
            first_unread = Some(out.len());
        }
        out.push(msg_item(cx, &t, label, divider));
    }
    (out, first_unread)
}

pub fn person(u: &proto::types::User) -> PersonItem {
    PersonItem { id: u.id.clone().into(), name: u.display_name.clone().into(), initials: initials::initials(&u.display_name).into(), color: color_of(&u.id), sub: format!("@{}", u.username).into() }
}

pub fn row(kind: i32, id: &str, conv: &str, title: String, text: String, sub: String) -> RowItem {
    RowItem {
        kind,
        id: id.into(),
        conv: conv.into(),
        title: title.clone().into(),
        sub: sub.into(),
        text: text.into(),
        badge: 0,
        flag: false,
        initials: initials::initials(&title).into(),
        color: color_of(id),
        header: SharedString::new(),
    }
}

pub fn bookmarks(c: &Client, conv: &str) -> Vec<BookmarkItem> {
    c.bookmarks(conv).into_iter().filter_map(|b| b.url.clone().map(|u| BookmarkItem { id: b.id.into(), title: b.title.into(), url: u.into() })).collect()
}
