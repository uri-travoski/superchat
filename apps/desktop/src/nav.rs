//! The one central Back stack. Every way of going "back" (Android's system Back, Esc, Alt+Left, the app bar arrows)
//! ends up in [`pop`]: the topmost popup, sheet, dialog or menu closes first, then the page above it, then the chat,
//! and only when nothing is left does the platform get to leave the app (Android moves the task to the background).
//!
//! The visible state lives in Slint properties. [`NavState`] is a plain snapshot of the ones that matter, so the
//! ordering rules are ordinary Rust that is unit tested without a window.

use crate::AppWindow;
use slint::{ComponentHandle, Weak};
use std::sync::{mpsc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavState {
    pub signed_in: bool,
    /// Two-pane layout: the chat list is always visible, so a chat is not a page of its own.
    pub wide: bool,
    pub show_chat: bool,
    pub has_conv: bool,
    pub info_open: bool,
    pub thread_open: bool,
    /// "", "search", "threads", "scheduled", "people", "switcher" or "settings".
    pub screen: String,
    /// Settings sub-page ("" is the Settings home).
    pub settings_page: String,
    /// Bottom sheet / menu / emoji picker kind ("" = none).
    pub menu_kind: String,
    /// Alert dialog kind ("" = none).
    pub confirm: String,
    pub form_open: bool,
    pub schedule_open: bool,
    pub bookmark_open: bool,
    pub new_channel_open: bool,
    pub tofu_open: bool,
    pub lock_entry_open: bool,
    pub attach_open: bool,
    pub jump_open: bool,
}

/// What one Back press closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Closed {
    Tofu,
    Confirm,
    Form,
    Schedule,
    Bookmark,
    NewChannel,
    Sheet,
    LockEntry,
    AttachBar,
    JumpBar,
    SettingsPage,
    Screen,
    Thread,
    Info,
    Chat,
}

/// Layers from the top down; the first one that is open is closed by Back.
pub fn top(s: &NavState) -> Option<Closed> {
    if !s.signed_in {
        return None;
    }
    let layers: [(bool, Closed); 15] = [
        (s.tofu_open, Closed::Tofu),
        (!s.confirm.is_empty(), Closed::Confirm),
        (s.form_open, Closed::Form),
        (s.schedule_open, Closed::Schedule),
        (s.bookmark_open, Closed::Bookmark),
        (s.new_channel_open, Closed::NewChannel),
        (!s.menu_kind.is_empty(), Closed::Sheet),
        (s.lock_entry_open, Closed::LockEntry),
        (s.attach_open, Closed::AttachBar),
        (s.jump_open, Closed::JumpBar),
        (s.screen == "settings" && !s.settings_page.is_empty(), Closed::SettingsPage),
        (!s.screen.is_empty(), Closed::Screen),
        (s.thread_open, Closed::Thread),
        (s.info_open && s.has_conv, Closed::Info),
        (!s.wide && s.show_chat, Closed::Chat),
    ];
    layers.into_iter().find(|(open, _)| *open).map(|(_, l)| l)
}

/// Closes the topmost layer in `s`. None means the stack is empty (the chat list on a phone, or the login screen).
pub fn pop(s: &mut NavState) -> Option<Closed> {
    let layer = top(s)?;
    match layer {
        Closed::Tofu => s.tofu_open = false,
        Closed::Confirm => s.confirm.clear(),
        Closed::Form => s.form_open = false,
        Closed::Schedule => s.schedule_open = false,
        Closed::Bookmark => s.bookmark_open = false,
        Closed::NewChannel => s.new_channel_open = false,
        Closed::Sheet => s.menu_kind.clear(),
        Closed::LockEntry => s.lock_entry_open = false,
        Closed::AttachBar => s.attach_open = false,
        Closed::JumpBar => s.jump_open = false,
        Closed::SettingsPage => s.settings_page.clear(),
        Closed::Screen => {
            s.screen.clear();
            s.settings_page.clear();
        }
        Closed::Thread => s.thread_open = false,
        Closed::Info => s.info_open = false,
        Closed::Chat => s.show_chat = false,
    }
    Some(layer)
}

pub fn read(ui: &AppWindow) -> NavState {
    NavState {
        signed_in: ui.get_signed_in(),
        wide: ui.get_wide(),
        show_chat: ui.get_show_chat(),
        has_conv: !ui.get_current_conv().is_empty(),
        info_open: ui.get_info_open(),
        thread_open: !ui.get_thread_id().is_empty(),
        screen: ui.get_screen().to_string(),
        settings_page: ui.get_settings_page().to_string(),
        menu_kind: ui.get_menu_kind().to_string(),
        confirm: ui.get_confirm().to_string(),
        form_open: ui.get_form_open(),
        schedule_open: ui.get_schedule_open(),
        bookmark_open: ui.get_bookmark_open(),
        new_channel_open: ui.get_new_channel_open(),
        tofu_open: ui.get_tofu_open(),
        lock_entry_open: ui.get_lock_entry_open(),
        attach_open: ui.get_attach_open(),
        jump_open: ui.get_jump_open(),
    }
}

/// Handles one Back press in the UI. Returns true when something was closed; false means "nothing left to close".
pub fn handle_back(ui: &AppWindow) -> bool {
    let mut s = read(ui);
    let Some(closed) = pop(&mut s) else { return false };
    match closed {
        Closed::Tofu => ui.invoke_tofu_cancel(),
        Closed::Confirm => ui.set_confirm("".into()),
        Closed::Form => ui.set_form_open(false),
        Closed::Schedule => ui.set_schedule_open(false),
        Closed::Bookmark => ui.set_bookmark_open(false),
        Closed::NewChannel => ui.set_new_channel_open(false),
        Closed::Sheet => ui.set_menu_kind("".into()),
        Closed::LockEntry => ui.invoke_lock_cancel(),
        Closed::AttachBar => ui.set_attach_open(false),
        Closed::JumpBar => ui.set_jump_open(false),
        Closed::SettingsPage => ui.set_settings_page("".into()),
        Closed::Screen => {
            ui.set_screen("".into());
            ui.set_settings_page("".into());
        }
        Closed::Thread => ui.invoke_close_thread(),
        Closed::Info => {
            ui.set_info_open(false);
            ui.set_tab(0);
        }
        Closed::Chat => ui.set_show_chat(false),
    }
    true
}

static UI: Mutex<Option<Weak<AppWindow>>> = Mutex::new(None);

pub fn register(ui: &AppWindow) {
    if let Ok(mut g) = UI.lock() {
        *g = Some(ui.as_weak());
    }
}

/// Back from outside the UI thread (Android's system Back arrives on the Java main thread). Blocks briefly for the
/// answer; false (also on timeout) lets the platform leave the app.
pub fn back_from_platform() -> bool {
    let Some(w) = UI.lock().ok().and_then(|g| g.clone()) else { return false };
    let (tx, rx) = mpsc::channel();
    let sent = slint::invoke_from_event_loop(move || {
        let handled = w.upgrade().map(|ui| handle_back(&ui)).unwrap_or(false);
        let _ = tx.send(handled);
    });
    if sent.is_err() {
        return false;
    }
    rx.recv_timeout(Duration::from_millis(600)).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phone() -> NavState {
        NavState { signed_in: true, ..Default::default() }
    }

    fn presses(s: &mut NavState) -> Vec<Closed> {
        let mut v = vec![];
        while let Some(c) = pop(s) {
            v.push(c);
            assert!(v.len() < 20, "Back must make progress");
        }
        v
    }

    #[test]
    fn list_with_nothing_open_exits() {
        let mut s = phone();
        assert_eq!(pop(&mut s), None);
    }

    #[test]
    fn login_screen_exits() {
        let mut s = NavState { signed_in: false, show_chat: true, menu_kind: "conv".into(), ..Default::default() };
        assert_eq!(pop(&mut s), None);
    }

    #[test]
    fn settings_subpage_then_settings_then_list() {
        let mut s = NavState { screen: "settings".into(), settings_page: "appearance".into(), ..phone() };
        assert_eq!(presses(&mut s), vec![Closed::SettingsPage, Closed::Screen]);
        assert!(s.screen.is_empty() && s.settings_page.is_empty());
    }

    #[test]
    fn sheet_over_settings_closes_only_the_sheet() {
        let mut s = NavState { screen: "settings".into(), settings_page: "appearance".into(), menu_kind: "pick-theme".into(), ..phone() };
        assert_eq!(pop(&mut s), Some(Closed::Sheet));
        assert_eq!(s.screen, "settings");
        assert_eq!(s.settings_page, "appearance");
    }

    #[test]
    fn chat_options_sheet_then_chat_then_list() {
        let mut s = NavState { show_chat: true, has_conv: true, menu_kind: "conv".into(), ..phone() };
        assert_eq!(presses(&mut s), vec![Closed::Sheet, Closed::Chat]);
        assert!(!s.show_chat);
    }

    #[test]
    fn dialogs_close_before_sheets_and_pages() {
        let mut s = NavState { show_chat: true, has_conv: true, menu_kind: "msg".into(), confirm: "delete-msg".into(), ..phone() };
        assert_eq!(pop(&mut s), Some(Closed::Confirm));
        assert_eq!(pop(&mut s), Some(Closed::Sheet));
        assert_eq!(pop(&mut s), Some(Closed::Chat));
        assert_eq!(pop(&mut s), None);
    }

    #[test]
    fn form_dialogs_each_close_alone() {
        for (field, want) in [("form", Closed::Form), ("schedule", Closed::Schedule), ("bookmark", Closed::Bookmark), ("channel", Closed::NewChannel), ("tofu", Closed::Tofu)] {
            let mut s = NavState { show_chat: true, has_conv: true, ..phone() };
            match field {
                "form" => s.form_open = true,
                "schedule" => s.schedule_open = true,
                "bookmark" => s.bookmark_open = true,
                "channel" => s.new_channel_open = true,
                _ => s.tofu_open = true,
            }
            assert_eq!(pop(&mut s), Some(want));
            assert_eq!(top(&s), Some(Closed::Chat), "{field}: the page below stays");
        }
    }

    #[test]
    fn thread_info_and_search_pop_in_order() {
        let mut s = NavState { show_chat: true, has_conv: true, info_open: true, ..phone() };
        assert_eq!(presses(&mut s), vec![Closed::Info, Closed::Chat]);
        let mut s = NavState { show_chat: true, has_conv: true, thread_open: true, ..phone() };
        assert_eq!(presses(&mut s), vec![Closed::Thread, Closed::Chat]);
        // Search opened from a chat returns to that chat, then the list.
        let mut s = NavState { show_chat: true, has_conv: true, screen: "search".into(), ..phone() };
        assert_eq!(presses(&mut s), vec![Closed::Screen, Closed::Chat]);
    }

    #[test]
    fn lock_entry_and_inline_bars() {
        let mut s = NavState { lock_entry_open: true, ..phone() };
        assert_eq!(pop(&mut s), Some(Closed::LockEntry));
        let mut s = NavState { show_chat: true, has_conv: true, attach_open: true, jump_open: true, ..phone() };
        assert_eq!(presses(&mut s), vec![Closed::AttachBar, Closed::JumpBar, Closed::Chat]);
    }

    #[test]
    fn wide_layout_has_no_chat_page() {
        // Two-pane: the chat sits next to the list, so with nothing else open Back has nothing to close.
        let mut s = NavState { wide: true, show_chat: true, has_conv: true, ..phone() };
        assert_eq!(pop(&mut s), None);
        // Panels and popups still close in order.
        let mut s = NavState { wide: true, show_chat: true, has_conv: true, thread_open: true, menu_kind: "chat-menu".into(), ..phone() };
        assert_eq!(presses(&mut s), vec![Closed::Sheet, Closed::Thread]);
    }

    #[test]
    fn info_without_a_conversation_is_ignored() {
        let mut s = NavState { info_open: true, ..phone() };
        assert_eq!(pop(&mut s), None);
    }

    #[test]
    fn deep_stack_unwinds_completely() {
        let mut s = NavState {
            show_chat: true,
            has_conv: true,
            info_open: true,
            thread_open: true,
            screen: "settings".into(),
            settings_page: "lock".into(),
            menu_kind: "pick-status".into(),
            confirm: "remove-pin".into(),
            ..phone()
        };
        assert_eq!(presses(&mut s), vec![Closed::Confirm, Closed::Sheet, Closed::SettingsPage, Closed::Screen, Closed::Thread, Closed::Info, Closed::Chat]);
        assert_eq!(pop(&mut s), None);
    }
}
