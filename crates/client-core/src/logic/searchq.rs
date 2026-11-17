//! SR-02: search text with filter chips. The Search screen shows chips; the same filters can be
//! typed: `from:@bob in:#general has:link has:file has:reaction before:01/12/2026 after:2026-11-01 budget`.

use super::datefmt::{days_from_civil, parse_date};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchFilters {
    /// Free text left after the filters were removed.
    pub text: String,
    /// `@name` / username / display name (leading `@` removed).
    pub from: Option<String>,
    /// Conversation name (leading `#` removed); resolved to an id by the caller.
    pub in_conv: Option<String>,
    pub before_ms: Option<i64>,
    pub after_ms: Option<i64>,
    pub has: Vec<String>,
    /// image, video, audio or document (file search).
    pub kind: Option<String>,
}

fn parse_any_date(v: &str, offset_secs: i32) -> Option<i64> {
    if let Some(ms) = parse_date(v, offset_secs) {
        return Some(ms);
    }
    // ISO 2026-11-17
    let mut it = v.split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) || !(1900..=3000).contains(&y) {
        return None;
    }
    Some(days_from_civil(y, m, d) * 86_400_000 - offset_secs as i64 * 1000)
}

/// Splits chips out of `input`. Unknown `word:value` tokens stay in the text.
pub fn parse(input: &str, utc_offset_secs: i32) -> SearchFilters {
    let mut f = SearchFilters::default();
    let mut rest: Vec<&str> = vec![];
    for tok in input.split_whitespace() {
        let Some((k, v)) = tok.split_once(':') else {
            rest.push(tok);
            continue;
        };
        let v = v.trim();
        if v.is_empty() {
            rest.push(tok);
            continue;
        }
        match k.to_lowercase().as_str() {
            "from" => f.from = Some(v.trim_start_matches('@').to_string()),
            "in" => f.in_conv = Some(v.trim_start_matches('#').to_string()),
            "has" if matches!(v, "link" | "file" | "reaction") => f.has.push(v.to_string()),
            "type" if matches!(v, "image" | "video" | "audio" | "document") => f.kind = Some(v.to_string()),
            "before" => match parse_any_date(v, utc_offset_secs) {
                Some(ms) => f.before_ms = Some(ms),
                None => rest.push(tok),
            },
            "after" => match parse_any_date(v, utc_offset_secs) {
                // "after 17/11" means from the end of that day.
                Some(ms) => f.after_ms = Some(ms + 86_400_000 - 1),
                None => rest.push(tok),
            },
            _ => rest.push(tok),
        }
    }
    f.text = rest.join(" ");
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chips_are_extracted() {
        let f = parse("budget from:@Bob in:#general has:link has:file before:01/12/2026 after:2026-11-01 q4", 0);
        assert_eq!(f.text, "budget q4");
        assert_eq!(f.from.as_deref(), Some("Bob"));
        assert_eq!(f.in_conv.as_deref(), Some("general"));
        assert_eq!(f.has, vec!["link", "file"]);
        assert_eq!(f.before_ms, Some(days_from_civil(2026, 12, 1) * 86_400_000));
        assert_eq!(f.after_ms, Some(days_from_civil(2026, 11, 1) * 86_400_000 + 86_399_999));
    }

    #[test]
    fn timezone_offset_moves_midnight() {
        let f = parse("before:17/11/2026", 8 * 3600);
        assert_eq!(f.before_ms, Some(days_from_civil(2026, 11, 17) * 86_400_000 - 8 * 3_600_000));
    }

    #[test]
    fn unknown_tokens_stay_text() {
        let f = parse("time: 10:30 has:bogus before:notadate", 0);
        assert_eq!(f.text, "time: 10:30 has:bogus before:notadate");
        assert!(f.has.is_empty() && f.before_ms.is_none());
    }
}
