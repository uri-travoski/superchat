//! E-05: spell check of the message boxes on the desktop. The words come from `client_core::spell` (a Hunspell-compatible
//! checker with a bundled Australian English dictionary and the system's dictionaries). Slint cannot underline part of
//! a text field, so each typo's place is measured with a hidden copy of the text in the window (`probe`), and the
//! squiggle is drawn over the field. Right-clicking a marked word opens a menu with suggestions, "Add to dictionary" and
//! "Ignore for now". On Android the keyboard does the checking, so nothing is drawn there.

use super::{AppWindow, Sh};
use client_core::spell::{self, Speller};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone)]
struct Mark {
    start: usize,
    end: usize,
    x: f32,
    y: f32,
    w: f32,
}

#[derive(Clone)]
struct Target {
    which: String,
    start: usize,
    end: usize,
    word: String,
    suggestions: Vec<String>,
}

#[derive(Default)]
struct State {
    marks: HashMap<String, Vec<Mark>>,
    last: HashMap<String, (String, f32)>,
    target: Option<Target>,
    ignore: HashSet<String>,
    probes: Rc<RefCell<Vec<(i32, f32, f32)>>>,
    timers: HashMap<String, slint::Timer>,
    langs: Vec<(String, String)>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

type Shared = Arc<Mutex<Option<Speller>>>;

/// Squiggle path for a mark `w` px wide (a 3 px tall zigzag).
fn zigzag(w: f32) -> String {
    let mut p = String::from("M0 2.2");
    let mut x = 0.0;
    let mut up = true;
    while x < w {
        x = (x + 2.5).min(w + 2.5);
        p.push_str(&format!(" L{:.1} {}", x, if up { 0.6 } else { 2.2 }));
        up = !up;
    }
    p
}

fn set_marks(ui: &AppWindow, which: &str, marks: &[Mark]) {
    let items: Vec<crate::SpellMark> = marks.iter().map(|m| crate::SpellMark { x: m.x, y: m.y, w: m.w, path: zigzag(m.w).into() }).collect();
    let model = ModelRc::new(VecModel::from(items));
    if which == "thread" {
        ui.set_thread_spell_marks(model);
    } else {
        ui.set_spell_marks(model);
    }
}

fn compute(ui: &AppWindow, sp: &Shared, which: &str, text: &str, width: f32) {
    let enabled = ui.get_spell_on() && !ui.get_is_android();
    let ranges = if enabled && !text.trim().is_empty() {
        let g = sp.lock().unwrap();
        match g.as_ref() {
            Some(s) => {
                let ignore = STATE.with(|st| st.borrow().ignore.clone());
                s.misspelled(text).into_iter().filter(|m| !ignore.contains(&text[m.start..m.end].to_lowercase())).take(24).collect::<Vec<_>>()
            }
            None => vec![],
        }
    } else {
        vec![]
    };
    if ranges.is_empty() {
        STATE.with(|st| st.borrow_mut().marks.remove(which));
        set_marks(ui, which, &[]);
        return;
    }
    // Measure: put the text into the hidden probe field (same width and font) and ask where each word starts and ends.
    ui.set_probe_text(text.into());
    ui.set_probe_width(width);
    let probes = STATE.with(|st| st.borrow().probes.clone());
    probes.borrow_mut().clear();
    for r in &ranges {
        ui.invoke_probe(r.start as i32);
        ui.invoke_probe(r.end as i32);
    }
    let got = probes.borrow().clone();
    let at = |off: usize| got.iter().find(|(o, _, _)| *o == off as i32).map(|(_, x, y)| (*x, *y));
    let mut marks = vec![];
    for r in ranges {
        if let (Some((sx, sy)), Some((ex, ey))) = (at(r.start), at(r.end)) {
            // A word that wraps onto the next line is left unmarked (rare).
            if (sy - ey).abs() < 2.0 && ex > sx {
                marks.push(Mark { start: r.start, end: r.end, x: sx, y: sy, w: ex - sx });
            }
        }
    }
    set_marks(ui, which, &marks);
    STATE.with(|st| st.borrow_mut().marks.insert(which.to_string(), marks));
}

fn lang_label(tag: &str, langs: &[(String, String)]) -> String {
    langs.iter().find(|(t, _)| t == tag).map(|(_, l)| l.clone()).unwrap_or_else(|| tag.replace('_', "-"))
}

pub fn install(ui: &AppWindow, sh: &Sh) {
    let sp: Shared = Arc::new(Mutex::new(None));
    SPELLER.with(|s| *s.borrow_mut() = Some(sp.clone()));
    let (data_dir, lang) = {
        let g = sh.lock().unwrap();
        (g.data_dir.clone(), g.prefs.spell_lang.clone())
    };
    let langs = spell::available();
    ui.set_spell_langs(ModelRc::new(VecModel::from(langs.iter().map(|(_, l)| SharedString::from(l.as_str())).collect::<Vec<_>>())));
    ui.set_spell_lang_label(lang_label(&lang, &langs).into());
    STATE.with(|st| st.borrow_mut().langs = langs);
    // Parsing the dictionary takes a moment: load it off the UI thread.
    {
        let (sp2, dir, lang2) = (sp.clone(), data_dir.clone(), lang);
        let wk = ui.as_weak();
        std::thread::spawn(move || {
            let s = Speller::load(&dir, &lang2);
            *sp2.lock().unwrap() = Some(s);
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = wk.upgrade() {
                    recheck_all(&ui);
                }
            });
        });
    }

    let probes = STATE.with(|st| st.borrow().probes.clone());
    ui.on_probed(move |off, x, y| probes.borrow_mut().push((off, x, y)));

    // A change of a message box: check shortly after the typing pauses.
    let (wk, sp2) = (ui.as_weak(), sp.clone());
    ui.on_spell_check(move |which, text, width| {
        let (which, text, width) = (which.to_string(), text.to_string(), width);
        STATE.with(|st| st.borrow_mut().last.insert(which.clone(), (text.clone(), width)));
        let (wk2, sp3, w2) = (wk.clone(), sp2.clone(), which.clone());
        let timer = slint::Timer::default();
        timer.start(slint::TimerMode::SingleShot, Duration::from_millis(220), move || {
            if let Some(ui) = wk2.upgrade() {
                compute(&ui, &sp3, &w2, &text, width);
            }
        });
        STATE.with(|st| st.borrow_mut().timers.insert(which, timer));
    });

    // Right-click on the box: is it a marked word?
    let (wk, sp2) = (ui.as_weak(), sp.clone());
    ui.on_spell_context(move |which, x, y, ax, ay| {
        let Some(ui) = wk.upgrade() else { return };
        let which = which.to_string();
        let line_h = ui.global::<crate::Theme>().get_fs() * 1.21;
        let hit = STATE.with(|st| st.borrow().marks.get(&which).and_then(|v| v.iter().find(|m| x >= m.x - 1.0 && x <= m.x + m.w + 1.0 && y >= m.y && y <= m.y + line_h).cloned()));
        let Some(m) = hit else { return };
        let text = STATE.with(|st| st.borrow().last.get(&which).map(|(t, _)| t.clone())).unwrap_or_default();
        if m.end > text.len() || !text.is_char_boundary(m.start) || !text.is_char_boundary(m.end) {
            return;
        }
        let word = text[m.start..m.end].to_string();
        let suggestions = sp2.lock().unwrap().as_ref().map(|s| s.suggest(&word)).unwrap_or_default();
        ui.set_spell_menu(ModelRc::new(VecModel::from(suggestions.iter().map(|s| SharedString::from(s.as_str())).collect::<Vec<_>>())));
        ui.set_spell_word(word.clone().into());
        ui.set_menu_x(ax);
        ui.set_menu_y(ay);
        ui.set_menu_kind("spell".into());
        STATE.with(|st| st.borrow_mut().target = Some(Target { which, start: m.start, end: m.end, word, suggestions }));
    });

    let wk = ui.as_weak();
    ui.on_spell_pick(move |i| {
        let Some(ui) = wk.upgrade() else { return };
        let Some(t) = STATE.with(|st| st.borrow().target.clone()) else { return };
        let Some(rep) = t.suggestions.get(i as usize) else { return };
        let current = if t.which == "thread" { ui.get_thread_composer() } else { ui.get_composer() }.to_string();
        // The text may have changed while the menu was open: only replace when the word is still there.
        if current.get(t.start..t.end) != Some(t.word.as_str()) {
            return;
        }
        let new = format!("{}{}{}", &current[..t.start], rep, &current[t.end..]);
        if t.which == "thread" {
            ui.set_thread_composer(new.into());
        } else {
            ui.set_composer(new.clone().into());
            ui.invoke_composer_edited(new.into());
        }
    });

    let (wk, sp2) = (ui.as_weak(), sp.clone());
    ui.on_spell_add(move || {
        let Some(ui) = wk.upgrade() else { return };
        if let Some(t) = STATE.with(|st| st.borrow().target.clone()) {
            if let Some(s) = sp2.lock().unwrap().as_mut() {
                s.add_word(&t.word);
            }
        }
        recheck_all(&ui);
    });

    let wk = ui.as_weak();
    ui.on_spell_ignore(move || {
        let Some(ui) = wk.upgrade() else { return };
        if let Some(t) = STATE.with(|st| st.borrow().target.clone()) {
            STATE.with(|st| st.borrow_mut().ignore.insert(t.word.to_lowercase()));
        }
        recheck_all(&ui);
    });

    // Language picker (Settings > Chats).
    let (wk, sp2, sh2) = (ui.as_weak(), sp, sh.clone());
    ui.on_spell_lang_pick(move |i| {
        let Some(ui) = wk.upgrade() else { return };
        let Some((tag, label)) = STATE.with(|st| st.borrow().langs.get(i as usize).cloned()) else { return };
        let dir = {
            let mut g = sh2.lock().unwrap();
            g.prefs.spell_lang = tag.clone();
            super::save_prefs(&g);
            g.data_dir.clone()
        };
        ui.set_spell_lang_label(label.into());
        *sp2.lock().unwrap() = None;
        let (sp3, wk2) = (sp2.clone(), ui.as_weak());
        std::thread::spawn(move || {
            let s = Speller::load(&dir, &tag);
            *sp3.lock().unwrap() = Some(s);
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = wk2.upgrade() {
                    recheck_all(&ui);
                }
            });
        });
    });
}

thread_local! {
    static SPELLER: RefCell<Option<Shared>> = const { RefCell::new(None) };
}

/// Checks both message boxes again (after a setting, the dictionary or a personal word changed).
pub fn recheck_all(ui: &AppWindow) {
    let Some(sp) = SPELLER.with(|s| s.borrow().clone()) else { return };
    let last: Vec<(String, (String, f32))> = STATE.with(|st| st.borrow().last.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
    for (which, (text, width)) in last {
        compute(ui, &sp, &which, &text, width);
    }
}
