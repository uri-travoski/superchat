//! NV-04: reordering the sidebar. Conversations can be dragged within a section or into another one, sections can be
//! dragged above or below each other, and every row has Move up / Move down in its menu (the way to do it on a phone).
//! The result is saved on the server, so it follows the user to every device.

use super::{toast, AppWindow, Sh};
use client_core::Client;
use proto::types::{ConversationKind, ConvPrefRequest};
use slint::ComponentHandle;
use std::collections::HashMap;

/// The conversation ids of every section in display order, as the sidebar shows them.
fn lists(c: &Client, sort_alpha: bool, unread_only: bool, hide_muted: bool) -> HashMap<String, Vec<String>> {
    let sections = c.sections();
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    // The order is saved for the whole section, so the lists are never filtered by "unread only" or "hide muted".
    let _ = (unread_only, hide_muted);
    for cs in c.sorted_conversations(sort_alpha, false, false) {
        if cs.conversation.archived {
            continue;
        }
        out.entry(crate::model::bucket_key(c, &cs, &sections)).or_default().push(cs.conversation.id.clone());
    }
    out
}

/// "channels" for channels, "dms" for everything else: the section a conversation falls back to.
fn natural(c: &Client, id: &str) -> &'static str {
    match c.conversations().iter().find(|x| x.conversation.id == id).map(|x| x.conversation.kind) {
        Some(ConversationKind::Channel) => "channels",
        _ => "dms",
    }
}

fn say(wk: &slint::Weak<AppWindow>, msg: &str) {
    let (wk, msg) = (wk.clone(), msg.to_string());
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = wk.upgrade() {
            toast(&ui, &msg);
        }
    });
}

/// Which section a row of the sidebar stands for: a header ("sec:KEY") is that section, a conversation is its own.
fn row_section(c: &Client, id: &str, sort_alpha: bool) -> Option<String> {
    if let Some(k) = id.strip_prefix("sec:") {
        return (!matches!(k, "shortcuts" | "archived")).then(|| k.to_string());
    }
    lists(c, sort_alpha, false, false).into_iter().find(|(_, v)| v.iter().any(|x| x == id)).map(|(k, _)| k)
}

pub fn install(ui: &AppWindow, client: &Client, sh: &Sh) {
    // Drag and drop: `zone` 1 drops before the target row, 2 after it.
    let (wk, c, sh2) = (ui.as_weak(), client.clone(), sh.clone());
    ui.on_reorder_drop(move |src, target, zone| {
        let (src, target, zone) = (src.to_string(), target.to_string(), zone);
        if src == target || target.is_empty() {
            return;
        }
        let alpha = sh2.lock().unwrap().prefs.sort == "alpha";
        let (c2, wk2) = (c.clone(), wk.clone());
        c.handle().spawn(async move {
            if let Some(src_key) = src.strip_prefix("sec:") {
                // A section dragged next to another section (or next to a conversation of one).
                let mut order = c2.section_order();
                let Some(pos) = order.iter().position(|k| k == src_key) else { return };
                let moved = order.remove(pos);
                let to = if target == "sec:shortcuts" {
                    Some(0)
                } else if target == "sec:archived" {
                    Some(order.len())
                } else {
                    row_section(&c2, &target, alpha).and_then(|k| order.iter().position(|x| *x == k)).map(|i| if zone == 2 { i + 1 } else { i })
                };
                let Some(to) = to else { return };
                order.insert(to.min(order.len()), moved);
                if let Err(e) = c2.reorder_sections(&order).await {
                    say(&wk2, &e.to_string());
                }
                return;
            }
            // A conversation: find where it lands.
            let mut all = lists(&c2, alpha, false, false);
            let from = all.iter().find(|(_, v)| v.iter().any(|x| *x == src)).map(|(k, _)| k.clone());
            let Some(from) = from else { return };
            let old_pos = all.get(&from).and_then(|v| v.iter().position(|x| *x == src));
            let (dest, index) = if let Some(k) = target.strip_prefix("sec:") {
                if matches!(k, "shortcuts" | "archived") {
                    return;
                }
                (k.to_string(), 0usize)
            } else {
                let Some(dest) = row_section(&c2, &target, alpha) else { return };
                let list = all.get(&dest).cloned().unwrap_or_default();
                let at = list.iter().position(|x| *x == target).unwrap_or(0);
                (dest, if zone == 2 { at + 1 } else { at })
            };
            if dest != from {
                let nat = natural(&c2, &src);
                let ok = dest == "fav" || dest == nat || !matches!(dest.as_str(), "channels" | "dms");
                if !ok {
                    say(&wk2, if nat == "channels" { "Channels can't go in Direct messages." } else { "Direct messages can't go in Channels." });
                    return;
                }
                let req = match dest.as_str() {
                    "fav" => ConvPrefRequest { favourite: Some(true), ..Default::default() },
                    "channels" | "dms" => ConvPrefRequest { favourite: Some(false), section_id: Some(String::new()), ..Default::default() },
                    sid => ConvPrefRequest { favourite: Some(false), section_id: Some(sid.to_string()), ..Default::default() },
                };
                if let Err(e) = c2.set_pref(&src, &req).await {
                    say(&wk2, &e.to_string());
                    return;
                }
                all = lists(&c2, alpha, false, false);
            }
            // The list may already contain the conversation (it was filed here by set_pref, or it was here all along):
            // take it out and put it at the dropped place. Inside one section the target index counted the old spot.
            let mut list = all.remove(&dest).unwrap_or_default();
            list.retain(|x| *x != src);
            let mut idx = index;
            if dest == from && old_pos.map(|o| o < idx).unwrap_or(false) {
                idx -= 1;
            }
            list.insert(idx.min(list.len()), src.clone());
            if let Err(e) = c2.reorder_conversations(&list).await {
                say(&wk2, &e.to_string());
            }
        });
    });

    // Move up / Move down from a menu: a conversation inside its section, or a section among the sections.
    let (wk, c, sh2) = (ui.as_weak(), client.clone(), sh.clone());
    ui.on_move_row(move |id, delta| {
        let id = id.to_string();
        let alpha = sh2.lock().unwrap().prefs.sort == "alpha";
        let (c2, wk2) = (c.clone(), wk.clone());
        c.handle().spawn(async move {
            if let Some(key) = id.strip_prefix("sec:") {
                let mut order = c2.section_order();
                let Some(i) = order.iter().position(|k| k == key) else { return };
                let j = i as i64 + delta as i64;
                if j < 0 || j >= order.len() as i64 {
                    say(&wk2, if delta < 0 { "This section is already first." } else { "This section is already last." });
                    return;
                }
                order.swap(i, j as usize);
                if let Err(e) = c2.reorder_sections(&order).await {
                    say(&wk2, &e.to_string());
                }
                return;
            }
            let all = lists(&c2, alpha, false, false);
            let Some((_, list)) = all.into_iter().find(|(_, v)| v.iter().any(|x| *x == id)) else { return };
            let mut list = list;
            let i = list.iter().position(|x| *x == id).unwrap_or(0);
            let j = i as i64 + delta as i64;
            if j < 0 || j >= list.len() as i64 {
                say(&wk2, if delta < 0 { "It is already first in its section." } else { "It is already last in its section." });
                return;
            }
            list.swap(i, j as usize);
            if let Err(e) = c2.reorder_conversations(&list).await {
                say(&wk2, &e.to_string());
            }
        });
    });

    // Back to the automatic order (activity or A-Z) and the default section order.
    let (wk, c) = (ui.as_weak(), client.clone());
    ui.on_reset_order(move || {
        let (c2, wk2) = (c.clone(), wk.clone());
        c.handle().spawn(async move {
            let a = c2.reorder_conversations(&[]).await;
            let b = c2.reorder_sections(&[]).await;
            match (a, b) {
                (Ok(_), Ok(_)) => say(&wk2, "Sidebar order reset"),
                (Err(e), _) | (_, Err(e)) => say(&wk2, &e.to_string()),
            }
        });
    });
}
