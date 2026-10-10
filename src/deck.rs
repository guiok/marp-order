//! Reading and rewriting a Marp file.
//!
//! The file is split into a fixed head (BOM + front matter) and a list of slides.
//! Each slide keeps its text exactly as it is in the file, so reordering only moves
//! blocks of text and never reformats anything.

/// Directives that, written without `_`, apply to the slide and all the following ones
/// (or to the whole deck). Moving a slide that has them changes the result.
const FORWARD_DIRECTIVES: &[&str] = &[
    "paginate",
    "header",
    "footer",
    "class",
    "color",
    "backgroundColor",
    "backgroundImage",
    "backgroundPosition",
    "backgroundRepeat",
    "backgroundSize",
    "theme",
    "style",
    "headingDivider",
    "size",
    "math",
    "lang",
];

use std::borrow::Cow;

pub struct Slide {
    /// Separator line (`---`) that preceded the slide in the file, with its line break.
    pub lead: Option<String>,
    pub text: String,
    /// Starts with a `headingDivider` heading, which opens the slide by itself, without `---`.
    pub heading_start: bool,
    /// First slide with a comment before the divider heading: if it leaves the start,
    /// Marp moves that comment to the previous slide.
    pub lead_comment: bool,
}

pub struct Deck {
    pub source: String,
    pub head: String,
    pub slides: Vec<Slide>,
    divider: Vec<u8>,
    newline: &'static str,
}

/// One position of the file to compose: slide index and, if edited, the new text.
pub type Entry = (usize, Option<String>);

#[derive(PartialEq, Clone, Copy)]
enum Prev {
    Blank,
    Para,
    Other,
}

/// Reading state at the start of a paragraph, in case it turns into a setext heading.
#[derive(Clone, Copy, Default)]
struct Mark {
    at: usize,
    seen_visible: bool,
    slide_visible: bool,
    slide_text: bool,
}

struct Builder<'a> {
    source: &'a str,
    slides: Vec<Slide>,
    start: usize,
    lead: Option<String>,
    heading_start: bool,
    lead_comment: bool,
}

impl Builder<'_> {
    fn cut(&mut self, end: usize, next_start: usize, lead: Option<String>, heading_start: bool) {
        self.slides.push(Slide {
            lead: self.lead.take(),
            text: self.source[self.start..end].to_string(),
            heading_start: self.heading_start,
            lead_comment: self.lead_comment,
        });
        self.start = next_start;
        self.lead = lead;
        self.heading_start = heading_start;
        self.lead_comment = false;
    }

    /// Divider heading at `mark.at`. As in Marpit, it opens a new slide if there has been
    /// any visible content in the deck (comments do not count).
    fn heading(&mut self, mark: Mark) {
        if mark.seen_visible {
            self.cut(mark.at, mark.at, None, true);
        } else if !mark.slide_visible {
            self.heading_start = true;
            self.lead_comment = mark.slide_text;
        }
    }
}

/// Splits `source` into slides from `head_len` on, following Marpit's rules.
fn scan(source: &str, head_len: usize, divider: &[u8]) -> Vec<Slide> {
    let mut b = Builder {
        source,
        slides: Vec::new(),
        start: head_len,
        lead: None,
        heading_start: false,
        lead_comment: false,
    };
    let mut prev = Prev::Blank;
    let mut container = false; // inside a list or block quote
    let mut seen_visible = false; // some visible content in the deck so far
    let mut slide_visible = false;
    let mut slide_text = false; // the current slide has a non-empty line
    let mut para = Mark::default();
    let mut fence: Option<(char, usize)> = None;
    let mut raw_end: Option<&'static str> = None;

    let mut off = head_len;
    for line in source[head_len..].split_inclusive('\n') {
        let ls = off;
        off += line.len();
        let (indent, rest) = split_indent(line.trim_end_matches(['\n', '\r']));
        let here = Mark {
            at: ls,
            seen_visible,
            slide_visible,
            slide_text,
        };

        if let Some(f) = fence {
            if fence_closes(rest, f) {
                fence = None;
            }
            prev = Prev::Other;
            continue;
        }
        if let Some(end) = raw_end {
            let closed = rest.to_ascii_lowercase().contains(end);
            if closed {
                raw_end = None;
            }
            if end != "-->" || (closed && !rest.ends_with(end)) {
                seen_visible = true;
                slide_visible = true;
            }
            prev = Prev::Other;
            continue;
        }
        if rest.is_empty() {
            prev = Prev::Blank;
            continue;
        }
        slide_text = true;
        if indent >= 4 && !container {
            // indented code or paragraph continuation: never a separator
            seen_visible = true;
            slide_visible = true;
            if prev != Prev::Para {
                prev = Prev::Other;
            }
            continue;
        }
        let nested = container && indent >= 2;
        let setext_ok = prev == Prev::Para && !container;

        if let Some((ch, pure)) = thematic_break(rest) {
            if nested {
                prev = Prev::Other;
                continue;
            }
            if ch == '-' && pure && setext_ok {
                // underline of a setext heading (`Title\n---`), not a separator
                if divider.contains(&2) {
                    b.heading(para);
                }
                prev = Prev::Other;
                continue;
            }
            b.cut(ls, off, Some(line.to_string()), false);
            prev = Prev::Blank;
            container = false;
            seen_visible = true;
            slide_visible = false;
            slide_text = false;
            continue;
        }
        if setext_ok && rest.chars().all(|c| c == '=') {
            if divider.contains(&1) {
                b.heading(para);
            }
            prev = Prev::Other;
            continue;
        }
        if let Some((open_len, end)) = raw_html_opens(rest) {
            let closed = rest[open_len..].to_ascii_lowercase().contains(end);
            if !closed {
                raw_end = Some(end);
            }
            if end != "-->" || (closed && !rest.ends_with(end)) {
                seen_visible = true;
                slide_visible = true;
            }
            prev = Prev::Other;
            continue;
        }
        // from here on every line is visible content
        seen_visible = true;
        slide_visible = true;

        if let Some(f) = fence_opens(rest) {
            fence = Some(f);
            prev = Prev::Other;
            continue;
        }
        if is_container_marker(rest) {
            container = true;
            prev = Prev::Para;
            continue;
        }
        if container && prev == Prev::Blank && indent < 2 {
            container = false;
        }
        if !nested {
            if let Some(level) = atx_level(rest) {
                if divider.contains(&level) {
                    b.heading(here);
                }
                container = false;
                prev = Prev::Other;
                continue;
            }
        }
        if prev != Prev::Para {
            para = here;
        }
        prev = Prev::Para;
    }
    let end = source.len();
    b.cut(end, end, None, false);
    b.slides
}

impl Deck {
    pub fn parse(source: String) -> Deck {
        let newline = if source.contains("\r\n") { "\r\n" } else { "\n" };
        let head_len = front_matter_len(&source);
        let divider = heading_divider(&source[..head_len], &source[head_len..]);
        let slides = scan(&source, head_len, &divider);
        let head = source[..head_len].to_string();
        Deck {
            source,
            head,
            slides,
            divider,
            newline,
        }
    }

    /// Composes the file with the slides in the given order. Each entry is the index of a
    /// slide in `slides` (may be missing or repeated) and, if it was edited, the new text.
    /// The original order without edits returns the file byte for byte, and pairs of
    /// untouched slides that were already neighbours are left alone.
    pub fn compose(&self, entries: &[Entry]) -> String {
        let n = self.slides.len();
        let untouched = entries.iter().all(|(_, edit)| edit.is_none());
        if untouched && entries.len() == n && entries.iter().enumerate().all(|(i, e)| i == e.0) {
            return self.source.clone();
        }
        let mut out = self.head.clone();
        for (pos, (id, edit)) in entries.iter().enumerate() {
            let slide = &self.slides[*id];
            let (text, heading_start) = match edit {
                Some(body) => {
                    let text = self.rewrap(slide, body);
                    let heading_start = !self.divider.is_empty()
                        && scan(&text, 0, &self.divider)[0].heading_start;
                    (Cow::Owned(text), heading_start)
                }
                None => (Cow::Borrowed(slide.text.as_str()), slide.heading_start),
            };
            if pos == 0 {
                let first_line = text.lines().next();
                if out.is_empty() && first_line.map_or(true, |l| l.trim_end() == "---") {
                    // without front matter, a `---` on the first line would become front matter
                    out.push_str(self.newline);
                }
                out.push_str(&text);
                continue;
            }
            let (prev, prev_edit) = &entries[pos - 1];
            if prev + 1 == *id && edit.is_none() && prev_edit.is_none() {
                if let Some(lead) = &slide.lead {
                    out.push_str(lead);
                }
                out.push_str(&text);
                continue;
            }
            // Without a blank line before it, `text\n---` would become a setext heading.
            self.ensure_blank_line(&mut out);
            if !heading_start {
                match &slide.lead {
                    Some(lead) if lead.ends_with('\n') => out.push_str(lead),
                    Some(lead) => {
                        out.push_str(lead);
                        out.push_str(self.newline);
                    }
                    None => {
                        out.push_str("---");
                        out.push_str(self.newline);
                    }
                }
            }
            out.push_str(&text);
        }
        if let Some((last, edit)) = entries.last() {
            if *last != n - 1 || edit.is_some() {
                // keep the end of the file (trailing line break or not) as it was
                let tail = &self.source[self.source.trim_end().len()..];
                let keep = out.trim_end().len().max(self.head.len());
                out.truncate(keep);
                out.push_str(tail);
            }
        }
        out
    }

    /// Text of an edited slide: the new content with the blank lines that surrounded
    /// the original and line breaks in the file's style.
    fn rewrap(&self, slide: &Slide, body: &str) -> String {
        let original = slide.text.as_str();
        let start = original.len() - original.trim_start().len();
        let end = (start + original.trim().len()).max(start);
        let (before, after) = if original.trim().is_empty() {
            ("", original)
        } else {
            (&original[..start], &original[end..])
        };
        let mut body = body.trim().replace("\r\n", "\n");
        if self.newline != "\n" {
            body = body.replace('\n', self.newline);
        }
        let mut text = format!("{before}{body}{after}");
        if !text.ends_with('\n') {
            text.push_str(self.newline);
        }
        text
    }

    /// Number of slides the file composed from `entries` would have.
    pub fn count(&self, entries: &[Entry]) -> usize {
        let text = self.compose(entries);
        scan(&text, self.head.len().min(text.len()), &self.divider).len()
    }

    fn ensure_blank_line(&self, out: &mut String) {
        if out.is_empty() {
            return;
        }
        if !out.ends_with('\n') {
            out.push_str(self.newline);
        }
        let body = &out[..out.len() - 1];
        let last_line = body.rsplit('\n').next().unwrap_or("");
        if !last_line.trim().is_empty() {
            out.push_str(self.newline);
        }
    }
}

/// Display title: the first heading of the slide or, failing that, the first line of text.
pub fn title(text: &str) -> String {
    let mut in_comment = false;
    let mut fallback = None;
    for line in text.lines() {
        let t = line.trim();
        if in_comment {
            in_comment = !t.contains("-->");
            continue;
        }
        if t.starts_with("<!--") {
            in_comment = !t.contains("-->");
            continue;
        }
        if t.is_empty() || t.starts_with("![") || t.starts_with("```") || t.starts_with('<') {
            continue;
        }
        if atx_level(t).is_some() {
            return clean_inline(t.trim_start_matches('#').trim_end_matches('#'));
        }
        if fallback.is_none() {
            fallback = Some(clean_inline(t));
        }
    }
    fallback.unwrap_or_default()
}

fn clean_inline(s: &str) -> String {
    s.replace("**", "")
        .replace("__", "")
        .replace('`', "")
        .trim()
        .to_string()
}

/// Directives of the slide whose effect extends to the following slides.
pub fn forward_directives(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find("<!--") {
        let after = &rest[open + 4..];
        let close = after.find("-->").unwrap_or(after.len());
        for line in after[..close].lines() {
            let Some((key, _)) = line.split_once(':') else {
                continue;
            };
            let key = key.trim();
            if FORWARD_DIRECTIVES.contains(&key) && !found.iter().any(|k| k == key) {
                found.push(key.to_string());
            }
        }
        rest = &after[close..];
    }
    found
}

fn front_matter_len(source: &str) -> usize {
    let bom = if source.starts_with('\u{feff}') { 3 } else { 0 };
    let mut lines = source[bom..].split_inclusive('\n');
    match lines.next() {
        Some(first) if first.trim_end() == "---" => {
            let mut off = bom + first.len();
            for line in lines {
                off += line.len();
                if line.trim_end() == "---" {
                    return off;
                }
            }
            bom
        }
        _ => bom,
    }
}

/// Heading levels that start a slide (`headingDivider`), from the front matter or a comment.
fn heading_divider(head: &str, body: &str) -> Vec<u8> {
    let mut levels = Vec::new();
    for line in head.lines().chain(body.lines()) {
        let t = line.trim().trim_start_matches("<!--").trim();
        let Some(value) = t.strip_prefix("headingDivider:") else {
            continue;
        };
        let value = value.trim_end_matches("-->").trim();
        let digits: Vec<u8> = value
            .chars()
            .filter_map(|c| c.to_digit(10))
            .map(|d| d as u8)
            .filter(|d| (1..=6).contains(d))
            .collect();
        levels = if value.starts_with('[') {
            digits
        } else {
            digits.first().map(|&n| (1..=n).collect()).unwrap_or_default()
        };
    }
    levels
}

fn split_indent(line: &str) -> (usize, &str) {
    let mut cols = 0;
    for (i, c) in line.char_indices() {
        match c {
            ' ' => cols += 1,
            '\t' => cols += 4,
            _ => return (cols, line[i..].trim_end()),
        }
    }
    (cols, "")
}

/// Thematic break (`---`, `***`, `___`, `- - -`). The boolean tells whether it has no inner spaces.
fn thematic_break(rest: &str) -> Option<(char, bool)> {
    let ch = rest.chars().next()?;
    if !matches!(ch, '-' | '*' | '_') {
        return None;
    }
    let mut count = 0;
    let mut pure = true;
    for c in rest.chars() {
        if c == ch {
            count += 1;
        } else if c == ' ' || c == '\t' {
            pure = false;
        } else {
            return None;
        }
    }
    (count >= 3).then_some((ch, pure))
}

fn fence_opens(rest: &str) -> Option<(char, usize)> {
    let ch = rest.chars().next()?;
    if ch != '`' && ch != '~' {
        return None;
    }
    let count = rest.chars().take_while(|&c| c == ch).count();
    if count < 3 || (ch == '`' && rest[count..].contains('`')) {
        return None;
    }
    Some((ch, count))
}

fn fence_closes(rest: &str, (ch, open): (char, usize)) -> bool {
    let count = rest.chars().take_while(|&c| c == ch).count();
    count >= open && rest[count..].trim().is_empty()
}

/// HTML blocks that only end at the closing mark (they may contain `---` and blank lines).
fn raw_html_opens(rest: &str) -> Option<(usize, &'static str)> {
    if rest.starts_with("<!--") {
        return Some((4, "-->"));
    }
    let lower = rest.to_ascii_lowercase();
    for (open, end) in [
        ("<style", "</style>"),
        ("<script", "</script>"),
        ("<pre", "</pre>"),
    ] {
        if let Some(after) = lower.strip_prefix(open) {
            if after.is_empty() || after.starts_with([' ', '\t', '>']) {
                return Some((open.len(), end));
            }
        }
    }
    None
}

fn is_container_marker(rest: &str) -> bool {
    let followed_by_space = |s: &str| s.is_empty() || s.starts_with([' ', '\t']);
    if rest.starts_with('>') {
        return true;
    }
    if rest.starts_with(['-', '+', '*']) {
        return followed_by_space(&rest[1..]);
    }
    let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    if (1..=9).contains(&digits) && rest[digits..].starts_with(['.', ')']) {
        return followed_by_space(&rest[digits + 1..]);
    }
    false
}

fn atx_level(rest: &str) -> Option<u8> {
    let count = rest.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&count) && (rest.len() == count || rest[count..].starts_with([' ', '\t']))
    {
        Some(count as u8)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASIC: &str = "---\nmarp: true\ntheme: default\n---\n\n# Um\n\ntexto\n\n---\n\n# Dois\n\n- a\n- b\n\n---\n\n# Três\n\nfim\n";

    fn reorder(deck: &Deck, order: &[usize]) -> String {
        let entries: Vec<Entry> = order.iter().map(|&id| (id, None)).collect();
        deck.compose(&entries)
    }

    fn edit(id: usize, text: &str) -> Entry {
        (id, Some(text.to_string()))
    }

    fn texts(deck: &Deck) -> Vec<String> {
        deck.slides.iter().map(|s| s.text.trim().to_string()).collect()
    }

    /// Composes in the given order, re-reads and checks the slides came out in that order.
    fn check_order(source: &str, order: &[usize]) {
        let deck = Deck::parse(source.to_string());
        let expected: Vec<String> = order
            .iter()
            .map(|&i| deck.slides[i].text.trim().to_string())
            .collect();
        let out = reorder(&deck, order);
        let again = Deck::parse(out.clone());
        assert_eq!(texts(&again), expected, "ordem {order:?}\n{out}");
        assert_eq!(again.head, deck.head);
    }

    #[test]
    fn splits_front_matter_and_slides() {
        let deck = Deck::parse(BASIC.to_string());
        assert_eq!(deck.head, "---\nmarp: true\ntheme: default\n---\n");
        assert_eq!(
            texts(&deck),
            ["# Um\n\ntexto", "# Dois\n\n- a\n- b", "# Três\n\nfim"]
        );
    }

    #[test]
    fn identity_is_byte_exact() {
        for src in [BASIC, "# só uma", "", "a\r\n\r\n---\r\n\r\nb", "---\n---\n"] {
            let deck = Deck::parse(src.to_string());
            let order: Vec<usize> = (0..deck.slides.len()).collect();
            assert_eq!(reorder(&deck, &order), src);
        }
    }

    #[test]
    fn every_permutation_round_trips() {
        for order in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            check_order(BASIC, &order);
            check_order("# A\n\ntexto\n\n---\n\n# B\n\n---\n\nparágrafo final", &order);
        }
    }

    #[test]
    fn moved_paragraph_does_not_become_setext_heading() {
        let src = "---\nmarp: true\n---\n# A\n---\núltima linha sem quebra";
        let deck = Deck::parse(src.to_string());
        assert_eq!(deck.slides.len(), 2);
        let out = reorder(&deck, &[1, 0]);
        assert_eq!(out, "---\nmarp: true\n---\núltima linha sem quebra\n\n---\n# A");
        check_order(src, &[1, 0]);
    }

    #[test]
    fn neighbours_keep_their_original_bytes() {
        let deck = Deck::parse(BASIC.to_string());
        // 1 and 2 stay neighbours: the text between them does not change
        let out = reorder(&deck, &[1, 2, 0]);
        assert!(out.contains("- a\n- b\n\n---\n\n# Três\n\nfim\n"));
        assert!(out.ends_with("texto\n"));
    }

    #[test]
    fn ignores_separators_inside_code_and_comments() {
        let src = "# A\n\n```yaml\n---\nx: 1\n---\n```\n\n~~~\n---\n~~~\n\n<!--\nnota\n\n---\n-->\n\n<style>\n\n---\n</style>\n\n---\n\n# B\n";
        let deck = Deck::parse(src.to_string());
        assert_eq!(deck.slides.len(), 2);
        check_order(src, &[1, 0]);
    }

    #[test]
    fn setext_heading_is_not_a_separator() {
        let deck = Deck::parse("Título\n---\n\ntexto\n\n---\n\nfim\n".to_string());
        assert_eq!(texts(&deck), ["Título\n---\n\ntexto", "fim"]);
    }

    #[test]
    fn separator_right_after_list_heading_or_comment() {
        for src in [
            "- item\n---\nB\n",
            "# A\n---\nB\n",
            "<!-- _class: lead -->\n---\nB\n",
            "> citação\n---\nB\n",
            "texto\n***\nB\n",
            "texto\n\n- - -\nB\n",
        ] {
            let deck = Deck::parse(src.to_string());
            assert_eq!(deck.slides.len(), 2, "{src:?}");
            check_order(src, &[1, 0]);
        }
    }

    #[test]
    fn nested_or_indented_rules_do_not_split() {
        for src in ["- item\n\n  ---\n\n  mais\n", "texto\n\n    ---\n"] {
            assert_eq!(Deck::parse(src.to_string()).slides.len(), 1, "{src:?}");
        }
    }

    #[test]
    fn crlf_is_preserved() {
        let src = "---\r\nmarp: true\r\n---\r\n\r\n# A\r\n\r\n---\r\n\r\n# B\r\n\r\n---\r\n\r\nC";
        let deck = Deck::parse(src.to_string());
        assert_eq!(deck.slides.len(), 3);
        let out = reorder(&deck, &[2, 0, 1]);
        assert!(!out.replace("\r\n", "").contains('\n'), "{out:?}");
        check_order(src, &[2, 0, 1]);
    }

    #[test]
    fn no_front_matter() {
        let src = "# A\n\n---\n\n# B\n";
        let deck = Deck::parse(src.to_string());
        assert_eq!(deck.head, "");
        assert_eq!(deck.slides.len(), 2);
        check_order(src, &[1, 0]);
    }

    #[test]
    fn empty_slide_first_without_front_matter_is_not_front_matter() {
        let src = "# A\n\n---\n\n---\n\n# C\n";
        check_order(src, &[1, 0, 2]);
        check_order(src, &[1, 2, 0]);
    }

    #[test]
    fn heading_divider_splits_on_headings() {
        let src = "---\nmarp: true\nheadingDivider: 2\n---\n\n# Capa\n\n## Um\n\ntexto\n\n### sub\n\n## Dois\n\n---\n\n## Três\nfim\n";
        let deck = Deck::parse(src.to_string());
        // `---` followed by a divider heading yields an empty slide in Marp
        assert_eq!(
            texts(&deck),
            ["# Capa", "## Um\n\ntexto\n\n### sub", "## Dois", "", "## Três\nfim"]
        );
        let starts: Vec<bool> = deck.slides.iter().map(|s| s.heading_start).collect();
        assert_eq!(starts, [true, true, true, false, true]);
        for order in [[4, 3, 2, 1, 0], [1, 0, 4, 3, 2], [2, 0, 1, 3, 4], [0, 4, 1, 2, 3]] {
            check_order(src, &order);
        }
        check_order(src, &[4, 2, 1, 0]);
    }

    #[test]
    fn heading_divider_ignores_comments_before_first_heading() {
        let deck = Deck::parse("<!-- headingDivider: [1, 3] -->\n# A\n## x\n### B\n# C\n".to_string());
        assert_eq!(texts(&deck)[1..], ["### B", "# C"]);
        assert!(deck.slides[0].lead_comment);
        assert!(!deck.slides[1].lead_comment);
    }

    #[test]
    fn heading_divider_after_comment_only_slide() {
        let src = "---\nheadingDivider: 1\n---\n\ntexto antes\n\n# A\n\n---\n\n<!-- c -->\n# B\n";
        let deck = Deck::parse(src.to_string());
        assert_eq!(texts(&deck), ["texto antes", "# A", "<!-- c -->", "# B"]);
        check_order(src, &[3, 0, 2, 1]);
    }

    #[test]
    fn heading_divider_setext_headings() {
        let src = "---\nheadingDivider: 2\n---\n# A\n\nSetext\ndois\n---\n\ntexto\n\nUm\n===\n\n## B\n";
        let deck = Deck::parse(src.to_string());
        assert_eq!(texts(&deck), ["# A", "Setext\ndois\n---\n\ntexto", "Um\n===", "## B"]);
        check_order(src, &[2, 3, 1, 0]);
    }

    #[test]
    fn delete_and_duplicate() {
        check_order(BASIC, &[0, 2]);
        check_order(BASIC, &[2]);
        check_order(BASIC, &[0, 0, 1, 2, 2]);
        let deck = Deck::parse(BASIC.to_string());
        assert_eq!(reorder(&deck, &[]), deck.head);
    }

    #[test]
    fn edited_slide_keeps_surrounding_layout() {
        let deck = Deck::parse(BASIC.to_string());
        let out = deck.compose(&[(0, None), edit(1, "# Novo\n\ntexto novo\n\n"), (2, None)]);
        assert_eq!(out, BASIC.replace("# Dois\n\n- a\n- b", "# Novo\n\ntexto novo"));
        // editing to the same text changes nothing
        let same = deck.compose(&[(0, None), edit(1, "# Dois\n\n- a\n- b"), (2, None)]);
        assert_eq!(same, BASIC);
    }

    #[test]
    fn edited_slide_can_move_split_and_end_the_file() {
        let deck = Deck::parse(BASIC.to_string());
        let out = deck.compose(&[(2, None), edit(0, "parágrafo"), edit(1, "A\n\n---\n\nB")]);
        let again = Deck::parse(out.clone());
        assert_eq!(texts(&again), ["# Três\n\nfim", "parágrafo", "A", "B"]);
        assert!(out.ends_with("B\n") && !out.ends_with("B\n\n"));
        assert_eq!(deck.count(&[(2, None), edit(0, "parágrafo")]), 2);
    }

    #[test]
    fn edited_paragraph_before_separator_is_not_setext() {
        let src = "# A\n---\n# B\n---\n# C";
        let deck = Deck::parse(src.to_string());
        let out = deck.compose(&[edit(0, "texto"), edit(1, ""), edit(2, "fim")]);
        assert_eq!(texts(&Deck::parse(out.clone())), ["texto", "", "fim"]);
        assert!(!out.ends_with('\n'));
        // without front matter, the first slide cannot start with `---`
        let first = deck.compose(&[edit(0, "---\nx: 1\n---\ny"), (1, None), (2, None)]);
        assert_eq!(Deck::parse(first).head, "");
    }

    #[test]
    fn edited_slide_uses_file_line_endings() {
        let deck = Deck::parse("a\r\n\r\n---\r\n\r\nb\r\n".to_string());
        let out = deck.compose(&[(0, None), edit(1, "um\ndois")]);
        assert_eq!(out, "a\r\n\r\n---\r\n\r\num\r\ndois\r\n");
    }

    #[test]
    fn edited_heading_divider_slide_gets_separator_when_heading_is_removed() {
        let src = "---\nheadingDivider: 1\n---\n# A\n\n# B\n\n# C\n";
        let deck = Deck::parse(src.to_string());
        let out = deck.compose(&[(0, None), edit(1, "sem título"), (2, None)]);
        assert_eq!(texts(&Deck::parse(out)), ["# A", "sem título", "# C"]);
        let out = deck.compose(&[(0, None), edit(1, "# Outro\n\ntexto"), (2, None)]);
        assert_eq!(texts(&Deck::parse(out.clone())), ["# A", "# Outro\n\ntexto", "# C"]);
        assert!(!out.contains("---\n# Outro"));
    }

    #[test]
    fn titles_and_directive_warnings() {
        assert_eq!(title("\n<!-- _class: lead -->\n\n## **Olá** `mundo`\n"), "Olá mundo");
        assert_eq!(title("![bg](a.png)\n\ntexto solto\n"), "texto solto");
        assert_eq!(title("\n\n"), "");
        assert_eq!(
            forward_directives("<!-- class: invert -->\n<!--\npaginate: true\n_footer: x\n-->"),
            ["class", "paginate"]
        );
        assert!(forward_directives("<!-- _class: lead -->\nnota: class: x").is_empty());
    }

    /// Checks the slide splitting against the real marp-cli. It is outside the regular
    /// `cargo test` because it needs `marp` in PATH: `cargo test -- --ignored`.
    #[test]
    #[ignore = "needs marp-cli in PATH"]
    fn slide_count_matches_marp_cli() {
        use std::process::Command;
        let fm = "---\nmarp: true\n---\n\n";
        let hd = "---\nmarp: true\nheadingDivider: 2\n---\n\n";
        let cases: Vec<String> = vec![
            BASIC.to_string(),
            format!("{fm}# A\n\n***\n\n# B\n\n___\n\n# C\n\n- - -\n\n# D\n"),
            format!("{fm}# A\n\n```\n---\n```\n\n---\n\n<!--\n---\n-->\n\n# B\n"),
            format!("{fm}Title\n---\n\ntext\n\n---\n\nend\n"),
            format!("{fm}- item\n---\nB\n"),
            format!("{fm}# heading\n---\nB\n"),
            format!("{fm}<!-- _class: lead -->\n---\nB\n"),
            format!("{fm}> quote\n---\nB\n"),
            format!("{fm}- item\n\n  ---\n\n  more\n\n---\n\nB\n"),
            format!("{fm}A\n\n    ---\n\nB\n"),
            format!("{fm}<style>\nsection {{ color: red; }}\n---\n</style>\n\n# A\n\n---\n\n# B\n"),
            format!("{fm}# A\n\n---\n---\n\n# B\n"),
            "# A\n\n---\n\n# B\n".to_string(),
            format!("{hd}# Cover\n\n## One\n\ntext\n\n### sub\n\n## Two\n\n---\n\n## Three\nend\n"),
            format!("{hd}<!-- note -->\n\n## One\n\n## Two\n"),
            format!("{hd}text before\n\n## A\n\n---\n\n<!-- c -->\n## B\n"),
            format!("{hd}intro\n\nSetext\n------\n\nmore\n\nTop\n===\n"),
            "---\nmarp: true\n---\n\n<!-- headingDivider: 1 -->\n\n# A\n\ntext\n\n# B\n\n## not a divider\n".to_string(),
        ];
        let dir = std::env::temp_dir().join(format!("marp-order-parity-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (n, source) in cases.iter().enumerate() {
            let input = dir.join(format!("case{n}.md"));
            let output = dir.join(format!("case{n}.html"));
            std::fs::write(&input, source).unwrap();
            let status = Command::new("marp")
                .arg("--no-stdin")
                .arg(&input)
                .arg("-o")
                .arg(&output)
                .output()
                .expect("marp-cli not found");
            assert!(status.status.success(), "marp failed on case {n}");
            let html = std::fs::read_to_string(&output).unwrap();
            let marp = html.matches("<section ").count();
            let ours = Deck::parse(source.clone()).slides.len();
            assert_eq!(ours, marp, "case {n}:\n{source}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
