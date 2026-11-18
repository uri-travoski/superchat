//! `:name:` emoji shortcodes for the composer's autocomplete.

pub const CODES: &[(&str, &str)] = &[
    ("smile", "\u{1F604}"), ("grin", "\u{1F601}"), ("joy", "\u{1F602}"), ("laughing", "\u{1F606}"), ("wink", "\u{1F609}"),
    ("blush", "\u{1F60A}"), ("heart_eyes", "\u{1F60D}"), ("thinking", "\u{1F914}"), ("neutral", "\u{1F610}"), ("roll_eyes", "\u{1F644}"),
    ("open_mouth", "\u{1F62E}"), ("cry", "\u{1F622}"), ("sob", "\u{1F62D}"), ("angry", "\u{1F620}"), ("scream", "\u{1F631}"),
    ("sleeping", "\u{1F634}"), ("partying", "\u{1F973}"), ("sunglasses", "\u{1F60E}"), ("thumbsup", "\u{1F44D}"), ("+1", "\u{1F44D}"),
    ("thumbsdown", "\u{1F44E}"), ("-1", "\u{1F44E}"), ("clap", "\u{1F44F}"), ("raised_hands", "\u{1F64C}"), ("pray", "\u{1F64F}"),
    ("muscle", "\u{1F4AA}"), ("wave", "\u{1F44B}"), ("ok_hand", "\u{1F44C}"), ("handshake", "\u{1F91D}"), ("eyes", "\u{1F440}"),
    ("brain", "\u{1F9E0}"), ("heart", "\u{2764}\u{FE0F}"), ("broken_heart", "\u{1F494}"), ("fire", "\u{1F525}"), ("sparkles", "\u{2728}"),
    ("tada", "\u{1F389}"), ("confetti", "\u{1F38A}"), ("white_check_mark", "\u{2705}"), ("check", "\u{2705}"), ("x", "\u{274C}"),
    ("warning", "\u{26A0}\u{FE0F}"), ("bulb", "\u{1F4A1}"), ("100", "\u{1F4AF}"), ("rocket", "\u{1F680}"), ("star", "\u{2B50}"),
    ("trophy", "\u{1F3C6}"), ("paperclip", "\u{1F4CE}"), ("calendar", "\u{1F4C5}"), ("memo", "\u{1F4DD}"), ("loudspeaker", "\u{1F4E3}"),
    ("bell", "\u{1F514}"), ("lock", "\u{1F512}"), ("wrench", "\u{1F527}"), ("computer", "\u{1F4BB}"), ("phone", "\u{1F4F1}"),
    ("coffee", "\u{2615}"), ("pizza", "\u{1F355}"), ("cake", "\u{1F382}"),
];

/// The `:query` being typed at the end of `text` (at least two characters after the colon), if any.
pub fn query(text: &str) -> Option<String> {
    let i = text.rfind(':')?;
    if i > 0 && !text[..i].ends_with(char::is_whitespace) {
        return None;
    }
    let q = &text[i + 1..];
    (q.chars().count() >= 2 && q.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '+' || c == '-')).then(|| q.to_lowercase())
}

pub fn matches(q: &str) -> Vec<String> {
    CODES.iter().filter(|(n, _)| n.starts_with(q)).take(8).map(|(_, e)| e.to_string()).collect()
}

/// Replaces the trailing `:query` of `text` with `emoji`.
pub fn complete(text: &str, emoji: &str) -> String {
    match text.rfind(':') {
        Some(i) => format!("{}{} ", &text[..i], emoji),
        None => format!("{text}{emoji}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finds_the_query_and_completes_it() {
        assert_eq!(query("hello :sm"), Some("sm".into()));
        assert_eq!(query("hello :s"), None);
        assert_eq!(query("a:sm"), None);
        assert!(matches("sm").contains(&"\u{1F604}".to_string()));
        assert_eq!(complete("hi :joy", "X"), "hi X ");
    }
}
