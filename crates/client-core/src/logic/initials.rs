//! 4.5 Avatars: two letters of the name when there is no photo, on a coloured tile.

/// Two or more words: first letter of the first two words ("Dan Smith" -> "DS").
/// One word: its first two letters ("general" -> "GE"). Leading `#` and `@` are stripped;
/// result is upper case.
pub fn initials(name: &str) -> String {
    let cleaned = name.trim().trim_start_matches(['#', '@']).trim();
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    let out: String = match words.len() {
        0 => return "?".to_string(),
        1 => words[0].chars().filter(|c| c.is_alphanumeric()).take(2).collect(),
        _ => words
            .iter()
            .filter_map(|w| w.chars().find(|c| c.is_alphanumeric()))
            .take(2)
            .collect(),
    };
    if out.is_empty() {
        "?".into()
    } else {
        out.to_uppercase()
    }
}

/// Fixed palette (all work with white text at 4.5:1 or better).
pub const PALETTE: [(u8, u8, u8); 8] = [
    (0x1B, 0x6F, 0xD8),
    (0x0B, 0x7A, 0x4E),
    (0xB4, 0x3C, 0x00),
    (0x8A, 0x2B, 0xB4),
    (0xB0, 0x1E, 0x4D),
    (0x00, 0x6D, 0x77),
    (0x5C, 0x5C, 0xE0),
    (0x7A, 0x5A, 0x00),
];

/// Stable palette index from the user / conversation id (FNV-1a, independent of platform).
pub fn color_index(id: &str) -> usize {
    let mut h: u32 = 0x811C_9DC5;
    for b in id.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    (h as usize) % PALETTE.len()
}

pub fn color_rgb(id: &str) -> (u8, u8, u8) {
    PALETTE[color_index(id)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_words() {
        assert_eq!(initials("Dan Smith"), "DS");
        assert_eq!(initials("  dan   smith  jones "), "DS");
    }

    #[test]
    fn one_word() {
        assert_eq!(initials("general"), "GE");
        assert_eq!(initials("x"), "X");
    }

    #[test]
    fn strips_prefixes() {
        assert_eq!(initials("#general"), "GE");
        assert_eq!(initials("@dan"), "DA");
        assert_eq!(initials("#design team"), "DT");
    }

    #[test]
    fn unicode_and_empty() {
        assert_eq!(initials("élodie roux"), "ÉR");
        assert_eq!(initials(""), "?");
        assert_eq!(initials("##"), "?");
    }

    #[test]
    fn colour_is_stable() {
        assert_eq!(color_index("abc"), color_index("abc"));
        assert!(color_index("user-1") < PALETTE.len());
        let spread: std::collections::HashSet<usize> = (0..50).map(|i| color_index(&format!("u{i}"))).collect();
        assert!(spread.len() > 3);
    }
}
