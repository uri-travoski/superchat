//! Message markup (M-12 .. M-14, M-16). The server stores the text verbatim and never renders
//! it; clients parse it into blocks and spans and draw them natively (never as HTML).
//!
//! Syntax: `*bold*`, `_italic_`, `~strike~`, `` `code` ``, fenced ``` blocks, `> quote`,
//! `- bullet`, `1. numbered`, `@username` mentions and bare `http(s)://` links.

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub link: Option<String>,
    /// Username when this span is an `@mention`.
    pub mention: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Paragraph(Vec<Span>),
    Code(String),
    Quote(Vec<Span>),
    List { ordered: bool, items: Vec<Vec<Span>> },
}

pub fn parse(input: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = vec![];
    let lines: Vec<&str> = input.lines().collect();
    let mut i = 0;
    let mut para: Vec<&str> = vec![];
    let flush = |para: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if !para.is_empty() {
            blocks.push(Block::Paragraph(parse_inline(&para.join("\n"))));
            para.clear();
        }
    };
    while i < lines.len() {
        let line = lines[i];
        let t = line.trim_start();
        if t.starts_with("```") {
            flush(&mut para, &mut blocks);
            let mut code = vec![];
            let after = t.trim_start_matches('`');
            // Same-line closing fence: ```code```
            if let Some(end) = after.find("```") {
                blocks.push(Block::Code(after[..end].trim().to_string()));
                i += 1;
                continue;
            }
            let first = after.trim();
            if !first.is_empty() && !first.chars().all(|c| c.is_alphanumeric() || c == '+' || c == '-') {
                code.push(first.to_string());
            }
            i += 1;
            let mut closed = false;
            while i < lines.len() {
                if lines[i].trim_start().starts_with("```") {
                    closed = true;
                    i += 1;
                    break;
                }
                code.push(lines[i].to_string());
                i += 1;
            }
            let _ = closed;
            blocks.push(Block::Code(code.join("\n")));
            continue;
        }
        if let Some(rest) = t.strip_prefix("> ").or_else(|| if t == ">" { Some("") } else { None }) {
            flush(&mut para, &mut blocks);
            let mut q = vec![rest.to_string()];
            i += 1;
            while i < lines.len() {
                let n = lines[i].trim_start();
                if let Some(r) = n.strip_prefix("> ").or_else(|| if n == ">" { Some("") } else { None }) {
                    q.push(r.to_string());
                    i += 1;
                } else {
                    break;
                }
            }
            blocks.push(Block::Quote(parse_inline(&q.join("\n"))));
            continue;
        }
        if let Some(item) = bullet(t) {
            flush(&mut para, &mut blocks);
            let mut items = vec![parse_inline(item)];
            i += 1;
            while i < lines.len() {
                match bullet(lines[i].trim_start()) {
                    Some(it) => {
                        items.push(parse_inline(it));
                        i += 1;
                    }
                    None => break,
                }
            }
            blocks.push(Block::List { ordered: false, items });
            continue;
        }
        if let Some(item) = numbered(t) {
            flush(&mut para, &mut blocks);
            let mut items = vec![parse_inline(item)];
            i += 1;
            while i < lines.len() {
                match numbered(lines[i].trim_start()) {
                    Some(it) => {
                        items.push(parse_inline(it));
                        i += 1;
                    }
                    None => break,
                }
            }
            blocks.push(Block::List { ordered: true, items });
            continue;
        }
        if line.trim().is_empty() {
            flush(&mut para, &mut blocks);
        } else {
            para.push(line);
        }
        i += 1;
    }
    flush(&mut para, &mut blocks);
    blocks
}

fn bullet(t: &str) -> Option<&str> {
    t.strip_prefix("- ").or_else(|| t.strip_prefix("• "))
}

fn numbered(t: &str) -> Option<&str> {
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 3 {
        return None;
    }
    t[digits..].strip_prefix(". ")
}

#[derive(Clone, Copy, Default)]
struct Style {
    bold: bool,
    italic: bool,
    strike: bool,
}

pub fn parse_inline(s: &str) -> Vec<Span> {
    let mut out = vec![];
    inline(s, Style::default(), &mut out);
    merge(out)
}

fn is_boundary_before(prev: Option<char>) -> bool {
    match prev {
        None => true,
        Some(c) => c.is_whitespace() || "([{>,.;:!?\"'-*_~".contains(c),
    }
}

fn is_boundary_after(next: Option<char>) -> bool {
    match next {
        None => true,
        Some(c) => c.is_whitespace() || ")]}<,.;:!?\"'-*_~".contains(c),
    }
}

fn inline(s: &str, style: Style, out: &mut Vec<Span>) {
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut buf = String::new();
    let push_text = |buf: &mut String, out: &mut Vec<Span>| {
        if !buf.is_empty() {
            out.push(Span { text: std::mem::take(buf), bold: style.bold, italic: style.italic, strike: style.strike, ..Default::default() });
        }
    };
    while i < chars.len() {
        let c = chars[i];
        let prev = if i > 0 { Some(chars[i - 1]) } else { None };
        match c {
            '`' => {
                if let Some(end) = find_close(&chars, i + 1, '`') {
                    if end > i + 1 {
                        push_text(&mut buf, out);
                        out.push(Span {
                            text: chars[i + 1..end].iter().collect(),
                            code: true,
                            bold: style.bold,
                            italic: style.italic,
                            strike: style.strike,
                            ..Default::default()
                        });
                        i = end + 1;
                        continue;
                    }
                }
            }
            '*' | '_' | '~' => {
                if is_boundary_before(prev) && i + 1 < chars.len() && !chars[i + 1].is_whitespace() {
                    if let Some(end) = find_close(&chars, i + 1, c) {
                        let after = chars.get(end + 1).copied();
                        let before_close = chars[end - 1];
                        if end > i + 1 && !before_close.is_whitespace() && is_boundary_after(after) {
                            push_text(&mut buf, out);
                            let mut st = style;
                            match c {
                                '*' => st.bold = true,
                                '_' => st.italic = true,
                                _ => st.strike = true,
                            }
                            let inner: String = chars[i + 1..end].iter().collect();
                            inline(&inner, st, out);
                            i = end + 1;
                            continue;
                        }
                    }
                }
            }
            '@' => {
                if is_boundary_before(prev) {
                    let name: String = chars[i + 1..]
                        .iter()
                        .take_while(|c| c.is_ascii_alphanumeric() || matches!(**c, '.' | '_' | '-'))
                        .collect();
                    let name_trim = name.trim_end_matches(['.', '-', '_']).to_string();
                    if !name_trim.is_empty() {
                        push_text(&mut buf, out);
                        let n = name_trim.chars().count();
                        out.push(Span {
                            text: format!("@{name_trim}"),
                            mention: Some(name_trim),
                            bold: style.bold,
                            italic: style.italic,
                            strike: style.strike,
                            ..Default::default()
                        });
                        i += 1 + n;
                        continue;
                    }
                }
            }
            'h'
                if is_boundary_before(prev) => {
                    let rest: String = chars[i..].iter().collect();
                    if rest.starts_with("http://") || rest.starts_with("https://") {
                        let url: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
                        let trimmed = url.trim_end_matches(['.', ',', ')', ';', '!', '?', '\'', '"']).to_string();
                        if trimmed.len() > 8 {
                            push_text(&mut buf, out);
                            let n = trimmed.chars().count();
                            out.push(Span { text: trimmed.clone(), link: Some(trimmed), bold: style.bold, italic: style.italic, strike: style.strike, ..Default::default() });
                            i += n;
                            continue;
                        }
                    }
                }
            _ => {}
        }
        buf.push(c);
        i += 1;
    }
    push_text(&mut buf, out);
}

fn find_close(chars: &[char], from: usize, delim: char) -> Option<usize> {
    (from..chars.len()).find(|&j| chars[j] == delim && chars[j] != '\n')
}

fn merge(spans: Vec<Span>) -> Vec<Span> {
    let mut out: Vec<Span> = vec![];
    for s in spans {
        if let Some(last) = out.last_mut() {
            if last.bold == s.bold
                && last.italic == s.italic
                && last.strike == s.strike
                && last.code == s.code
                && last.link.is_none()
                && s.link.is_none()
                && last.mention.is_none()
                && s.mention.is_none()
            {
                last.text.push_str(&s.text);
                continue;
            }
        }
        out.push(s);
    }
    out
}

fn md_escape(t: &str) -> String {
    let mut o = String::with_capacity(t.len());
    for c in t.chars() {
        if matches!(c, '\\' | '*' | '_' | '~' | '`' | '[' | ']' | '<' | '>' | '#' | '|') {
            o.push('\\');
        }
        o.push(c);
    }
    o
}

/// CommonMark rendering of inline spans for Slint's `StyledText` (bold, italic, strike, code, links;
/// mentions are bold in `accent`, an `#rrggbb` string).
pub fn spans_to_markdown(spans: &[Span], accent: &str) -> String {
    let mut out = String::new();
    for s in spans {
        let mut t = if s.code { format!("`{}`", s.text.replace('`', "'")) } else { md_escape(&s.text) };
        if let Some(l) = &s.link {
            t = format!("[{}]({})", md_escape(&s.text), l);
        }
        if s.mention.is_some() {
            t = format!("<font color=\"{accent}\">**{t}**</font>");
        } else {
            if s.strike {
                t = format!("~~{t}~~");
            }
            if s.italic {
                t = format!("*{t}*");
            }
            if s.bold {
                t = format!("**{t}**");
            }
        }
        out.push_str(&t);
    }
    out
}

/// One renderable block for the UI: `kind` 0 paragraph, 1 code, 2 quote; `markdown` is ready for StyledText.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiBlock {
    pub kind: u8,
    pub markdown: String,
    pub plain: String,
}

pub fn ui_blocks(input: &str, accent: &str) -> Vec<UiBlock> {
    let mut out = vec![];
    for b in parse(input) {
        match b {
            Block::Paragraph(s) => out.push(UiBlock { kind: 0, markdown: spans_to_markdown(&s, accent).replace('\n', "  \n"), plain: spans_text(&s) }),
            Block::Quote(s) => out.push(UiBlock { kind: 2, markdown: spans_to_markdown(&s, accent), plain: spans_text(&s) }),
            Block::Code(c) => out.push(UiBlock { kind: 1, markdown: String::new(), plain: c }),
            Block::List { ordered, items } => {
                let md: Vec<String> = items.iter().enumerate().map(|(n, it)| if ordered { format!("{}. {}", n + 1, spans_to_markdown(it, accent)) } else { format!("- {}", spans_to_markdown(it, accent)) }).collect();
                let plain: Vec<String> = items.iter().map(|it| spans_text(it)).collect();
                out.push(UiBlock { kind: 0, markdown: md.join("\n"), plain: plain.join("\n") });
            }
        }
    }
    out
}

/// Plain text with the markup removed (list previews, notifications).
pub fn to_plain(input: &str) -> String {
    let mut lines = vec![];
    for b in parse(input) {
        match b {
            Block::Paragraph(s) | Block::Quote(s) => lines.push(spans_text(&s)),
            Block::Code(c) => lines.push(c),
            Block::List { ordered, items } => {
                for (n, it) in items.iter().enumerate() {
                    lines.push(if ordered { format!("{}. {}", n + 1, spans_text(it)) } else { format!("• {}", spans_text(it)) });
                }
            }
        }
    }
    lines.join(" ").split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn spans_text(spans: &[Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}

/// Usernames mentioned in a message body, in order, without duplicates.
pub fn mentioned_usernames(input: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for b in parse(input) {
        let spans: Vec<Span> = match b {
            Block::Paragraph(s) | Block::Quote(s) => s,
            Block::List { items, .. } => items.into_iter().flatten().collect(),
            Block::Code(_) => vec![],
        };
        for s in spans {
            if let Some(m) = s.mention {
                if !out.iter().any(|x| x.eq_ignore_ascii_case(&m)) {
                    out.push(m);
                }
            }
        }
    }
    out
}

/// Autocomplete: the text typed after an `@` that the cursor is inside, if any.
pub fn mention_query(text: &str, cursor: usize) -> Option<String> {
    let before: String = text.chars().take(cursor).collect();
    let at = before.rfind('@')?;
    let prev = before[..at].chars().last();
    if !is_boundary_before(prev) {
        return None;
    }
    let q = &before[at + 1..];
    if q.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
        Some(q.to_string())
    } else {
        None
    }
}

/// Toolbar helper (M-15): wraps the selection `[start, end)` (char indices) in `marker`.
/// Returns the new text and the new selection.
pub fn wrap_selection(text: &str, start: usize, end: usize, marker: &str) -> (String, usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let (s, e) = (start.min(chars.len()), end.min(chars.len()).max(start.min(chars.len())));
    let m: Vec<char> = marker.chars().collect();
    let mut out: Vec<char> = chars[..s].to_vec();
    out.extend(&m);
    out.extend(&chars[s..e]);
    out.extend(&m);
    out.extend(&chars[e..]);
    (out.into_iter().collect(), s + m.len(), e + m.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(s: &str) -> Vec<Span> {
        parse_inline(s)
    }

    #[test]
    fn bold_italic_strike_code() {
        let s = spans("a *bold* _it_ ~gone~ `x*y*` z");
        assert_eq!(s[0].text, "a ");
        assert!(s[1].bold && s[1].text == "bold");
        assert!(s[3].italic && s[3].text == "it");
        assert!(s[5].strike && s[5].text == "gone");
        assert!(s[7].code && s[7].text == "x*y*");
        assert_eq!(s.last().unwrap().text, " z");
    }

    #[test]
    fn nesting() {
        let s = spans("*bold _both_ text*");
        assert!(s.iter().any(|x| x.bold && x.italic && x.text == "both"));
    }

    #[test]
    fn snake_case_is_not_italic() {
        let s = spans("use snake_case_names here");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].text, "use snake_case_names here");
    }

    #[test]
    fn math_stars_stay_literal() {
        let s = spans("2 * 3 * 4");
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn unterminated_is_literal() {
        let s = spans("a *b");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].text, "a *b");
    }

    #[test]
    fn mentions_and_links() {
        let s = spans("hi @dan.smith, see https://example.com/a?b=1.");
        assert_eq!(s[1].mention.as_deref(), Some("dan.smith"));
        assert_eq!(s[3].link.as_deref(), Some("https://example.com/a?b=1"));
        assert_eq!(mentioned_usernames("@a and @b and @A"), vec!["a".to_string(), "b".to_string()]);
        assert!(spans("mail me at x@y.com")[0].mention.is_none());
    }

    #[test]
    fn code_block() {
        let b = parse("before\n```\nlet x = 1;\n*not bold*\n```\nafter");
        assert_eq!(b.len(), 3);
        assert_eq!(b[1], Block::Code("let x = 1;\n*not bold*".into()));
    }

    #[test]
    fn code_block_with_lang_and_inline_fence() {
        assert_eq!(parse("```rust\nfn main(){}\n```"), vec![Block::Code("fn main(){}".into())]);
        assert_eq!(parse("```one liner```"), vec![Block::Code("one liner".into())]);
    }

    #[test]
    fn quotes_and_lists() {
        let b = parse("> quoted *text*\n> more\n- a\n- b\n1. one\n2. two");
        assert!(matches!(&b[0], Block::Quote(s) if spans_text(s) == "quoted text\nmore"));
        assert!(matches!(&b[1], Block::List { ordered: false, items } if items.len() == 2));
        assert!(matches!(&b[2], Block::List { ordered: true, items } if items.len() == 2));
    }

    #[test]
    fn plain_text() {
        assert_eq!(to_plain("*Hello* _there_\n- a\n- b"), "Hello there • a • b");
    }

    #[test]
    fn mention_autocomplete_query() {
        assert_eq!(mention_query("hey @da", 7), Some("da".into()));
        assert_eq!(mention_query("hey @", 5), Some("".into()));
        assert_eq!(mention_query("mail a@b", 8), None);
        assert_eq!(mention_query("no mention", 4), None);
    }

    #[test]
    fn toolbar_wrap() {
        let (t, s, e) = wrap_selection("hello world", 6, 11, "*");
        assert_eq!(t, "hello *world*");
        assert_eq!((s, e), (7, 12));
        let (t, _, _) = wrap_selection("x", 1, 1, "`");
        assert_eq!(t, "x``");
    }

    #[test]
    fn html_is_just_text() {
        let s = spans("<b>x</b> <script>alert(1)</script>");
        assert_eq!(spans_text(&s), "<b>x</b> <script>alert(1)</script>");
    }
}

#[cfg(test)]
mod ui_tests {
    use super::*;

    #[test]
    fn markdown_for_styled_text() {
        let b = ui_blocks("*bold* and _it_ with `code` and @bob https://x.test", "#0dbd8b");
        assert_eq!(b.len(), 1);
        assert!(b[0].markdown.contains("**bold**") && b[0].markdown.contains("*it*") && b[0].markdown.contains("`code`"));
        assert!(b[0].markdown.contains("<font color=\"#0dbd8b\">**@bob**</font>"));
        assert!(b[0].markdown.contains("[https://x.test](https://x.test)"));
    }

    #[test]
    fn blocks_code_quote_list() {
        let b = ui_blocks("```\nlet x = 1;\n```\n> quoted\n- a\n- b", "#000000");
        assert_eq!(b.iter().map(|x| x.kind).collect::<Vec<_>>(), vec![1, 2, 0]);
        assert_eq!(b[0].plain, "let x = 1;");
        assert_eq!(b[2].markdown, "- a\n- b");
    }

    #[test]
    fn special_characters_are_escaped() {
        let b = ui_blocks("1 < 2 [not a link] # not a heading", "#000000");
        assert!(b[0].markdown.contains("\\<") && b[0].markdown.contains("\\["));
    }
}
