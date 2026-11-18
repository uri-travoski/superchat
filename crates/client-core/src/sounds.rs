//! N-04: the notification sounds that ship with the app, and the rules for which one a conversation uses.
//!
//! "default" is the system's own notification sound (the desktop notification daemon, the Android default channel),
//! "none" is silent, every other id is a bundled sound that the apps play themselves (desktop) or attach to an
//! Android notification channel (one channel per sound, because Android fixes a channel's sound once it exists).

use std::path::{Path, PathBuf};

pub struct Sound {
    pub id: &'static str,
    pub label: &'static str,
    pub wav: &'static [u8],
}

pub const SOUNDS: [Sound; 4] = [
    Sound { id: "chime", label: "Chime", wav: include_bytes!("../sounds/chime.wav") },
    Sound { id: "pop", label: "Pop", wav: include_bytes!("../sounds/pop.wav") },
    Sound { id: "knock", label: "Knock", wav: include_bytes!("../sounds/knock.wav") },
    Sound { id: "bell", label: "Bell", wav: include_bytes!("../sounds/bell.wav") },
];

pub fn is_valid(id: &str) -> bool {
    matches!(id, "default" | "none") || SOUNDS.iter().any(|s| s.id == id)
}

pub fn label(id: &str) -> &'static str {
    match id {
        "default" => "System default",
        "none" => "Silent",
        _ => SOUNDS.iter().find(|s| s.id == id).map(|s| s.label).unwrap_or("System default"),
    }
}

/// Writes the bundled sounds under `<dir>/sounds` (once) and returns the folder.
pub fn ensure_files(dir: &Path) -> PathBuf {
    let d = dir.join("sounds");
    let _ = std::fs::create_dir_all(&d);
    for s in &SOUNDS {
        let p = d.join(format!("{}.wav", s.id));
        if std::fs::metadata(&p).map(|m| m.len() as usize != s.wav.len()).unwrap_or(true) {
            let _ = std::fs::write(&p, s.wav);
        }
    }
    d
}

/// The file of a bundled sound, None for "default" and "none".
pub fn file(dir: &Path, id: &str) -> Option<PathBuf> {
    SOUNDS.iter().find(|s| s.id == id)?;
    let d = ensure_files(dir);
    Some(d.join(format!("{id}.wav")))
}
