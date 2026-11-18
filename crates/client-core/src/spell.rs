//! E-05: spell checking for the message box. A pure-Rust Hunspell-compatible checker (`spellbook`) so it behaves the
//! same on Linux, Windows and Android with nothing to install. Dictionaries: the system's Hunspell dictionaries for the
//! user's language when present (Linux: /usr/share/hunspell), else the bundled Australian English one (SCOWL, see
//! dict/en_AU-LICENSE.txt). Words the user adds are kept in `spell-user.txt` next to the preferences.

use spellbook::Dictionary;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const EN_AU_AFF: &str = include_str!("../dict/en_AU.aff");
const EN_AU_DIC: &str = include_str!("../dict/en_AU.dic");

pub struct Speller {
    dict: Dictionary,
    pub lang: String,
    /// Where the dictionary came from: "bundled" or the file path of the system dictionary.
    pub source: String,
    user: BTreeSet<String>,
    user_file: Option<PathBuf>,
}

/// A word that is not in the dictionary: byte offsets into the checked text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Misspelling {
    pub start: usize,
    pub end: usize,
}

fn system_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = vec![];
    if let Ok(d) = std::env::var("SUPERCHAT_DICT_DIR") {
        v.push(PathBuf::from(d));
    }
    for d in ["/usr/share/hunspell", "/usr/share/myspell", "/usr/share/myspell/dicts", "/app/share/hunspell", "/usr/local/share/hunspell"] {
        v.push(PathBuf::from(d));
    }
    v
}

/// `en_AU` / `en-AU` / `en_AU.UTF-8` -> "en_AU"; "C" and "POSIX" mean nothing.
fn lang_tag(raw: &str) -> Option<String> {
    let t = raw.split(['.', '@']).next()?.replace('-', "_");
    (t.len() >= 2 && t != "C" && t != "POSIX").then_some(t)
}

fn find_system(lang: &str) -> Option<(PathBuf, PathBuf)> {
    let base = lang.split('_').next().unwrap_or(lang).to_string();
    for d in system_dirs() {
        // The bundled en_AU is better than a different English variant of the system, so only an exact match counts.
        let names: Vec<String> = if lang == "en_AU" { vec![lang.to_string()] } else { vec![lang.to_string(), base.clone()] };
        for name in names {
            let (aff, dic) = (d.join(format!("{name}.aff")), d.join(format!("{name}.dic")));
            if aff.is_file() && dic.is_file() {
                return Some((aff, dic));
            }
        }
        // en -> en_GB, en_US ... (any dictionary of the same language)
        if lang == "en_AU" {
            continue;
        }
        if let Ok(rd) = std::fs::read_dir(&d) {
            let mut names: Vec<String> = rd.filter_map(|e| e.ok()).filter_map(|e| e.file_name().into_string().ok()).filter(|n| n.starts_with(&format!("{base}_")) && n.ends_with(".dic")).collect();
            names.sort();
            for n in names {
                let stem = n.trim_end_matches(".dic");
                let (aff, dic) = (d.join(format!("{stem}.aff")), d.join(&n));
                if aff.is_file() {
                    return Some((aff, dic));
                }
            }
        }
    }
    None
}

/// The dictionaries the user can pick: the bundled one and every Hunspell dictionary found on the system, as
/// (tag such as "en_GB", label).
pub fn available() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![("en_AU".into(), "English (Australia)".into())];
    for d in system_dirs() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        let mut names: Vec<String> = rd.filter_map(|e| e.ok()).filter_map(|e| e.file_name().into_string().ok()).filter(|n| n.ends_with(".dic")).collect();
        names.sort();
        for n in names {
            let tag = n.trim_end_matches(".dic").to_string();
            if d.join(format!("{tag}.aff")).is_file() && !out.iter().any(|(t, _)| *t == tag) {
                let label = format!("{} (system)", tag.replace('_', "-"));
                out.push((tag, label));
            }
        }
    }
    out
}

impl Speller {
    /// The bundled Australian English dictionary and no personal words (tests, Windows without system dictionaries).
    pub fn bundled() -> Speller {
        let dict = Dictionary::new(EN_AU_AFF, EN_AU_DIC).expect("bundled dictionary parses");
        Speller { dict, lang: "en_AU".into(), source: "bundled".into(), user: BTreeSet::new(), user_file: None }
    }

    /// Loads the dictionary for `lang` (e.g. "en_AU"; empty reads LC_ALL / LANG): the system one if there is one, else
    /// the bundled one, then the personal words from `<dir>/spell-user.txt`.
    pub fn load(dir: &Path, lang: &str) -> Speller {
        let want = lang_tag(lang).or_else(|| ["LC_ALL", "LC_MESSAGES", "LANG"].iter().filter_map(|k| std::env::var(k).ok()).find_map(|v| lang_tag(&v)));
        let mut sp = None;
        if let Some(w) = &want {
            // English falls back to the bundled en_AU rather than a system en_US when the user asked for en_AU.
            if let Some((aff, dic)) = find_system(w) {
                if let (Ok(a), Ok(d)) = (std::fs::read_to_string(&aff), std::fs::read_to_string(&dic)) {
                    if let Ok(dict) = Dictionary::new(&a, &d) {
                        let tag = dic.file_stem().and_then(|s| s.to_str()).unwrap_or(w).to_string();
                        sp = Some(Speller { dict, lang: tag, source: dic.display().to_string(), user: BTreeSet::new(), user_file: None });
                    }
                }
            }
        }
        let mut sp = sp.unwrap_or_else(Speller::bundled);
        let f = dir.join("spell-user.txt");
        if let Ok(t) = std::fs::read_to_string(&f) {
            for w in t.lines().map(str::trim).filter(|w| !w.is_empty()) {
                sp.user.insert(w.to_string());
            }
        }
        sp.user_file = Some(f);
        sp
    }

    pub fn check(&self, word: &str) -> bool {
        let w = word.trim_matches(|c: char| c == '\'' || c == '\u{2019}');
        w.is_empty() || self.user.contains(w) || self.user.contains(&w.to_lowercase()) || self.dict.check(&w.replace('\u{2019}', "'"))
    }

    pub fn suggest(&self, word: &str) -> Vec<String> {
        let mut out = vec![];
        self.dict.suggest(&word.replace('\u{2019}', "'"), &mut out);
        out.truncate(6);
        out
    }

    /// Remembers a word as correct (also in a capitalised or lower-case form).
    pub fn add_word(&mut self, word: &str) {
        let w = word.trim();
        if w.is_empty() || self.user.contains(w) {
            return;
        }
        self.user.insert(w.to_string());
        if let Some(f) = &self.user_file {
            let _ = std::fs::write(f, self.user.iter().map(|w| format!("{w}\n")).collect::<String>());
        }
    }

    /// The misspelled words of a message. Code (`inline` and fenced blocks), links, @mentions, #tags, :emoji:, words
    /// with digits or inner capitals (CamelCase, snake_case) and one-letter words are left alone.
    pub fn misspelled(&self, text: &str) -> Vec<Misspelling> {
        let mut out = vec![];
        for (start, end) in words(text) {
            if !self.check(&text[start..end]) {
                out.push(Misspelling { start, end });
            }
        }
        out
    }
}

/// Byte ranges of the words worth checking.
pub fn words(text: &str) -> Vec<(usize, usize)> {
    let mut out = vec![];
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut in_fence = false;
    let mut line_start = true;
    let mut in_inline = false;
    while i < text.len() {
        let c = text[i..].chars().next().unwrap();
        let clen = c.len_utf8();
        if line_start && text[i..].starts_with("```") {
            in_fence = !in_fence;
            i += 3;
            line_start = false;
            continue;
        }
        if c == '\n' {
            line_start = true;
            in_inline = false;
            i += 1;
            continue;
        }
        line_start = false;
        if in_fence {
            i += clen;
            continue;
        }
        if c == '`' {
            in_inline = !in_inline;
            i += 1;
            continue;
        }
        if in_inline {
            i += clen;
            continue;
        }
        if c.is_alphabetic() {
            // The whole run of word characters (letters, digits, apostrophes inside, underscores).
            let s = i;
            let mut j = i;
            while j < text.len() {
                let ch = text[j..].chars().next().unwrap();
                let inner = (ch == '\'' || ch == '\u{2019}') && text[j + ch.len_utf8()..].chars().next().map(|n| n.is_alphabetic()).unwrap_or(false);
                if ch.is_alphanumeric() || ch == '_' || inner {
                    j += ch.len_utf8();
                } else {
                    break;
                }
            }
            let word = &text[s..j];
            let before = text[..s].chars().next_back();
            let after = text[j..].chars().next();
            let skip_ctx = matches!(before, Some('@') | Some('#') | Some(':') | Some('/') | Some('.') | Some('\\') | Some('-')) && !matches!(before, Some('-') if s >= 2 && text[..s - 1].chars().next_back().map(|p| p.is_whitespace()).unwrap_or(true))
                || matches!(after, Some(':') | Some('/') | Some('@')) && bytes.get(j + 1).map(|b| *b != b' ').unwrap_or(false);
            let has_digit = word.chars().any(|c| c.is_ascii_digit());
            let has_us = word.contains('_');
            let inner_caps = word.chars().skip(1).any(|c| c.is_uppercase()) && word.chars().any(|c| c.is_lowercase());
            let all_caps = word.chars().all(|c| !c.is_lowercase()) && word.chars().count() <= 6;
            if !skip_ctx && !has_digit && !has_us && !inner_caps && !all_caps && word.chars().count() > 1 && !looks_like_url_part(text, s, j) {
                out.push((s, j));
            }
            i = j;
            continue;
        }
        i += clen;
    }
    out
}

/// True when the word is part of a link or an e-mail address (a chunk without spaces that contains `://`, `www.` or `@`).
fn looks_like_url_part(text: &str, s: usize, e: usize) -> bool {
    let a = text[..s].rfind(char::is_whitespace).map(|p| p + 1).unwrap_or(0);
    let b = text[e..].find(char::is_whitespace).map(|p| e + p).unwrap_or(text.len());
    let chunk = &text[a..b];
    chunk.contains("://") || chunk.starts_with("www.") || chunk.contains('@') && chunk.contains('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_typos_and_suggests_australian_spellings() {
        let sp = Speller::bundled();
        let text = "Thsi is a mesage about the colour and the recieve button.";
        let bad: Vec<&str> = sp.misspelled(text).iter().map(|m| &text[m.start..m.end]).collect();
        assert_eq!(bad, vec!["Thsi", "mesage", "recieve"]);
        assert!(sp.check("colour"), "Australian spelling is right");
        assert!(!sp.check("color"), "American spelling is flagged by en_AU");
        assert!(sp.suggest("mesage").iter().any(|s| s == "message"), "{:?}", sp.suggest("mesage"));
        assert!(sp.suggest("recieve").iter().any(|s| s == "receive"));
    }

    #[test]
    fn code_links_mentions_and_identifiers_are_left_alone() {
        let sp = Speller::bundled();
        let text = "Run `cargoo test` then see https://exampel.com/pathh or mail bobb@exampel.com\n```\nfunc xyzzy()\n```\n@alicee #chanl :smilee: file_nme SuperChatt v1.3 NASA fine";
        assert_eq!(sp.misspelled(text), vec![], "{:?}", sp.misspelled(text).iter().map(|m| &text[m.start..m.end]).collect::<Vec<_>>());
        // But ordinary words next to such things are still checked.
        let t2 = "teh `code` and teh link https://x.y";
        assert_eq!(sp.misspelled(t2).len(), 2);
    }

    #[test]
    fn apostrophes_unicode_and_offsets() {
        let sp = Speller::bundled();
        let text = "It\u{2019}s fine, don't worry. Zoë wrongg";
        // Non-ASCII text must not break the byte offsets, whether or not the word is in the dictionary.
        let _ = sp.check("café");
        let m = sp.misspelled(text);
        let last = m.last().expect("flags the typo");
        assert_eq!(&text[last.start..last.end], "wrongg");
        assert!(m.iter().all(|x| text.is_char_boundary(x.start) && text.is_char_boundary(x.end)));
    }

    #[test]
    fn personal_words_are_kept_between_runs() {
        let dir = std::env::temp_dir().join(format!("scspell-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut sp = Speller::load(&dir, "en_AU");
        assert!(!sp.check("Zorblax"));
        sp.add_word("Zorblax");
        assert!(sp.check("Zorblax"));
        let again = Speller::load(&dir, "en_AU");
        assert!(again.check("Zorblax"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loads_a_system_dictionary_when_asked_for_its_language() {
        // The CI image may not have Hunspell dictionaries; only assert when one is there.
        if std::path::Path::new("/usr/share/hunspell/en_US.dic").exists() {
            let dir = std::env::temp_dir().join(format!("scspell2-{}", std::process::id()));
            let sp = Speller::load(&dir, "en_US");
            assert_eq!(sp.lang, "en_US");
            assert!(sp.source.ends_with("en_US.dic"), "{}", sp.source);
            assert!(sp.check("color"));
        }
    }
}
