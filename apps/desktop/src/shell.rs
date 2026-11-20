//! 1.2 shell: the Slack-style sidebar, DM list, Activity, Later (saved items and labels), details, people picker,
//! search filters and the composer helpers. Pure view-model glue; the rules live in `client_core`.

use super::{model, now_ms, open_thread, people_list, refresh, save_prefs, select_conv, show_message, toast, tz_offset, view, AppWindow, Sh, St};
use crate::model::{label_items, saved_rows, side_rows, SideOpts, LABEL_COLORS};
use crate::{platform, shortcodes};
use client_core::logic::{datefmt, initials};
use client_core::Client;
use proto::types::ConversationKind;
use slint::ComponentHandle;

/// Recomputes everything the shell shows besides the open conversation. Called from `refresh`.
pub fn refresh_shell(ui: &AppWindow, c: &Client, g: &St) {
    let v = view(g);
    let opts = SideOpts {
        sort_alpha: g.prefs.sort == "alpha",
        unread_only: g.prefs.unread_only,
        hide_muted: g.prefs.hide_muted,
        collapsed: &g.prefs.collapsed_for(&c.my_id().unwrap_or_default()),
        drafts: c.all_drafts().iter().filter(|d| !d.body.trim().is_empty() && d.thread_root.is_none()).count(),
    };
    ui.set_side_rows(model(side_rows(c, &opts, &v)));
    ui.set_dm_rows(model(crate::model::dm_rows(c, &v)));
    // Phone tabs: DMs and Channels, pinned first, sorted and filtered by the 3-dot menu's choices.
    ui.set_phone_dms(model(crate::model::phone_rows(c, client_core::ListKind::Dms, opts.sort_alpha, opts.unread_only, opts.hide_muted, &v)));
    ui.set_phone_chans(model(crate::model::phone_rows(c, client_core::ListKind::Channels, opts.sort_alpha, opts.unread_only, opts.hide_muted, &v)));
    let chan_unread: i64 = c.conversations().iter().filter(|cs| cs.conversation.kind == ConversationKind::Channel && !cs.conversation.archived).map(|cs| cs.unread + cs.mentions).sum();
    ui.set_chan_unread(chan_unread as i32);
    let dm_unread: i64 = c.conversations().iter().filter(|cs| cs.conversation.kind != ConversationKind::Channel).map(|cs| cs.unread + cs.mentions).sum();
    ui.set_dm_unread(dm_unread as i32);
    ui.set_activity_unread(c.activity_unread() as i32);
    ui.set_activity_rows(model(crate::model::activity_rows(c, ui.get_activity_filter(), &v)));
    ui.set_saved_rows(model(saved_rows(c, ui.get_label_filter().as_str(), &v)));
    let msg = ui.get_label_msg().to_string();
    let on = if msg.is_empty() { vec![] } else { c.saved_item(&msg).map(|i| i.label_ids).unwrap_or_default() };
    ui.set_labels(model(label_items(c, &on)));
    if let Some(m) = c.server_meta() {
        ui.set_server_initials(initials::initials(&m.name).chars().take(2).collect::<String>().into());
        ui.set_server_name(m.name.into());
    }
    if let Some(me) = c.me() {
        ui.set_presence_i(match me.presence_pref.as_str() {
            "dnd" => 3,
            "away" => 2,
            _ => 1,
        });
    }
    // App Lock management: the list of locked conversations is only shown while they are not hidden.
    let engaged = c.applock().is_engaged();
    let all = c.store().conversations().unwrap_or_default();
    let locked = c.applock().locked_ids();
    let rows = |ids: &[String], want_locked: bool| -> Vec<crate::RowItem> {
        all.iter()
            .filter(|cs| ids.contains(&cs.conversation.id) == want_locked)
            .map(|cs| {
                let t = c.conversation_title(cs);
                model::row(0, &cs.conversation.id, &cs.conversation.id, t, String::new(), String::new())
            })
            .collect()
    };
    ui.set_locked_convs(model(if engaged { vec![] } else { rows(&locked, true) }));
    ui.set_lockable_convs(model(if engaged { vec![] } else { rows(&locked, false) }));
}

/// Details of the open conversation: topic, kind, members, rights.
pub fn refresh_details(ui: &AppWindow, c: &Client, cs: &proto::types::ConversationSummary, me_id: &str, v: &crate::model::View) {
    let conv = &cs.conversation;
    ui.set_current_topic(conv.topic.clone().unwrap_or_default().into());
    ui.set_current_kind(
        match conv.kind {
            ConversationKind::Dm => "dm",
            ConversationKind::SelfNote => "self",
            ConversationKind::GroupDm => "group",
            ConversationKind::Channel => "channel",
        }
        .into(),
    );
    ui.set_current_created(datefmt::format_date(conv.created_ms, v.off).into());
    ui.set_member_count(conv.members.len() as i32);
    ui.set_can_manage(c.can_manage_members(&conv.id));
    let p = c.pref(&conv.id);
    ui.set_current_muted(p.is_muted(v.now));
    ui.set_current_notify(
        match p.notify.as_str() {
            "mentions" => "Mentions only",
            "nothing" => "Nothing",
            _ => "All messages",
        }
        .into(),
    );
    ui.set_head_people(model(
        conv.members
            .iter()
            .filter(|m| m.user_id != me_id || conv.members.len() == 1)
            .take(3)
            .map(|m| {
                let n = c.display_name(&m.user_id);
                crate::PersonItem { id: m.user_id.clone().into(), name: n.clone().into(), initials: initials::initials(&n).into(), color: model::color_of(&m.user_id), sub: Default::default(), sel: false, tag: Default::default() }
            })
            .collect(),
    ));
    if !ui.get_info_open() {
        ui.set_topic_edit(conv.topic.clone().unwrap_or_default().into());
        ui.set_name_edit(c.conversation_title(cs).into());
    }
}

fn conv_title_of(c: &Client, id: &str) -> String {
    c.conversations().into_iter().find(|x| x.conversation.id == id).map(|cs| c.conversation_title(&cs)).unwrap_or_default()
}

fn fill_picker(ui: &AppWindow, c: &Client, sh: &Sh) {
    let (mode, q, sel, cur) = {
        let g = sh.lock().unwrap();
        (ui.get_pp_mode().to_string(), ui.get_pp_text().to_string(), g.pp_sel.clone(), g.current.clone())
    };
    if mode == "channels" {
        // Channel browser: the channels you belong to (channels are private to their members).
        let ql = q.trim().to_lowercase();
        let mut list: Vec<crate::PersonItem> = c
            .conversations()
            .into_iter()
            .filter(|cs| cs.conversation.kind == ConversationKind::Channel && !cs.conversation.archived)
            .filter(|cs| ql.is_empty() || c.conversation_title(cs).to_lowercase().contains(&ql))
            .map(|cs| {
                let mut p = model::person_blank();
                p.id = cs.conversation.id.clone().into();
                p.name = format!("# {}", c.conversation_title(&cs)).into();
                p.initials = "#".into();
                p.color = model::color_of(&cs.conversation.id);
                p.sub = format!("{} member{}", cs.conversation.members.len(), if cs.conversation.members.len() == 1 { "" } else { "s" }).into();
                p.tag = if cs.unread > 0 { format!("{} unread", cs.unread).into() } else { Default::default() };
                p
            })
            .collect();
        list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        ui.set_pp_count(0);
        ui.set_pp_list(model(list));
        return;
    }
    let existing: Vec<String> = if mode == "add-people" {
        c.conversations().into_iter().find(|x| x.conversation.id == cur).map(|cs| cs.conversation.members.iter().map(|m| m.user_id.clone()).collect()).unwrap_or_default()
    } else {
        vec![]
    };
    let mut list = people_list(c, &q);
    list.retain(|p| !existing.contains(&p.id.to_string()) && !c.user(p.id.as_str()).map(|u| u.is_bot).unwrap_or(false));
    for p in list.iter_mut() {
        p.sel = sel.contains(p.id.as_str());
    }
    ui.set_pp_count(sel.len() as i32);
    ui.set_pp_list(model(list));
}

pub fn install(ui: &AppWindow, client: &Client, sh: &Sh) {
    // ---- tabs and sidebar ---------------------------------------------------------------------------------------
    {
        let wk_nav = ui.as_weak();
        ui.on_phone_nav(move |t| {
            if let Some(ui) = wk_nav.upgrade() {
                crate::nav::phone_go(&ui, t.as_str());
            }
        });
        let (wk, c, sh2, sh3) = (ui.as_weak(), client.clone(), sh.clone(), sh.clone());
        ui.on_set_main_tab(move |t| {
            let Some(ui) = wk.upgrade() else { return };
            // Phone: remember the bottom tab for the next start.
            if !ui.get_wide() {
                let tab = ui.get_phone_tab().to_string();
                let mut g = sh3.lock().unwrap();
                if g.prefs.last_tab != tab {
                    g.prefs.last_tab = tab;
                    save_prefs(&g);
                }
            }
            match t.as_str() {
                "search" => {
                    super::prepare_search(&ui, &c, &sh2);
                }
                "activity" => {
                    let (wk2, c2) = (ui.as_weak(), c.clone());
                    c.handle().spawn(async move {
                        let _ = c2.refresh_activity().await;
                        let _ = slint::invoke_from_event_loop(move || {
                            let _ = wk2;
                        });
                    });
                }
                "later" => {
                    let c2 = c.clone();
                    c.handle().spawn(async move {
                        let _ = c2.refresh_saved().await;
                        c2.load_saved_messages().await;
                    });
                }
                _ => {}
            }
            refresh(&ui, &c, &sh2);
        });
        let (wk, c, sh2) = (ui.as_weak(), client.clone(), sh.clone());
        ui.on_side_clicked(move |id, kind| {
            let Some(ui) = wk.upgrade() else { return };
            if kind.as_str() == "link" {
                match id.as_str() {
                    "threads" => ui.invoke_open_threads(),
                    "drafts" => ui.invoke_open_drafts(),
                    "later" => ui.invoke_go_home_tab("later".into()),
                    _ => {}
                }
                return;
            }
            select_conv(&ui, &c, &sh2, &id, true);
        });
        let (wk, c, sh2) = (ui.as_weak(), client.clone(), sh.clone());
        ui.on_side_toggle(move |id| {
            let Some(ui) = wk.upgrade() else { return };
            if let Some(sid) = id.strip_prefix("sec:").filter(|s| !matches!(*s, "fav" | "channels" | "dms" | "shortcuts" | "archived")) {
                let collapsed = c.sections().into_iter().find(|s| s.id == sid).map(|s| s.collapsed).unwrap_or(false);
                let (c2, sid) = (c.clone(), sid.to_string());
                c.handle().spawn(async move {
                    let _ = c2.update_section(&sid, &proto::types::SectionRequest { collapsed: Some(!collapsed), ..Default::default() }).await;
                });
                return;
            }
            let mut g = sh2.lock().unwrap();
            g.prefs.toggle_collapsed(&c.my_id().unwrap_or_default(), id.as_str());
            save_prefs(&g);
            drop(g);
            refresh(&ui, &c, &sh2);
        });
        let wk = ui.as_weak();
        let c = client.clone();
        ui.on_side_plus(move |kind| {
            let Some(ui) = wk.upgrade() else { return };
            match kind.as_str() {
                "dm" => ui.invoke_new_message(),
                _ => {
                    if c.me().map(|m| m.user.role.is_admin()).unwrap_or(false) {
                        ui.set_new_channel_name("".into());
                        ui.set_new_channel_open(true);
                    } else {
                        toast(&ui, "Channels are created by admins. Ask an admin to add you to one.");
                    }
                }
            }
        });
        let (wk, c, sh2) = (ui.as_weak(), client.clone(), sh.clone());
        ui.on_select_conv_step(move |d| {
            let Some(ui) = wk.upgrade() else { return };
            let list = c.sorted_conversations(sh2.lock().unwrap().prefs.sort == "alpha", false, false);
            if list.is_empty() {
                return;
            }
            let cur = sh2.lock().unwrap().current.clone();
            let i = list.iter().position(|x| x.conversation.id == cur).map(|i| i as i32 + d).unwrap_or(0).clamp(0, list.len() as i32 - 1) as usize;
            select_conv(&ui, &c, &sh2, &list[i].conversation.id, true);
        });
        let (c, sh2, wk) = (client.clone(), sh.clone(), ui.as_weak());
        ui.on_mark_read(move |id| {
            let (c2, id) = (c.clone(), id.to_string());
            c.handle().spawn(async move {
                let _ = c2.mark_read(&id).await;
            });
            if let Some(ui) = wk.upgrade() {
                refresh(&ui, &c, &sh2);
            }
        });
    }

    // ---- Activity ---------------------------------------------------------------------------------------------
    {
        let (wk, c, sh2) = (ui.as_weak(), client.clone(), sh.clone());
        ui.on_activity_filter_changed(move |_| {
            if let Some(ui) = wk.upgrade() {
                refresh(&ui, &c, &sh2);
            }
        });
        let c = client.clone();
        ui.on_mark_activity_read(move || {
            let c2 = c.clone();
            c.handle().spawn(async move {
                let _ = c2.mark_activity_read().await;
            });
        });
        let (wk, c, sh2) = (ui.as_weak(), client.clone(), sh.clone());
        ui.on_activity_clicked(move |conv, msg, root, _id| {
            let Some(ui) = wk.upgrade() else { return };
            ui.set_screen("".into());
            if !root.is_empty() {
                select_conv(&ui, &c, &sh2, &conv, true);
                open_thread(&ui, &c, &sh2, &conv, &root);
            } else {
                show_message(&ui, &c, &sh2, &conv, &msg);
            }
        });
    }

    // ---- Later: saved items and labels ----------------------------------------------------------------------------
    {
        let (wk, c, sh2) = (ui.as_weak(), client.clone(), sh.clone());
        ui.on_later_clicked(move |conv, id| {
            if let Some(ui) = wk.upgrade() {
                ui.set_screen("".into());
                show_message(&ui, &c, &sh2, &conv, &id);
            }
        });
        let (wk, c) = (ui.as_weak(), client.clone());
        ui.on_save_msg(move |id| {
            let (wk, c2, id) = (wk.clone(), c.clone(), id.to_string());
            let saved = c.is_saved(&id);
            c.handle().spawn(async move {
                let r = if saved { c2.unsave_message(&id).await.map(|_| "Removed from saved items") } else { c2.save_message(&id, None).await.map(|_| "Saved for later. Find it under Later.") };
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = wk.upgrade() {
                        toast(&ui, &match r {
                            Ok(m) => m.to_string(),
                            Err(e) => format!("Could not save: {e}"),
                        });
                    }
                });
            });
        });
        let (wk, c) = (ui.as_weak(), client.clone());
        ui.on_unsave(move |id| {
            let _ = &wk;
            let (c2, id) = (c.clone(), id.to_string());
            c.handle().spawn(async move {
                let _ = c2.unsave_message(&id).await;
            });
        });
        let (wk, c, sh2) = (ui.as_weak(), client.clone(), sh.clone());
        ui.on_label_picker(move |id| {
            let (wk2, c2, sh3, id) = (wk.clone(), c.clone(), sh2.clone(), id.to_string());
            c.handle().spawn(async move {
                // Adding a label saves the message first.
                if !c2.is_saved(&id) {
                    let _ = c2.save_message(&id, None).await;
                }
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = wk2.upgrade() {
                        ui.set_label_msg(id.into());
                        ui.set_label_open(true);
                        refresh(&ui, &c2, &sh3);
                    }
                });
            });
        });
        let c = client.clone();
        ui.on_label_toggle(move |msg, label| {
            let (c2, msg, label) = (c.clone(), msg.to_string(), label.to_string());
            c.handle().spawn(async move {
                let mut ids = c2.saved_item(&msg).map(|i| i.label_ids).unwrap_or_default();
                if let Some(p) = ids.iter().position(|x| *x == label) {
                    ids.remove(p);
                } else {
                    ids.push(label);
                }
                let _ = c2.set_saved_labels(&msg, &ids).await;
            });
        });
        let (wk, c) = (ui.as_weak(), client.clone());
        ui.on_label_create(move |name, color| {
            let (wk2, c2, name) = (wk.clone(), c.clone(), name.trim().to_string());
            let color = LABEL_COLORS[(color.max(0) as usize).min(5)].to_string();
            let msg = wk.upgrade().map(|u| u.get_label_msg().to_string()).unwrap_or_default();
            c.handle().spawn(async move {
                let r = c2.create_label(&name, &color).await;
                if let (Ok(list), false) = (&r, msg.is_empty()) {
                    if let Some(l) = list.iter().find(|l| l.name.eq_ignore_ascii_case(&name)) {
                        let mut ids = c2.saved_item(&msg).map(|i| i.label_ids).unwrap_or_default();
                        ids.push(l.id.clone());
                        let _ = c2.set_saved_labels(&msg, &ids).await;
                    }
                }
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = wk2.upgrade() {
                        match r {
                            Ok(_) => ui.set_label_new_name("".into()),
                            Err(e) => toast(&ui, &e.to_string()),
                        }
                    }
                });
            });
        });
        let (wk, c) = (ui.as_weak(), client.clone());
        ui.on_label_rename(move |id, name| {
            let (wk2, c2, id, name) = (wk.clone(), c.clone(), id.to_string(), name.trim().to_string());
            c.handle().spawn(async move {
                if let Err(e) = c2.update_label(&id, Some(&name), None).await {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = wk2.upgrade() {
                            toast(&ui, &e.to_string());
                        }
                    });
                }
            });
        });
        let c = client.clone();
        ui.on_label_recolor(move |id| {
            let (c2, id) = (c.clone(), id.to_string());
            let cur = c.labels().into_iter().find(|l| l.id == id).map(|l| l.color).unwrap_or_default();
            let next = LABEL_COLORS.iter().position(|x| x.eq_ignore_ascii_case(&cur)).map(|i| (i + 1) % LABEL_COLORS.len()).unwrap_or(0);
            c.handle().spawn(async move {
                let _ = c2.update_label(&id, None, Some(LABEL_COLORS[next])).await;
            });
        });
        let (wk, c) = (ui.as_weak(), client.clone());
        ui.on_label_delete(move |id| {
            let (c2, id) = (c.clone(), id.to_string());
            if let Some(ui) = wk.upgrade() {
                if ui.get_label_filter().as_str() == id {
                    ui.set_label_filter("".into());
                }
            }
            c.handle().spawn(async move {
                let _ = c2.delete_label(&id).await;
            });
        });
        let (wk, c, sh2) = (ui.as_weak(), client.clone(), sh.clone());
        ui.on_label_filter_changed(move |_| {
            if let Some(ui) = wk.upgrade() {
                refresh(&ui, &c, &sh2);
            }
        });
    }

    // ---- people picker, membership --------------------------------------------------------------------------------
    {
        let (wk, c, sh2) = (ui.as_weak(), client.clone(), sh.clone());
        ui.on_pp_open_for(move |_mode| {
            let Some(ui) = wk.upgrade() else { return };
            sh2.lock().unwrap().pp_sel.clear();
            fill_picker(&ui, &c, &sh2);
        });
        let (wk, c, sh2) = (ui.as_weak(), client.clone(), sh.clone());
        ui.on_pp_filter(move |_| {
            if let Some(ui) = wk.upgrade() {
                fill_picker(&ui, &c, &sh2);
            }
        });
        let (wk, c, sh2) = (ui.as_weak(), client.clone(), sh.clone());
        ui.on_pp_toggle(move |id| {
            let Some(ui) = wk.upgrade() else { return };
            match ui.get_pp_mode().as_str() {
                "new-message" | "directory" => {
                    ui.set_pp_open(false);
                    ui.invoke_start_dm(id);
                    return;
                }
                "channels" => {
                    ui.set_pp_open(false);
                    select_conv(&ui, &c, &sh2, &id, true);
                    ui.set_show_chat(true);
                    return;
                }
                _ => {}
            }
            {
                let mut g = sh2.lock().unwrap();
                let id = id.to_string();
                if !g.pp_sel.remove(&id) {
                    g.pp_sel.insert(id);
                }
            }
            fill_picker(&ui, &c, &sh2);
        });
        let (wk, c, sh2) = (ui.as_weak(), client.clone(), sh.clone());
        ui.on_pp_confirm(move || {
            let Some(ui) = wk.upgrade() else { return };
            let (cur, ids): (String, Vec<String>) = {
                let g = sh2.lock().unwrap();
                (g.current.clone(), g.pp_sel.iter().cloned().collect())
            };
            if ids.is_empty() || cur.is_empty() {
                return;
            }
            ui.set_pp_open(false);
            let (wk2, c2, sh3) = (ui.as_weak(), c.clone(), sh2.clone());
            c.handle().spawn(async move {
                let r = c2.add_members(&cur, &ids).await;
                let n = ids.len();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = wk2.upgrade() {
                        match r {
                            Ok(_) => toast(&ui, &if n == 1 { "Added 1 person".to_string() } else { format!("Added {n} people") }),
                            Err(e) => toast(&ui, &format!("Could not add people: {e}")),
                        }
                        refresh(&ui, &c2, &sh3);
                    }
                });
            });
        });
        let (wk, c, sh2) = (ui.as_weak(), client.clone(), sh.clone());
        ui.on_remove_member(move |uid| {
            let cur = sh2.lock().unwrap().current.clone();
            let (wk2, c2, sh3, uid) = (wk.clone(), c.clone(), sh2.clone(), uid.to_string());
            c.handle().spawn(async move {
                let r = c2.remove_member(&cur, &uid).await;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = wk2.upgrade() {
                        match r {
                            Ok(_) => toast(&ui, "Removed"),
                            Err(e) => toast(&ui, &format!("Could not remove: {e}")),
                        }
                        refresh(&ui, &c2, &sh3);
                    }
                });
            });
        });
        let (wk, c, sh2) = (ui.as_weak(), client.clone(), sh.clone());
        ui.on_save_topic(move |t| {
            let cur = sh2.lock().unwrap().current.clone();
            let (wk2, c2, t) = (wk.clone(), c.clone(), t.trim().to_string());
            c.handle().spawn(async move {
                let r = c2.set_topic(&cur, &t).await;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = wk2.upgrade() {
                        toast(&ui, &match r {
                            Ok(()) => "Topic saved".to_string(),
                            Err(e) => format!("Could not save the topic: {e}"),
                        });
                    }
                });
            });
        });
        let (wk, c, sh2) = (ui.as_weak(), client.clone(), sh.clone());
        ui.on_rename_conv(move |n| {
            let cur = sh2.lock().unwrap().current.clone();
            let (wk2, c2, n) = (wk.clone(), c.clone(), n.trim().to_string());
            if n.is_empty() {
                return;
            }
            c.handle().spawn(async move {
                let r = c2.rename_conversation(&cur, &n).await;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = wk2.upgrade() {
                        toast(&ui, &match r {
                            Ok(()) => "Renamed".to_string(),
                            Err(e) => format!("Could not rename: {e}"),
                        });
                    }
                });
            });
        });
        let (wk, c, sh2) = (ui.as_weak(), client.clone(), sh.clone());
        ui.on_archive_toggle(move || {
            let cur = sh2.lock().unwrap().current.clone();
            let archived = c.conversations().iter().any(|x| x.conversation.id == cur && x.conversation.archived);
            let (wk2, c2) = (wk.clone(), c.clone());
            c.handle().spawn(async move {
                let r = c2.set_archived(&cur, !archived).await;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = wk2.upgrade() {
                        toast(&ui, &match r {
                            Ok(()) if archived => "Unarchived".to_string(),
                            Ok(()) => "Archived. It moved to the Archived section.".to_string(),
                            Err(e) => format!("Could not change it: {e}"),
                        });
                    }
                });
            });
        });
        let (wk, c) = (ui.as_weak(), client.clone());
        ui.on_open_details_data(move || {
            let _ = (&wk, &c);
        });
    }

    // ---- composer helpers, message extras ---------------------------------------------------------------------------
    {
        let c = client.clone();
        let wk = ui.as_weak();
        ui.on_copy_text(move |id| {
            let Some(ui) = wk.upgrade() else { return };
            if let Some(m) = c.store().message(&id).ok().flatten() {
                let ok = platform::copy_text(&model::plain(&m.body));
                toast(&ui, if ok { "Text copied." } else { "No clipboard tool found (install wl-clipboard or xclip)." });
            }
        });
        let wk = ui.as_weak();
        ui.on_emoji_pick_sugg(move |e| {
            let Some(ui) = wk.upgrade() else { return };
            ui.set_composer(shortcodes::complete(ui.get_composer().as_str(), e.as_str()).into());
            ui.set_emoji_sugg(model(vec![]));
        });
        let (wk, c) = (ui.as_weak(), client.clone());
        ui.on_quote_reply(move |id| {
            let Some(ui) = wk.upgrade() else { return };
            if let Some(m) = c.store().message(&id).ok().flatten() {
                let t: String = model::plain(&m.body).split_whitespace().collect::<Vec<_>>().join(" ");
                ui.set_quote_text(format!("Replying to {}: {}", c.display_name(&m.sender_id), t).into());
                ui.set_quote_id(id);
            }
        });
        let (wk, c, sh2) = (ui.as_weak(), client.clone(), sh.clone());
        ui.on_up_edit(move || {
            let Some(ui) = wk.upgrade() else { return };
            let cur = sh2.lock().unwrap().current.clone();
            let me = c.my_id().unwrap_or_default();
            if let Some(t) = c.timeline(&cur, 60, None).into_iter().rev().find(|t| t.message.sender_id == me && t.message.thread_root.is_none() && !t.local && c.can_edit(&t.message)) {
                ui.invoke_start_edit(t.message.id.into());
            }
        });
        let (wk, c) = (ui.as_weak(), client.clone());
        ui.on_open_admin_panel(move || {
            let Some(ui) = wk.upgrade() else { return };
            match c.session().map(|s| s.server) {
                Some(s) => {
                    platform::open_url(&format!("{}/admin/", s.trim_end_matches('/')));
                    toast(&ui, "Opening the admin panel in your browser.");
                }
                None => toast(&ui, "Not signed in."),
            }
        });
    }

    // ---- search filters ---------------------------------------------------------------------------------------------
    {
        let (wk, c) = (ui.as_weak(), client.clone());
        ui.on_search_filter(move |kind, value| {
            let Some(ui) = wk.upgrade() else { return };
            let off = tz_offset();
            let token = match kind.as_str() {
                "from" => c.user(&value).map(|u| format!("from:{}", u.username)).unwrap_or_default(),
                "in" => {
                    let t = conv_title_of(&c, &value);
                    if t.is_empty() { String::new() } else { format!("in:{}", t.replace(' ', "-")) }
                }
                "has" => format!("has:{value}"),
                "date" => {
                    let now = now_ms();
                    let day = |d: i64| datefmt::format_date(now - d * 86_400_000, off);
                    match value.as_str() {
                        "today" => format!("after:{}", day(0)),
                        "yesterday" => format!("after:{}", day(1)),
                        "7d" => format!("after:{}", day(7)),
                        _ => format!("after:{}", day(30)),
                    }
                }
                _ => String::new(),
            };
            if token.is_empty() {
                return;
            }
            let cur = ui.get_search_text().to_string();
            let text = if cur.is_empty() { token } else { format!("{cur} {token}") };
            ui.set_search_text(text.clone().into());
            ui.invoke_search(text.into());
        });
    }

    // ---- App Lock management ------------------------------------------------------------------------------------------
    {
        let (wk, c, sh2) = (ui.as_weak(), client.clone(), sh.clone());
        ui.on_lock_add(move |id| {
            let Some(ui) = wk.upgrade() else { return };
            let _ = c.applock().set_conv_locked(&id, true);
            crate::lock::changed(&ui, &c, &sh2);
        });
        let (wk, c, sh2) = (ui.as_weak(), client.clone(), sh.clone());
        ui.on_lock_remove_conv(move |id| {
            let Some(ui) = wk.upgrade() else { return };
            let _ = c.applock().set_conv_locked(&id, false);
            crate::lock::changed(&ui, &c, &sh2);
        });
    }
}
