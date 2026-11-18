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
    fn roundtrip_and_defaults() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(Prefs::load(d.path()), Prefs::default());
        let p = Prefs { theme: "dark".into(), font_scale: 115, ..Default::default() };
        p.save(d.path());
        assert_eq!(Prefs::load(d.path()), p);
        assert!(!Prefs::default().hour24, "Australian default is 12-hour");
    }
}
