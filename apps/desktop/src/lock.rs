//! App Lock wiring: the padlock, the PIN screen, the settings section and the idle / background timers.
//! The rules (hash, rate limit, timeouts, hiding) live in `client_core::applock`; this file only connects them to the UI.

use super::{refresh, toast, AppWindow, Sh};
use client_core::applock::{wall_ms, PinError, Unlock};
use client_core::Client;
use slint::ComponentHandle;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

static LAST_INPUT_MS: AtomicI64 = AtomicI64::new(0);
static BACKGROUND: AtomicBool = AtomicBool::new(false);
static IDLE_SENT: AtomicBool = AtomicBool::new(false);

/// Any tap, click, key press or scroll. Platform glue calls this (desktop: window events, Android: `onUserInteraction`).
pub fn note_input() {
    LAST_INPUT_MS.store(wall_ms(), Ordering::Relaxed);
}

/// The app went to the background (`true`) or came back (`false`).
pub fn note_background(background: bool) {
    BACKGROUND.store(background, Ordering::Relaxed);
}

fn digits(t: &str) -> String {
    t.chars().filter(|c| c.is_ascii_digit()).take(client_core::applock::PIN_LEN).collect()
}

fn pin_msg(e: &PinError) -> String {
    e.to_string()
}

/// Re-reads the lock state into the UI. When the lock just engaged, everything that could show a locked chat is closed.
pub fn changed(ui: &AppWindow, c: &Client, sh: &Sh) {
    let set = c.applock().is_set();
    let engaged = c.applock().is_engaged();
    ui.set_lock_set(set);
    ui.set_lock_engaged(engaged);
    if engaged {
        ui.set_lock_entry_open(false);
        ui.set_lock_pin("".into());
        ui.set_info_open(false);
        ui.set_confirm("".into());
        let cur = sh.lock().unwrap().current.clone();
        if !cur.is_empty() && c.is_hidden(&cur) {
            sh.lock().unwrap().current.clear();
            ui.set_current_conv("".into());
            ui.set_current_title("".into());
            ui.set_current_sub("".into());
            ui.set_show_chat(false);
            ui.set_thread_id("".into());
            ui.set_composer("".into());
            ui.set_messages(super::model(vec![]));
        }
        // Search results, thread lists, drafts and the switcher were built while the chats were visible.
        let s = ui.get_screen();
        if !s.is_empty() && s != "settings" {
            ui.set_screen("".into());
        }
        if ui.get_menu_kind() != "" {
            ui.set_menu_kind("".into());
        }
    }
    refresh(ui, c, sh);
}

pub fn install(ui: &AppWindow, c: &Client, sh: &Sh) {
    changed(ui, c, sh);

    {
        let (wk, c, sh) = (ui.as_weak(), c.clone(), sh.clone());
        ui.on_lock_tapped(move || {
            let Some(ui) = wk.upgrade() else { return };
            if !c.applock().is_set() {
                return;
            }
            if c.applock().is_engaged() {
                ui.set_lock_pin("".into());
                ui.set_lock_error("".into());
                ui.set_lock_entry_open(true);
            } else {
                c.applock().lock();
                changed(&ui, &c, &sh);
            }
        });
    }
    {
        let wk = ui.as_weak();
        ui.on_lock_cancel(move || {
            if let Some(ui) = wk.upgrade() {
                ui.set_lock_entry_open(false);
                ui.set_lock_pin("".into());
                ui.set_lock_error("".into());
            }
        });
    }
    {
        let (wk, c, sh) = (ui.as_weak(), c.clone(), sh.clone());
        ui.on_pin_edited(move |field, text| {
            let Some(ui) = wk.upgrade() else { return };
            let d = digits(text.as_str());
            match field.as_str() {
                "cur" => ui.set_lock_cur(d.into()),
                "new" => ui.set_lock_new(d.into()),
                "new2" => ui.set_lock_new2(d.into()),
                _ => {
                    ui.set_lock_pin(d.clone().into());
                    if d.is_empty() {
                        return;
                    }
                    if d.len() < client_core::applock::PIN_LEN {
                        if ui.get_lock_error().as_str() == "Incorrect PIN" {
                            ui.set_lock_error("".into());
                        }
                        return;
                    }
                    match c.applock().unlock(&d, wall_ms()) {
                        Unlock::Ok => {
                            ui.set_lock_entry_open(false);
                            ui.set_lock_error("".into());
                            ui.set_lock_pin("".into());
                            changed(&ui, &c, &sh);
                        }
                        Unlock::Wrong => {
                            ui.set_lock_pin("".into());
                            ui.set_lock_error(match c.applock().blocked_secs(wall_ms()) {
                                0 => "Incorrect PIN".into(),
                                n => PinError::Blocked(n).to_string().into(),
                            });
                        }
                        Unlock::Blocked(n) => {
                            ui.set_lock_pin("".into());
                            ui.set_lock_error(PinError::Blocked(n).to_string().into());
                        }
                        Unlock::NotSet => {
                            ui.set_lock_entry_open(false);
                            changed(&ui, &c, &sh);
                        }
                    }
                }
            }
        });
    }
    {
        let (wk, c, sh) = (ui.as_weak(), c.clone(), sh.clone());
        ui.on_lock_enable(move || {
            let Some(ui) = wk.upgrade() else { return };
            match c.applock().set_pin(ui.get_lock_new().as_str(), ui.get_lock_new2().as_str(), wall_ms()) {
                Ok(()) => {
                    clear_fields(&ui);
                    ui.set_lock_msg("App Lock is on. Open a conversation's options and choose Lock this conversation.".into());
                    changed(&ui, &c, &sh);
                }
                Err(e) => ui.set_lock_msg(pin_msg(&e).into()),
            }
        });
    }
    {
        let (wk, c, sh) = (ui.as_weak(), c.clone(), sh.clone());
        ui.on_lock_change(move || {
            let Some(ui) = wk.upgrade() else { return };
            match c.applock().change_pin(ui.get_lock_cur().as_str(), ui.get_lock_new().as_str(), ui.get_lock_new2().as_str(), wall_ms()) {
                Ok(()) => {
                    clear_fields(&ui);
                    ui.set_lock_msg("PIN changed.".into());
                    changed(&ui, &c, &sh);
                }
                Err(e) => ui.set_lock_msg(pin_msg(&e).into()),
            }
        });
    }
    {
        let (wk, c, sh) = (ui.as_weak(), c.clone(), sh.clone());
        ui.on_lock_remove(move || {
            let Some(ui) = wk.upgrade() else { return };
            match c.applock().remove_pin(ui.get_lock_cur().as_str(), wall_ms()) {
                Ok(()) => {
                    clear_fields(&ui);
                    ui.set_lock_msg("App Lock is off. All conversations are shown again.".into());
                    changed(&ui, &c, &sh);
                }
                Err(e) => ui.set_lock_msg(pin_msg(&e).into()),
            }
        });
    }
    {
        let (wk, c, sh) = (ui.as_weak(), c.clone(), sh.clone());
        ui.on_lock_now(move || {
            let Some(ui) = wk.upgrade() else { return };
            c.applock().lock();
            clear_fields(&ui);
            changed(&ui, &c, &sh);
        });
    }
    {
        let (wk, c, sh) = (ui.as_weak(), c.clone(), sh.clone());
        ui.on_toggle_conv_lock(move |id| {
            let Some(ui) = wk.upgrade() else { return };
            let now_locked = !c.is_conv_locked(id.as_str());
            match c.applock().set_conv_locked(id.as_str(), now_locked) {
                Ok(()) => {
                    toast(&ui, if now_locked { "Locked. It is hidden whenever App Lock is on." } else { "Lock removed from this conversation." });
                    refresh(&ui, &c, &sh);
                }
                Err(e) => toast(&ui, &pin_msg(&e)),
            }
        });
    }
    {
        let c = c.clone();
        ui.on_is_conv_locked(move |id| c.is_conv_locked(id.as_str()));
    }
}

fn clear_fields(ui: &AppWindow) {
    ui.set_lock_cur("".into());
    ui.set_lock_new("".into());
    ui.set_lock_new2("".into());
}

/// Called twice a second: applies platform input / background signals and the idle timeout, counts down a block.
pub fn tick(ui: &AppWindow, c: &Client, sh: &Sh, was_background: &mut bool) {
    // P-02 auto-away: no input for 10 minutes (desktop) or the app in the background (phone) reports "idle".
    {
        let now = wall_ms();
        let li = LAST_INPUT_MS.load(Ordering::Relaxed);
        let idle = (cfg!(target_os = "android") && BACKGROUND.load(Ordering::Relaxed)) || (li > 0 && now - li > 10 * 60 * 1000);
        if IDLE_SENT.swap(idle, Ordering::Relaxed) != idle {
            c.set_idle(idle);
        }
    }
    let a = c.applock();
    if !a.is_set() {
        return;
    }
    let now = wall_ms();
    let li = LAST_INPUT_MS.load(Ordering::Relaxed);
    if li > 0 {
        a.activity(li);
    }
    let mut locked = false;
    let bg = BACKGROUND.load(Ordering::Relaxed);
    if bg != *was_background {
        *was_background = bg;
        locked |= a.set_background(bg, now);
    }
    locked |= a.check_idle(now);
    if locked {
        changed(ui, c, sh);
    }
    if ui.get_lock_entry_open() {
        let n = a.blocked_secs(now);
        let e = ui.get_lock_error();
        if n > 0 {
            ui.set_lock_error(PinError::Blocked(n).to_string().into());
        } else if e.starts_with("Too many") {
            ui.set_lock_error("".into());
        }
    }
}
