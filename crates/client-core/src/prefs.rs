//! Non-secret UI preferences (E-01..E-04). Kept apart from the chat cache so they survive a
//! forced wipe: they contain no chat data.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Prefs {
    pub last_server: String,
    pub last_username: String,
    /// "system", "light" or "dark".
    pub theme: String,
    /// Index into the accent/sidebar presets.
    pub accent: u8,
    /// 100, 115 or 130 (Regular, Large, Extra large).
    pub font_scale: u16,
    /// Whole-UI zoom in percent (Ctrl +/-/0).
    pub zoom: u16,
    pub compact: bool,
    pub reduce_motion: bool,
    pub hour24: bool,
    pub notifications: bool,
    pub notification_preview: bool,
    pub allow_insecure_http: bool,
    pub start_minimised: bool,
    pub launch_on_login: bool,
    /// Conversation list sort: "activity" or "alpha" (NV-05).
    pub sort: String,
    pub unread_only: bool,
    pub hide_muted: bool,
    /// Android push choice: "none", "ntfy" or "fcm" (N-01).
    pub push_choice: String,
    pub ntfy_url: String,
    /// This device's ntfy topic and the server-side registration id (N-01).
    pub ntfy_topic: String,
    pub push_reg_id: String,
    /// Android: keep the foreground service running (X-06).
    pub foreground_service: bool,
    /// Desktop: closing the window keeps the app in the tray (E-08).
    pub close_to_tray: bool,
    /// SR-06: recent searches, newest first (max 10), and saved searches.
    pub recent_searches: Vec<String>,
    pub saved_searches: Vec<String>,
    /// Quick switcher history.
    pub spell_check: bool,
    /// N-04: play a sound for new messages at all (off = every notification is silent).
    pub notification_sound: bool,
    /// N-04: the sound for conversations without their own choice ("default" = the system sound, "none", or a bundled id).
    pub sound_default: String,
    /// N-04: per-conversation sound, conversation id to sound id.
    pub sound_conv: std::collections::BTreeMap<String, String>,
    /// N-04: show the unread count on the tray icon, the window title and the launcher badge.
    pub badge_counts: bool,
    /// Sidebar sections the user collapsed (`sec:fav`, `sec:channels`, `sec:dms`).
    pub collapsed_sections: Vec<String>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            last_server: String::new(),
            last_username: String::new(),
            theme: "system".into(),
            accent: 0,
            font_scale: 100,
            zoom: 100,
            compact: false,
            reduce_motion: false,
            hour24: false,
            notifications: true,
            notification_preview: true,
            allow_insecure_http: false,
            start_minimised: false,
            launch_on_login: false,
            sort: "activity".into(),
            unread_only: false,
            hide_muted: false,
            push_choice: "none".into(),
            ntfy_url: String::new(),
            ntfy_topic: String::new(),
            push_reg_id: String::new(),
            foreground_service: true,
            close_to_tray: true,
            recent_searches: vec![],
            saved_searches: vec![],
            spell_check: true,
            collapsed_sections: vec![],
            notification_sound: true,
            sound_default: "default".into(),
            sound_conv: Default::default(),
            badge_counts: true,
        }
    }
}

impl Prefs {
    pub fn load(dir: &std::path::Path) -> Prefs {
        std::fs::read_to_string(Self::path(dir)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, dir: &std::path::Path) {
        let _ = std::fs::create_dir_all(dir);
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::path(dir), s);
        }
    }

    fn path(dir: &std::path::Path) -> PathBuf {
        dir.join("prefs.json")
    }

    /// Sidebar sections the given account collapsed. Keys are stored per account as `<account>|<key>`
    /// (the account id is unique per server and user); keys without a prefix are 1.2 leftovers and apply to everyone.
    /// N-04: the sound a new message in `conv_id` plays: the conversation's own choice, else the default; "none" when
    /// sounds are off.
    pub fn sound_for(&self, conv_id: &str) -> String {
        if !self.notification_sound {
            return "none".into();
        }
        self.sound_conv.get(conv_id).cloned().filter(|s| crate::sounds::is_valid(s)).unwrap_or_else(|| if crate::sounds::is_valid(&self.sound_default) { self.sound_default.clone() } else { "default".into() })
    }

    /// N-04: sets (or, with "default", clears) the conversation's own sound.
    pub fn set_conv_sound(&mut self, conv_id: &str, sound: &str) {
        if sound == "default" || !crate::sounds::is_valid(sound) {
            self.sound_conv.remove(conv_id);
        } else {
            self.sound_conv.insert(conv_id.to_string(), sound.to_string());
        }
    }

    pub fn collapsed_for(&self, account: &str) -> Vec<String> {
        let pre = format!("{account}|");
        self.collapsed_sections.iter().filter_map(|k| if let Some(r) = k.strip_prefix(&pre) { Some(r.to_string()) } else if !k.contains('|') { Some(k.clone()) } else { None }).collect()
    }

    /// Flips one section key for one account and persists nothing itself (the caller saves).
    pub fn toggle_collapsed(&mut self, account: &str, key: &str) {
        let now = self.collapsed_for(account).iter().any(|k| k == key);
        let scoped = format!("{account}|{key}");
        self.collapsed_sections.retain(|k| *k != scoped && k != key);
        if !now {
            self.collapsed_sections.push(scoped);
        }
    }

    /// SR-06: remembers a search (newest first, no duplicates, at most 10).
    pub fn remember_search(&mut self, q: &str) {
        let q = q.trim();
        if q.is_empty() {
            return;
        }
        self.recent_searches.retain(|r| r != q);
        self.recent_searches.insert(0, q.to_string());
        self.recent_searches.truncate(10);
    }

    /// Combined text scale factor (font size step times zoom), capped at 200%.
    pub fn text_scale(&self, system_scale: f32) -> f32 {
        (self.font_scale as f32 / 100.0 * system_scale * self.zoom as f32 / 100.0).clamp(0.5, 2.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_is_capped_at_200_percent() {
        let p = Prefs { font_scale: 130, zoom: 150, ..Default::default() };
        assert_eq!(p.text_scale(1.5), 2.0);
        let p = Prefs::default();
        assert!((p.text_scale(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn notification_sound_per_conversation_with_global_switch() {
        let dir = std::env::temp_dir().join(format!("scsound-{}", std::process::id()));
        let mut p = Prefs::default();
        assert_eq!(p.sound_for("c1"), "default");
        p.sound_default = "pop".into();
        p.set_conv_sound("c1", "bell");
        p.set_conv_sound("c2", "none");
        p.set_conv_sound("c3", "not-a-sound");
        assert_eq!(p.sound_for("c1"), "bell");
        assert_eq!(p.sound_for("c2"), "none");
        assert_eq!(p.sound_for("c3"), "pop", "unknown ids are ignored");
        p.save(&dir);
        let back = Prefs::load(&dir);
        assert_eq!(back.sound_for("c1"), "bell");
        // Back to the default clears the override.
        let mut back = back;
        back.set_conv_sound("c1", "default");
        assert_eq!(back.sound_for("c1"), "pop");
        // The global switch silences everything, including conversation choices.
        back.set_conv_sound("c1", "bell");
        back.notification_sound = false;
        assert_eq!(back.sound_for("c1"), "none");
        // Old prefs files without the new fields still load with sounds on.
        std::fs::write(dir.join("prefs.json"), r#"{"theme":"dark"}"#).unwrap();
        let old = Prefs::load(&dir);
        assert!(old.notification_sound && old.badge_counts);
        assert_eq!(old.sound_for("x"), "default");
        // The bundled files exist and are real WAVs.
        let f = crate::sounds::file(&dir, "chime").unwrap();
        assert!(crate::media::analyze_audio(&f).unwrap().duration_ms.unwrap() > 300);
        assert!(crate::sounds::file(&dir, "default").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn collapsed_sections_are_per_account_and_survive_a_reload() {
        let d = tempfile::tempdir().unwrap();
        let mut p = Prefs::default();
        p.toggle_collapsed("u1", "sec:shortcuts");
        p.toggle_collapsed("u2", "sec:channels");
        p.save(d.path());
        let p = Prefs::load(d.path());
        assert_eq!(p.collapsed_for("u1"), vec!["sec:shortcuts".to_string()]);
        assert_eq!(p.collapsed_for("u2"), vec!["sec:channels".to_string()]);
        let mut p = p;
        p.toggle_collapsed("u1", "sec:shortcuts");
        assert!(p.collapsed_for("u1").is_empty());
        // A 1.2 key without an account prefix still applies, and toggling it replaces it with a scoped key.
        let mut p = Prefs { collapsed_sections: vec!["sec:dms".into()], ..Default::default() };
        assert_eq!(p.collapsed_for("u1"), vec!["sec:dms".to_string()]);
        p.toggle_collapsed("u1", "sec:dms");
        assert!(p.collapsed_for("u1").is_empty());
    }

    #[test]
    fn roundtrip_and_defaults() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(Prefs::load(d.path()), Prefs::default());
        let p = Prefs { theme: "dark".into(), font_scale: 115, ..Default::default() };
        p.save(d.path());
        assert_eq!(Prefs::load(d.path()), p);
        assert!(!Prefs::default().hour24, "Australian default is 12-hour");
    }
}
