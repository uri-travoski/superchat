//! App Lock wiring: the PIN screen (own keypad), the menu row, long-press "Lock chat", the settings page and the idle / background timers.
//! The rules (hash, rate limit, timeouts, hiding) live in `client_core::applock`; this file only connects them to the UI.
//! 1.4.1: every change goes through `Client` wrappers that emit `ClientEvent::LockChanged`; the UI re-reads everything at once
//! (no restart, scroll position and tab stay as they were). The slow PIN check runs on a worker thread, never on the UI thread.

use super::{refresh, AppWindow, Sh};
use client_core::applock::{wall_ms, PinError, Unlock, PIN_LEN};
use client_core::Client;
use slint::ComponentHandle;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

static LAST_INPUT_MS: AtomicI64 = AtomicI64::new(0);
static BACKGROUND: AtomicBool = AtomicBool::new(false);
static IDLE_SENT: AtomicBool = AtomicBool::new(false);

/// What the PIN screen is asking for right now.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Flow {
    /// Reveal the locked chats.
    Unlock,
    /// Choose a PIN (first entry, then the confirmation); a chat waiting to be locked is in `Fl::pending`.
    Setup(Option<String>),
    ChangeCur,
    ChangeNew(String),
    ChangeRep(String, String),
    RemoveCur,
}

struct Fl {
    flow: Flow,
    pending: Option<String>,
    busy: bool,
    undo: Option<String>,
    toast_gen: u64,
}

static FL: Mutex<Fl> = Mutex::new(Fl { flow: Flow::Unlock, pending: None, busy: false, undo: None, toast_gen: 0 });

fn fl() -> std::sync::MutexGuard<'static, Fl> {
    FL.lock().unwrap_or_else(|e| e.into_inner())
}

/// Any tap, click, key press or scroll. Platform glue calls this (desktop: window events, Android: `onUserInteraction`).
pub fn note_input() {
    LAST_INPUT_MS.store(wall_ms(), Ordering::Relaxed);
}

/// The app went to the background (`true`) or came back (`false`).
pub fn note_background(background: bool) {
    BACKGROUND.store(background, Ordering::Relaxed);
}

fn texts(flow: &Flow) -> (&'static str, &'static str) {
    match flow {
        Flow::Unlock => ("App Lock", "Enter your PIN"),
        Flow::Setup(None) => ("Set a PIN", "Choose 6 digits. It stays on this device."),
        Flow::Setup(Some(_)) => ("Set a PIN", "Enter it again to confirm"),
        Flow::ChangeCur => ("Change PIN", "Enter your current PIN"),
        Flow::ChangeNew(_) => ("Change PIN", "Choose a new PIN"),
        Flow::ChangeRep(..) => ("Change PIN", "Enter the new PIN again"),
        Flow::RemoveCur => ("Turn off App Lock", "Enter your PIN to turn it off"),
    }
}

fn show_flow(ui: &AppWindow, flow: Flow) {
    let (t, h) = texts(&flow);
    ui.set_lock_title(t.into());
    ui.set_lock_hint(h.into());
    fl().flow = flow;
}

fn open_entry(ui: &AppWindow, flow: Flow, pending: Option<String>) {
    {
        let mut f = fl();
        f.pending = pending;
        f.busy = false;
    }
    show_flow(ui, flow);
    ui.set_lock_pin("".into());
    ui.set_lock_error("".into());
    ui.set_lock_shake(0);
    ui.set_menu_kind("".into());
    ui.set_lock_entry_open(true);
}

fn close_entry(ui: &AppWindow) {
    {
        let mut f = fl();
        f.pending = None;
        f.busy = false;
        f.flow = Flow::Unlock;
    }
    ui.set_lock_entry_open(false);
    ui.set_lock_pin("".into());
    ui.set_lock_error("".into());
    ui.set_lock_shake(0);
}

fn wrong(ui: &AppWindow, msg: &str) {
    ui.set_lock_pin("".into());
    ui.set_lock_error(msg.into());
    ui.set_lock_shake(1);
}

/// A short toast; with `undo` it carries an "Undo" action and stays a little longer.
fn toast_msg(ui: &AppWindow, text: &str, undo: Option<String>) {
    let has_undo = undo.is_some();
    let gen = {
        let mut f = fl();
        f.toast_gen += 1;
        f.undo = undo;
        f.toast_gen
    };
    ui.set_toast(text.into());
    ui.set_toast_undo(has_undo);
    let wk = ui.as_weak();
    slint::Timer::single_shot(Duration::from_millis(if has_undo { 5000 } else { 2500 }), move || {
        if fl().toast_gen != gen {
            return;
        }
        fl().undo = None;
        if let Some(ui) = wk.upgrade() {
            ui.set_toast("".into());
            ui.set_toast_undo(false);
        }
    });
}

fn timeout_label(min: u32) -> &'static str {
    match min {
        0 => "Immediately",
        1 => "1 minute",
        15 => "15 minutes",
        _ => "5 minutes",
    }
}

/// Re-reads the lock state into the UI. When a conversation that is open became hidden, the chat is left gracefully (back to the
/// list, same tab); when the app locks, everything that could show locked content is closed.
pub fn changed(ui: &AppWindow, c: &Client, sh: &Sh) {
    let set = c.applock().is_set();
    let engaged = c.applock().is_engaged();
    ui.set_lock_set(set);
    ui.set_lock_engaged(engaged);
    let min = c.applock().timeout_min();
    ui.set_lock_timeout(min as i32);
    ui.set_lock_timeout_label(timeout_label(min).into());
    if engaged {
        // A PIN screen for unlocking stays; one that was changing the PIN cannot be open while locked.
        if fl().flow != Flow::Unlock {
            close_entry(ui);
        }
        ui.set_confirm("".into());
        ui.set_info_open(false);
        // Search results, thread lists, drafts and the switcher were built while the chats were visible.
        let s = ui.get_screen();
        if !s.is_empty() && s != "settings" {
            ui.set_screen("".into());
        }
        if ui.get_menu_kind() != "" {
            ui.set_menu_kind("".into());
        }
    }
    let cur = sh.lock().unwrap().current.clone();
    if !cur.is_empty() && c.is_hidden(&cur) {
        sh.lock().unwrap().current.clear();
        ui.set_current_conv("".into());
        ui.set_current_title("".into());
        ui.set_current_sub("".into());
        ui.set_show_chat(false);
        ui.set_info_open(false);
        ui.set_thread_id("".into());
        ui.set_composer("".into());
        ui.set_messages(super::model(vec![]));
    }
    refresh(ui, c, sh);
}

/// Runs `work` on a worker thread (Argon2id is slow) and `done` on the UI thread with its result.
fn background<R: Send + 'static>(ui: &AppWindow, c: &Client, sh: &Sh, work: impl FnOnce(&Client) -> R + Send + 'static, done: impl FnOnce(&AppWindow, &Client, &Sh, R) + Send + 'static) {
    fl().busy = true;
    let (wk, c, sh) = (ui.as_weak(), c.clone(), sh.clone());
    std::thread::spawn(move || {
        let r = work(&c);
        let _ = slint::invoke_from_event_loop(move || {
            fl().busy = false;
            if let Some(ui) = wk.upgrade() {
                done(&ui, &c, &sh, r);
            }
        });
    });
}

fn pin_error_text(c: &Client, e: &PinError) -> String {
    match e {
        PinError::Wrong => match c.applock().blocked_secs(wall_ms()) {
            0 => "Incorrect PIN".into(),
            n => PinError::Blocked(n).to_string(),
        },
        other => other.to_string(),
    }
}

fn submit(ui: &AppWindow, c: &Client, sh: &Sh, pin: String) {
    let flow = fl().flow.clone();
    match flow {
        Flow::Unlock => background(
            ui,
            c,
            sh,
            move |c| c.unlock_app(&pin, wall_ms()),
            |ui, c, sh, r| match r {
                Unlock::Ok | Unlock::NotSet => {
                    close_entry(ui);
                    changed(ui, c, sh);
                }
                Unlock::Wrong => wrong(ui, &pin_error_text(c, &PinError::Wrong)),
                Unlock::Blocked(n) => wrong(ui, &PinError::Blocked(n).to_string()),
            },
        ),
        Flow::Setup(None) => {
            ui.set_lock_pin("".into());
            show_flow(ui, Flow::Setup(Some(pin)));
        }
        Flow::Setup(Some(first)) => {
            if first != pin {
                show_flow(ui, Flow::Setup(None));
                wrong(ui, &PinError::Mismatch.to_string());
                return;
            }
            background(
                ui,
                c,
                sh,
                move |c| c.applock().set_pin(&first, &pin, wall_ms()),
                |ui, c, sh, r| match r {
                    Ok(()) => {
                        let pending = fl().pending.take();
                        close_entry(ui);
                        match pending {
                            Some(id) if c.set_conversation_locked(&id, true).is_ok() => toast_msg(ui, "Chat locked", Some(id)),
                            _ => toast_msg(ui, "App Lock is on", None),
                        }
                        changed(ui, c, sh);
                    }
                    Err(e) => {
                        show_flow(ui, Flow::Setup(None));
                        wrong(ui, &e.to_string());
                    }
                },
            );
        }
        Flow::ChangeCur => background(
            ui,
            c,
            sh,
            move |c| (c.applock().verify(&pin, wall_ms()), pin),
            |ui, c, _sh, (r, pin)| match r {
                Unlock::Ok => {
                    ui.set_lock_pin("".into());
                    show_flow(ui, Flow::ChangeNew(pin));
                }
                Unlock::Wrong => wrong(ui, &pin_error_text(c, &PinError::Wrong)),
                Unlock::Blocked(n) => wrong(ui, &PinError::Blocked(n).to_string()),
                Unlock::NotSet => close_entry(ui),
            },
        ),
        Flow::ChangeNew(cur) => {
            ui.set_lock_pin("".into());
            show_flow(ui, Flow::ChangeRep(cur, pin));
        }
        Flow::ChangeRep(cur, new) => {
            if new != pin {
                show_flow(ui, Flow::ChangeNew(cur));
                wrong(ui, &PinError::Mismatch.to_string());
                return;
            }
            background(
                ui,
                c,
                sh,
                move |c| c.applock().change_pin(&cur, &new, &pin, wall_ms()),
                |ui, c, sh, r| match r {
                    Ok(()) => {
                        close_entry(ui);
                        toast_msg(ui, "PIN changed", None);
                        changed(ui, c, sh);
                    }
                    Err(e) => {
                        show_flow(ui, Flow::ChangeCur);
                        wrong(ui, &pin_error_text(c, &e));
                    }
                },
            );
        }
        Flow::RemoveCur => background(
            ui,
            c,
            sh,
            move |c| c.applock().remove_pin(&pin, wall_ms()),
            |ui, c, sh, r| match r {
                Ok(()) => {
                    close_entry(ui);
                    toast_msg(ui, "App Lock is off", None);
                    changed(ui, c, sh);
                }
                Err(e) => wrong(ui, &pin_error_text(c, &e)),
            },
        ),
    }
}

pub fn install(ui: &AppWindow, c: &Client, sh: &Sh) {
    changed(ui, c, sh);

    // The "Lock" / "Unlock" row of the 3-dots menu: locked chats visible -> hide them at once; hidden -> open the PIN screen.
    {
        let (wk, c, sh) = (ui.as_weak(), c.clone(), sh.clone());
        ui.on_lock_tapped(move || {
            let Some(ui) = wk.upgrade() else { return };
            if !c.applock().is_set() {
                return;
            }
            if c.applock().is_engaged() {
                open_entry(&ui, Flow::Unlock, None);
            } else {
                c.lock_app();
                changed(&ui, &c, &sh);
                toast_msg(&ui, "Locked", None);
            }
        });
    }
    {
        let wk = ui.as_weak();
        ui.on_lock_cancel(move || {
            if let Some(ui) = wk.upgrade() {
                close_entry(&ui);
            }
        });
    }
    {
        let (wk, c, sh) = (ui.as_weak(), c.clone(), sh.clone());
        ui.on_lock_key(move |key| {
            let Some(ui) = wk.upgrade() else { return };
            if fl().busy {
                return;
            }
            let mut pin = ui.get_lock_pin().to_string();
            match key.as_str() {
                "del" => {
                    pin.pop();
                }
                k if k.len() == 1 && k.as_bytes()[0].is_ascii_digit() => {
                    if pin.len() >= PIN_LEN {
                        return;
                    }
                    pin.push_str(k);
                }
                _ => return,
            }
            if !ui.get_lock_error().is_empty() && !ui.get_lock_error().starts_with("Too many") {
                ui.set_lock_error("".into());
            }
            ui.set_lock_pin(pin.clone().into());
            if pin.len() == PIN_LEN {
                submit(&ui, &c, &sh, pin);
            }
        });
    }
    // Settings page: turn on / change / turn off start the same PIN screen.
    {
        let (wk, c) = (ui.as_weak(), c.clone());
        ui.on_lock_start(move |kind| {
            let Some(ui) = wk.upgrade() else { return };
            match kind.as_str() {
                "setup" if !c.applock().is_set() => open_entry(&ui, Flow::Setup(None), None),
                "change" if c.applock().is_set() => open_entry(&ui, Flow::ChangeCur, None),
                "remove" if c.applock().is_set() => open_entry(&ui, Flow::RemoveCur, None),
                _ => {}
            }
        });
    }
    {
        let (wk, c, sh) = (ui.as_weak(), c.clone(), sh.clone());
        ui.on_lock_timeout_pick(move |m| {
            let Some(ui) = wk.upgrade() else { return };
            let m = m.max(0) as u32;
            if c.applock().set_timeout_min(m).is_ok() {
                toast_msg(&ui, &format!("Auto-lock: {}", timeout_label(m).to_lowercase()), None);
            }
            changed(&ui, &c, &sh);
        });
    }
    // "Lock chat" (one tap, it disappears, snackbar with Undo) and "Unlock chat (keep visible)". Without a PIN: inline setup first.
    {
        let (wk, c, sh) = (ui.as_weak(), c.clone(), sh.clone());
        ui.on_toggle_conv_lock(move |id| {
            let Some(ui) = wk.upgrade() else { return };
            let id = id.to_string();
            if !c.applock().is_set() {
                open_entry(&ui, Flow::Setup(None), Some(id));
                return;
            }
            let now_locked = !c.is_conv_locked(&id);
            match c.set_conversation_locked(&id, now_locked) {
                Ok(()) => {
                    toast_msg(&ui, if now_locked { "Chat locked" } else { "Chat will stay visible" }, if now_locked { Some(id) } else { None });
                    changed(&ui, &c, &sh);
                }
                Err(e) => toast_msg(&ui, &e.to_string(), None),
            }
        });
    }
    {
        let (wk, c, sh) = (ui.as_weak(), c.clone(), sh.clone());
        ui.on_toast_action(move || {
            let Some(ui) = wk.upgrade() else { return };
            let undo = fl().undo.take();
            ui.set_toast("".into());
            ui.set_toast_undo(false);
            if let Some(id) = undo {
                let _ = c.set_conversation_locked(&id, false);
                changed(&ui, &c, &sh);
            }
        });
    }
    {
        let c = c.clone();
        ui.on_is_conv_locked(move |id| c.is_conv_locked(id.as_str()));
    }
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
    let bg = BACKGROUND.load(Ordering::Relaxed);
    let bg_changed = if bg != *was_background {
        *was_background = bg;
        Some(bg)
    } else {
        None
    };
    if c.lock_timers(bg_changed, now) {
        changed(ui, c, sh);
    }
    if ui.get_lock_entry_open() {
        let n = a.blocked_secs(now);
        let e = ui.get_lock_error();
        if n > 0 && fl().flow == Flow::Unlock {
            ui.set_lock_error(PinError::Blocked(n).to_string().into());
        } else if e.starts_with("Too many") {
            ui.set_lock_error("".into());
        }
    }
}
