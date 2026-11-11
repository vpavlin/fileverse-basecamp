//! The visual editor's format bridge. The view edits formatted text in Qt's
//! RichText mode (a QTextDocument); drafts are stored as Markdown. So:
//!
//!   Markdown --to_html--> HTML that Qt imports --(user edits)--> Qt's own HTML --to_md--> Markdown
//!
//! Qt's Markdown writer was not used for the way back: it garbles code blocks and
//! tables that follow lists or quotes. Qt's HTML is regular instead: one element
//! per text block (`<h1>`, `<p>`, `<li>`, one `<pre>` per code line, `<hr />`),
//! inline formats as `style` attributes. Block styles (heading, list, task, quote,
//! code) are changed here too (`set_block`), because QML has no block-format API:
//! the view says which text block the cursor is in, counted the way Qt counts.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::doc::ImageRef;
use crate::md::{segments, Segment};

// ── images ───────────────────────────────────────────────────────────────────

/// How images cross between Markdown (`![alt](fvimg:…)`) and the editor, whose
/// sandbox only loads local files from the view's own directory: `src_of` gives the
/// file the editor shows for a reference, `url_of` maps such a file back to the
/// reference. Installed by the caller around a conversion with `with_images`.
pub struct Images {
    pub src_of: Box<dyn Fn(&ImageRef) -> Option<String>>,
    pub url_of: Box<dyn Fn(&str) -> Option<String>>,
}

thread_local! {
    static IMAGES: std::cell::RefCell<Option<Images>> = const { std::cell::RefCell::new(None) };
}

pub fn with_images<T>(images: Images, f: impl FnOnce() -> T) -> T {
    IMAGES.with(|c| *c.borrow_mut() = Some(images));
    let out = f();
    IMAGES.with(|c| *c.borrow_mut() = None);
    out
}

fn image_src(r: &ImageRef) -> Option<String> {
    IMAGES.with(|c| c.borrow().as_ref().and_then(|i| (i.src_of)(r)))
}

fn image_url(src: &str) -> Option<String> {
    IMAGES.with(|c| c.borrow().as_ref().and_then(|i| (i.url_of)(src)))
}

// ── HTML in (Qt's output) ────────────────────────────────────────────────────

enum Tok {
    Open(String, Vec<(String, String)>),
    Close(String),
    Text(String),
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.find(';').filter(|e| *e <= 10);
        let Some(end) = end else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
            _ if ent.starts_with('#') => ent[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn tokenize(html: &str) -> Vec<Tok> {
    let mut toks = vec![];
    let b = html.as_bytes();
    let mut i = 0;
    let mut text_start = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        if i > text_start {
            toks.push(Tok::Text(decode_entities(&html[text_start..i])));
        }
        // comment / doctype
        if html[i..].starts_with("<!--") {
            i = html[i..].find("-->").map(|e| i + e + 3).unwrap_or(b.len());
            text_start = i;
            continue;
        }
        let mut j = i + 1;
        let mut quote: Option<u8> = None;
        while j < b.len() {
            match (quote, b[j]) {
                (Some(q), c) if c == q => quote = None,
                (None, b'"') | (None, b'\'') => quote = Some(b[j]),
                (None, b'>') => break,
                _ => {}
            }
            j += 1;
        }
        let inner = &html[i + 1..j.min(b.len())];
        i = (j + 1).min(b.len());
        text_start = i;
        if inner.starts_with('!') || inner.starts_with('?') {
            continue;
        }
        if let Some(name) = inner.strip_prefix('/') {
            toks.push(Tok::Close(name.trim().to_ascii_lowercase()));
            continue;
        }
        let self_closing = inner.trim_end().ends_with('/');
        let inner = inner.trim_end().trim_end_matches('/');
        let name_end = inner.find(|c: char| c.is_whitespace()).unwrap_or(inner.len());
        let name = inner[..name_end].to_ascii_lowercase();
        let attrs = parse_attrs(&inner[name_end..]);
        // raw-text elements: skip their content
        if name == "style" || name == "script" || name == "head" {
            let close = format!("</{name}");
            let lower = html[i..].to_ascii_lowercase();
            i = lower.find(&close).map(|e| i + e).unwrap_or(b.len());
            i = html[i..].find('>').map(|e| i + e + 1).unwrap_or(b.len());
            text_start = i;
            continue;
        }
        toks.push(Tok::Open(name.clone(), attrs));
        if self_closing || name == "br" || name == "hr" || name == "meta" || name == "img" {
            toks.push(Tok::Close(name));
        }
    }
    if text_start < b.len() {
        toks.push(Tok::Text(decode_entities(&html[text_start..])));
    }
    toks
}

fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let mut out = vec![];
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let ks = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' {
            i += 1;
        }
        if ks == i {
            i += 1;
            continue;
        }
        let key = s[ks..i].to_ascii_lowercase();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut val = String::new();
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                let vs = i + 1;
                i = vs;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                val = decode_entities(&s[vs..i.min(b.len())]);
                i += 1;
            } else {
                let vs = i;
                while i < b.len() && !b[i].is_ascii_whitespace() {
                    i += 1;
                }
                val = decode_entities(&s[vs..i]);
            }
        }
        out.push((key, val));
    }
    out
}

fn attr<'a>(a: &'a [(String, String)], k: &str) -> &'a str {
    a.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str()).unwrap_or("")
}

// ── the block model both directions share ────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
enum Block {
    /// `inline` is Markdown for the block's text (marks and links included).
    Heading(u8, String),
    Para(String),
    Quote(String),
    /// `list` tells separate lists apart (a new top-level list starts a new number).
    Item { list: usize, ordered: bool, depth: usize, task: Option<bool>, inline: String },
    Code(Vec<String>),
    Hr,
    /// Header row first.
    Table(Vec<Vec<String>>),
    /// Side by side (dDocs' columns): each column's blocks; the editor shows them as a
    /// borderless one-row table.
    Columns { layout: String, cols: Vec<Vec<Block>> },
    /// An empty text block (Qt keeps them; Markdown drops them).
    Empty,
}

impl Block {
    /// How many pieces of the view's plain text this block is. The view splits Qt's text
    /// at paragraph separators (U+2029) and at the table markers Qt puts there (U+FDD0
    /// before each cell, U+FDD1 after the last): a code block is one piece per line, a
    /// table or columns one per text block in each cell (the empty piece Qt may keep
    /// after a table is counted by `locate`).
    fn units(&self) -> usize {
        match self {
            Block::Code(lines) => lines.len().max(1),
            Block::Table(rows) => rows.iter().map(|r| r.len()).sum::<usize>(),
            Block::Columns { cols, .. } => cols.iter().map(|c| c.iter().map(Block::units).sum::<usize>().max(1)).sum::<usize>(),
            _ => 1,
        }
    }
}

#[derive(Default, Clone, Copy, PartialEq)]
struct Style {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
}

fn style_of(css: &str, base: Style) -> Style {
    let mut s = base;
    let css = css.to_ascii_lowercase();
    for decl in css.split(';') {
        let Some((k, v)) = decl.split_once(':') else { continue };
        let (k, v) = (k.trim(), v.trim());
        match k {
            "font-weight" => s.bold = v == "bold" || v.parse::<u32>().map(|w| w >= 600).unwrap_or(false),
            "font-style" => s.italic = v == "italic" || v == "oblique",
            "text-decoration" => {
                if v.contains("line-through") {
                    s.strike = true
                }
            }
            "font-family" => s.code = ["monospace", "courier", "mono", "consolas", "menlo"].iter().any(|m| v.contains(m)),
            _ => {}
        }
    }
    s
}

/// Escape text so Markdown reads it back as the same text.
fn escape_md(t: &str) -> String {
    let mut o = String::with_capacity(t.len());
    for c in t.chars() {
        if matches!(c, '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '~' | '|') {
            o.push('\\');
        }
        o.push(c);
    }
    o
}

/// Text runs with styles -> inline Markdown. Marks are opened and closed around
/// each run's trimmed text, so `**bold **` cannot happen.
struct InlineBuf {
    runs: Vec<(Style, Option<String>, String)>,
    /// Markdown to keep as is (an image), at the position of the run with that index.
    raw: Vec<(usize, String)>,
}

impl InlineBuf {
    fn new() -> Self {
        InlineBuf { runs: vec![], raw: vec![] }
    }
    fn push_raw(&mut self, md: String) {
        self.raw.push((self.runs.len(), md));
        // a boundary, so text after it does not merge into the run before
        self.runs.push((Style::default(), None, String::new()));
    }
    fn push(&mut self, st: Style, link: Option<String>, text: &str) {
        if text.is_empty() {
            return;
        }
        let boundary = self.raw.last().map(|(k, _)| *k + 1 == self.runs.len()).unwrap_or(false);
        if let Some(last) = self.runs.last_mut().filter(|_| !boundary) {
            if last.0 == st && last.1 == link {
                last.2.push_str(text);
                return;
            }
        }
        self.runs.push((st, link, text.to_string()));
    }
    fn markdown(&self, plain_bold_ok: bool) -> String {
        let mut out = String::new();
        let mut i = 0;
        let raw_at = |i: usize| self.raw.iter().filter(move |(k, _)| *k == i).map(|(_, m)| m.as_str());
        while i < self.runs.len() {
            for m in raw_at(i) {
                // a line break only between text: Qt writes an empty line as <br />, and
                // one at the start or end of a block means nothing in Markdown
                if m == "\\\n" && (out.trim().is_empty() || self.runs[i..].iter().all(|r| r.2.trim_matches(|c: char| c.is_whitespace() || c == '\u{200b}').is_empty())) {
                    continue;
                }
                out.push_str(m);
            }
            // a link spans consecutive runs with the same href
            if let Some(href) = &self.runs[i].1 {
                let mut j = i;
                let mut label = String::new();
                while j < self.runs.len() && self.runs[j].1.as_deref() == Some(href.as_str()) {
                    label.push_str(&mark(&self.runs[j].0, &self.runs[j].2, plain_bold_ok));
                    j += 1;
                }
                // a bare address (its text is the address, unstyled) stays bare
                let plain: String = self.runs[i..j].iter().map(|r| r.2.as_str()).collect();
                let unstyled = self.runs[i..j].iter().all(|r| r.0 == Style::default());
                if unstyled && plain == *href && (href.starts_with("https://") || href.starts_with("http://")) && !href.contains(['*', '_', '[', ']', '<', '>', '`', '(', ')', ' ', '\\']) {
                    out.push_str(href);
                } else {
                    out.push_str(&format!("[{label}]({})", href.replace(' ', "%20").replace(')', "%29")));
                }
                i = j;
                continue;
            }
            out.push_str(&mark(&self.runs[i].0, &self.runs[i].2, plain_bold_ok));
            i += 1;
        }
        // not Unicode whitespace: no-break spaces at the start are an indent (Tab in the editor)
        out.trim_matches(|c: char| c == ' ' || c == '\n' || c == '\t' || c == '\r').to_string()
    }
}

fn mark(st: &Style, text: &str, bold_ok: bool) -> String {
    if text.trim().is_empty() {
        return text.replace('\n', " ");
    }
    let lead = &text[..text.len() - text.trim_start().len()];
    let trail = &text[text.trim_end().len()..];
    let core = text.trim();
    let mut s = if st.code { format!("`{}`", core.replace('`', "'")) } else { escape_md(core) };
    if st.strike {
        s = format!("~~{s}~~");
    }
    if st.italic {
        s = format!("*{s}*");
    }
    if st.bold && bold_ok {
        s = format!("**{s}**");
    }
    format!("{lead}{s}{trail}").replace('\n', " ")
}

/// Qt's HTML (from `TextArea.text` in RichText mode) -> blocks.
fn blocks_from_html(html: &str) -> Vec<Block> {
    let toks = tokenize(html);
    // inside <body> only (Qt always writes one)
    let start = toks.iter().position(|t| matches!(t, Tok::Open(n, _) if n == "body")).map(|b| b + 1).unwrap_or(0);
    blocks_from_toks(&toks[start..])
}

fn blocks_from_toks(toks: &[Tok]) -> Vec<Block> {
    let mut blocks = vec![];
    // list stack: (ordered)
    let mut lists: Vec<bool> = vec![];
    let mut list_id = 0usize;
    let mut quote_depth = 0usize;
    let mut i = 0;
    let mut code_run: Vec<String> = vec![];
    let flush_code = |code_run: &mut Vec<String>, blocks: &mut Vec<Block>| {
        if !code_run.is_empty() {
            blocks.push(Block::Code(std::mem::take(code_run)));
        }
    };
    while i < toks.len() {
        match &toks[i] {
            Tok::Open(n, a) if n == "ul" || n == "ol" => {
                flush_code(&mut code_run, &mut blocks);
                let _ = a;
                if lists.is_empty() {
                    list_id += 1;
                }
                lists.push(n == "ol");
                i += 1;
            }
            Tok::Close(n) if n == "ul" || n == "ol" => {
                lists.pop();
                i += 1;
            }
            Tok::Open(n, _) if n == "blockquote" => {
                quote_depth += 1;
                i += 1;
            }
            Tok::Close(n) if n == "blockquote" => {
                quote_depth = quote_depth.saturating_sub(1);
                i += 1;
            }
            Tok::Open(n, a) if matches!(n.as_str(), "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li" | "pre") => {
                let name = n.clone();
                let css = attr(a, "style").to_string();
                let class = attr(a, "class").to_string();
                let (buf, plain, next) = read_inline(toks, i + 1, &name);
                i = next;
                let empty = css.contains("-qt-paragraph-type:empty") || plain.trim().is_empty();
                if name == "pre" {
                    // the editor indents code with no-break spaces (Qt collapses inserted spaces)
                    code_run.push(plain.trim_end_matches('\n').replace('\u{a0}', " "));
                    continue;
                }
                flush_code(&mut code_run, &mut blocks);
                let block = match name.as_str() {
                    "li" => Block::Item {
                        list: list_id,
                        ordered: *lists.last().unwrap_or(&false),
                        depth: lists.len().max(1) - 1,
                        task: match class.as_str() {
                            "checked" => Some(true),
                            "unchecked" => Some(false),
                            _ => None,
                        },
                        inline: buf.markdown(true),
                    },
                    h if h.starts_with('h') => {
                        let level: u8 = h[1..].parse().unwrap_or(1);
                        if empty { Block::Empty } else { Block::Heading(level.min(3), buf.markdown(false)) }
                    }
                    // Qt turns <blockquote> into a paragraph indented on both sides; an empty
                    // line in a quote (Enter in a quote) is still the quote's, so that a second
                    // Enter there can end the quote
                    _ if quote_depth > 0 || (css.contains("margin-left:40px") && css.contains("margin-right:40px")) => {
                        if empty { Block::Quote(ZWSP.to_string()) } else { Block::Quote(buf.markdown(true)) }
                    }
                    _ if empty => Block::Empty,
                    _ => Block::Para(buf.markdown(true)),
                };
                blocks.push(block);
            }
            Tok::Open(n, _) if n == "hr" => {
                flush_code(&mut code_run, &mut blocks);
                blocks.push(Block::Hr);
                i += 1;
            }
            Tok::Open(n, a) if n == "table" => {
                flush_code(&mut code_run, &mut blocks);
                // a borderless single-row table is how the editor shows columns
                if attr(a, "border") == "0" {
                    if let Some((cols, widths, next)) = read_columns(toks, i + 1) {
                        let layout = match (widths.first(), widths.last()) {
                            (Some(&f), Some(&l)) if widths.len() >= 2 && f > l * 1.5 => "align-right",
                            (Some(&f), Some(&l)) if widths.len() >= 2 && l > f * 1.5 => "align-left",
                            _ => "align-center",
                        };
                        blocks.push(Block::Columns { layout: layout.into(), cols });
                        i = next;
                        continue;
                    }
                }
                let (rows, next) = read_table(toks, i + 1);
                i = next;
                if !rows.is_empty() {
                    blocks.push(Block::Table(rows));
                }
            }
            Tok::Text(t) if !t.trim().is_empty() && lists.is_empty() => {
                // stray text at body level: keep it as a paragraph
                flush_code(&mut code_run, &mut blocks);
                blocks.push(Block::Para(escape_md(t.trim())));
                i += 1;
            }
            _ => i += 1,
        }
    }
    flush_code(&mut code_run, &mut blocks);
    blocks
}

/// Read inline content until the element `end` closes; nested lists end an `<li>`'s own text.
fn read_inline(toks: &[Tok], mut i: usize, end: &str) -> (InlineBuf, String, usize) {
    let mut buf = InlineBuf::new();
    let mut plain = String::new();
    let mut styles: Vec<(String, Style)> = vec![];
    let mut link: Option<String> = None;
    let base = Style::default();
    while i < toks.len() {
        let cur = styles.last().map(|(_, s)| *s).unwrap_or(base);
        match &toks[i] {
            Tok::Close(n) if n == end => return (buf, plain, i + 1),
            // a nested list inside <li>: the item's own text ends here
            Tok::Open(n, _) if end == "li" && (n == "ul" || n == "ol") => return (buf, plain, i),
            // a block element starting means the current one ended without a close tag
            Tok::Open(n, _) if matches!(n.as_str(), "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "pre" | "table") && n != end => {
                if end != "li" {
                    return (buf, plain, i);
                }
                i += 1;
            }
            Tok::Open(n, a) if n == "img" => {
                if let Some(url) = image_url(attr(a, "src")) {
                    buf.push_raw(format!("![{}]({url})", attr(a, "alt").replace(['[', ']'], "")));
                    plain.push_str("[image]");
                }
                i += 1;
            }
            Tok::Open(n, a) => {
                let st = match n.as_str() {
                    "b" | "strong" => Style { bold: true, ..cur },
                    "i" | "em" => Style { italic: true, ..cur },
                    "s" | "del" | "strike" => Style { strike: true, ..cur },
                    "code" | "tt" => Style { code: true, ..cur },
                    _ => style_of(attr(a, "style"), cur),
                };
                if n == "a" {
                    let h = attr(a, "href");
                    if !h.is_empty() {
                        link = Some(h.to_string());
                    }
                }
                if n == "br" {
                    // Shift+Enter: a line break inside the paragraph (Markdown's backslash break)
                    buf.push_raw("\\\n".into());
                    plain.push('\n');
                } else {
                    styles.push((n.clone(), st));
                }
                i += 1;
            }
            Tok::Close(n) => {
                if n == "a" {
                    link = None;
                }
                if let Some(pos) = styles.iter().rposition(|(name, _)| name == n) {
                    styles.truncate(pos);
                }
                i += 1;
            }
            Tok::Text(t) => {
                // Qt pretty-prints with newlines between tags; inside text, white-space is pre-wrap
                let t = if t.trim().is_empty() && t.contains('\n') { String::new() } else { t.replace('\n', " ") };
                // the placeholder `set_block` puts in empty blocks
                let t = t.replace(ZWSP, "");
                buf.push(cur, link.clone(), &t);
                plain.push_str(&t);
                i += 1;
            }
        }
    }
    (buf, plain, i)
}

/// A one-row table read as columns: each cell's blocks and its width (%), and the
/// token after the table. `None` when it has more than one row.
fn read_columns(toks: &[Tok], start: usize) -> Option<(Vec<Vec<Block>>, Vec<f64>, usize)> {
    let mut i = start;
    let mut rows = 0;
    let mut cols = vec![];
    let mut widths = vec![];
    let mut depth = 0usize; // nested tables
    while i < toks.len() {
        match &toks[i] {
            Tok::Open(n, _) if n == "table" => depth += 1,
            Tok::Close(n) if n == "table" => {
                if depth == 0 {
                    return (rows == 1 && !cols.is_empty()).then_some((cols, widths, i + 1));
                }
                depth -= 1;
            }
            Tok::Open(n, _) if n == "tr" && depth == 0 => rows += 1,
            Tok::Open(n, a) if (n == "td" || n == "th") && depth == 0 => {
                widths.push(attr(a, "width").trim_end_matches('%').parse::<f64>().unwrap_or(0.0));
                // the cell's tokens, up to its own closing tag
                let (from, mut j, mut d) = (i + 1, i + 1, 0usize);
                while j < toks.len() {
                    match &toks[j] {
                        Tok::Open(t, _) if t == "table" => d += 1,
                        Tok::Close(t) if t == "table" => d = d.saturating_sub(1),
                        Tok::Close(t) if (t == "td" || t == "th") && d == 0 => break,
                        _ => {}
                    }
                    j += 1;
                }
                cols.push(blocks_from_toks(&toks[from..j]));
                i = j;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn read_table(toks: &[Tok], mut i: usize) -> (Vec<Vec<String>>, usize) {
    let mut rows: Vec<Vec<String>> = vec![];
    while i < toks.len() {
        match &toks[i] {
            Tok::Close(n) if n == "table" => return (rows, i + 1),
            Tok::Open(n, _) if n == "tr" => {
                rows.push(vec![]);
                i += 1;
            }
            Tok::Open(n, _) if n == "td" || n == "th" => {
                let name = n.clone();
                // a cell holds paragraphs; join them
                let mut j = i + 1;
                let mut parts = vec![];
                loop {
                    match toks.get(j) {
                        None => break,
                        Some(Tok::Close(c)) if *c == name => {
                            j += 1;
                            break;
                        }
                        Some(Tok::Open(p, _)) if p == "p" => {
                            let (buf, _, next) = read_inline(toks, j + 1, "p");
                            // Qt writes the header row bold: that is the header, not a mark
                            parts.push(buf.markdown(name != "th" && rows.len() > 1));
                            j = next;
                        }
                        Some(Tok::Text(t)) if !t.trim().is_empty() => {
                            parts.push(escape_md(t.trim()));
                            j += 1;
                        }
                        _ => j += 1,
                    }
                }
                let cell = parts.join(" ").replace('|', "\\|");
                if let Some(r) = rows.last_mut() {
                    r.push(cell);
                }
                i = j;
            }
            _ => i += 1,
        }
    }
    (rows, i)
}

fn blocks_to_md(blocks: &[Block]) -> String {
    let mut out = String::new();
    let mut prev_list: Option<usize> = None;
    let mut counters: Vec<usize> = vec![];
    for b in blocks {
        let list = match b {
            Block::Item { list, .. } => Some(*list),
            _ => None,
        };
        let same_list = list.is_some() && list == prev_list;
        if !out.is_empty() {
            out.push_str(if same_list { "\n" } else { "\n\n" });
        }
        if !same_list {
            counters.clear();
        }
        match b {
            Block::Heading(l, t) => out.push_str(&format!("{} {t}", "#".repeat(*l as usize))),
            Block::Para(t) => out.push_str(&guard_start(t)),
            Block::Quote(t) => out.push_str(&format!("> {t}")),
            Block::Item { ordered, depth, task, inline, .. } => {
                counters.truncate(depth + 1);
                while counters.len() <= *depth {
                    counters.push(0);
                }
                counters[*depth] += 1;
                let marker = if *ordered { format!("{}.", counters[*depth]) } else { "-".into() };
                let pad = "    ".repeat(*depth);
                let tick = match task {
                    Some(true) => "[x] ",
                    Some(false) => "[ ] ",
                    None => "",
                };
                out.push_str(&format!("{pad}{marker} {tick}{inline}"));
            }
            // Qt keeps no language on a code block; a Mermaid diagram says what it is
            Block::Code(lines) => out.push_str(&format!("```{}\n{}\n```", if crate::diagram::looks_like_mermaid(&lines.join("\n")) { "mermaid" } else { "" }, lines.join("\n"))),
            Block::Hr => out.push_str("---"),
            Block::Table(rows) => {
                let cols = rows.iter().map(|r| r.len()).max().unwrap_or(1).max(1);
                let row = |r: &Vec<String>| {
                    let mut cells = r.clone();
                    cells.resize(cols, String::new());
                    format!("| {} |", cells.join(" | "))
                };
                out.push_str(&row(&rows[0]));
                out.push('\n');
                out.push_str(&format!("|{}", " --- |".repeat(cols)));
                for r in &rows[1..] {
                    out.push('\n');
                    out.push_str(&row(r));
                }
            }
            Block::Columns { layout, cols } => {
                out.push_str(&format!("::: columns {layout}\n\n"));
                for (k, c) in cols.iter().enumerate() {
                    if k > 0 {
                        out.push_str("::: column\n\n");
                    }
                    let inner = blocks_to_md(c);
                    if !inner.trim().is_empty() {
                        out.push_str(inner.trim_end());
                        out.push_str("\n\n");
                    }
                }
                out.push_str(":::");
            }
            // An empty line the user made: Markdown has none, so a paragraph of one
            // non-breaking space (to_html turns it back into an empty paragraph).
            Block::Empty => out.push_str(EMPTY_LINE),
        }
        prev_list = list;
    }
    out.push('\n');
    out
}

/// A paragraph that starts like a block marker must not become one.
fn guard_start(t: &str) -> String {
    // "1. x" is escaped at the dot ("1\. x"): a backslash before a digit is not an escape
    if let Some((n, rest)) = t.split_once(". ") {
        if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) {
            return format!("{n}\\. {rest}");
        }
    }
    let starts_marker = t.starts_with('#') || t.starts_with("- ") || t.starts_with("+ ") || t.starts_with("---");
    if starts_marker {
        format!("\\{t}")
    } else {
        t.to_string()
    }
}

/// How an empty line from the editor is written in Markdown.
pub const EMPTY_LINE: &str = "&nbsp;";
/// An empty paragraph as Qt writes it (and keeps it).
const QT_EMPTY_P: &str = "<p style=\"-qt-paragraph-type:empty;\"><br /></p>";

/// Qt's HTML -> Markdown.
pub fn to_md(html: &str) -> String {
    blocks_to_md(&blocks_from_html(html))
}

/// Code fences (` ``` ` lines) of a Markdown text: (index of the opening line, language,
/// body).
fn fences(md: &str) -> Vec<(usize, String, String)> {
    let lines: Vec<&str> = md.lines().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim_start();
        if let Some(lang) = t.strip_prefix("```") {
            let start = i;
            let mut body = vec![];
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                body.push(lines[i]);
                i += 1;
            }
            out.push((start, lang.trim().to_string(), body.join("\n")));
        }
        i += 1;
    }
    out
}

/// Give the code blocks of Markdown that came back from the visual editor (which keeps
/// no languages) the languages they had in `previous`: a block with the same text gets
/// its language; otherwise, while the number of code blocks is unchanged, the one in the
/// same place does.
pub fn restore_code_langs(md: &str, previous: &str) -> String {
    let old = fences(previous);
    let new = fences(md);
    if old.is_empty() || new.is_empty() {
        return md.to_string();
    }
    let mut lines: Vec<String> = md.lines().map(str::to_string).collect();
    for (k, (at, lang, body)) in new.iter().enumerate() {
        if !lang.is_empty() {
            continue;
        }
        let by_text = old.iter().find(|(_, l, b)| !l.is_empty() && b.trim() == body.trim()).map(|(_, l, _)| l.clone());
        let by_place = (old.len() == new.len()).then(|| old[k].1.clone()).filter(|l| !l.is_empty());
        if let Some(l) = by_text.or(by_place) {
            let indent: String = lines[*at].chars().take_while(|c| c.is_whitespace()).collect();
            lines[*at] = format!("{indent}```{l}");
        }
    }
    let mut out = lines.join("\n");
    if md.ends_with('\n') {
        out.push('\n');
    }
    out
}

// ── Markdown -> HTML for Qt ──────────────────────────────────────────────────

fn esc_html(t: &str) -> String {
    t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Markdown -> HTML that Qt's importer turns into the same blocks `to_md` reads.
/// Columns become a borderless one-row table, `fvimg:` images `<img>` (when an image
/// bridge is installed, see `with_images`).
pub fn to_html(md: &str) -> String {
    let mut out = String::from("<html><body>");
    for seg in segments(md) {
        match seg {
            Segment::Md(m) => out.push_str(&md_html(&m, 560)),
            Segment::Columns { layout, cols } => {
                let n = cols.len().max(1) as f64;
                let w: Vec<f64> = (0..cols.len())
                    .map(|k| match layout.as_str() {
                        "align-right" if cols.len() >= 2 => if k == 0 { 2.0 } else { 1.0 },
                        "align-left" if cols.len() >= 2 => if k + 1 == cols.len() { 2.0 } else { 1.0 },
                        _ => 1.0,
                    })
                    .collect();
                let total: f64 = w.iter().sum::<f64>().max(1.0);
                out.push_str("<table border=\"0\" width=\"100%\" cellpadding=\"8\"><tr>");
                for (k, c) in cols.iter().enumerate() {
                    let pct = (w[k] / total * 100.0).round();
                    out.push_str(&format!("<td width=\"{pct}%\" valign=\"top\">"));
                    out.push_str(&md_html(c, (520.0 * w[k] / total * n / n).max(160.0) as u32));
                    out.push_str("</td>");
                }
                out.push_str("</tr></table>");
            }
        }
    }
    out.push_str("</body></html>");
    // a paragraph of only a non-breaking space (`&nbsp;`, an empty line) is an empty
    // paragraph in the editor: no hidden character to type around
    out.replace("<p>\u{a0}</p>", QT_EMPTY_P)
}

/// One run of plain Markdown as HTML (no wrapper). `img_width`: how wide images show.
fn md_html(md: &str, img_width: u32) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_TASKLISTS);
    let mut out = String::new();
    let mut in_code = false;
    let mut in_link = false;
    let mut code = String::new();
    let mut item_open: Vec<bool> = vec![];
    // Task markers arrive after the <li> start; remember where it was written.
    let mut li_at: Option<usize> = None;
    let mut table_head = false;
    let mut image_alt: Option<(String, String)> = None; // (alt so far, destination)
    for ev in crate::md::merged_events(md, opts) {
        if let Some((alt, dest)) = image_alt.as_mut() {
            match ev {
                Event::End(TagEnd::Image) => {
                    let (a, d) = (std::mem::take(alt), std::mem::take(dest));
                    image_alt = None;
                    match ImageRef::from_url(&d).and_then(|r| image_src(&r)) {
                        Some(src) => out.push_str(&format!("<img src=\"{}\" width=\"{img_width}\" alt=\"{}\" />", esc_html(&src), esc_html(&a))),
                        None => out.push_str(&esc_html(&format!("[image{}]", if a.is_empty() { String::new() } else { format!(": {a}") }))),
                    }
                }
                Event::Text(t) | Event::Code(t) => alt.push_str(&t),
                _ => {}
            }
            continue;
        }
        match ev {
            Event::Start(Tag::Heading { level, .. }) => {
                let l = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    _ => 3,
                };
                out.push_str(&format!("<h{l}>"));
            }
            Event::End(TagEnd::Heading(level)) => {
                let l = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    _ => 3,
                };
                out.push_str(&format!("</h{l}>"));
            }
            Event::Start(Tag::Paragraph) => {
                if item_open.last() != Some(&true) {
                    out.push_str("<p>")
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if item_open.last() != Some(&true) {
                    out.push_str("</p>")
                } else {
                    out.push(' ')
                }
            }
            Event::Start(Tag::BlockQuote(_)) => out.push_str("<blockquote>"),
            Event::End(TagEnd::BlockQuote(_)) => out.push_str("</blockquote>"),
            Event::Start(Tag::CodeBlock(kind)) => {
                let _ = matches!(kind, CodeBlockKind::Fenced(_));
                in_code = true;
                code.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code = false;
                out.push_str(&format!("<pre>{}</pre>", esc_html(code.trim_end_matches('\n'))));
            }
            Event::Start(Tag::List(first)) => {
                out.push_str(if first.is_some() { "<ol>" } else { "<ul>" });
            }
            Event::End(TagEnd::List(ordered)) => out.push_str(if ordered { "</ol>" } else { "</ul>" }),
            Event::Start(Tag::Item) => {
                li_at = Some(out.len());
                out.push_str("<li>");
                item_open.push(true);
            }
            Event::End(TagEnd::Item) => {
                out.push_str("</li>");
                item_open.pop();
            }
            Event::TaskListMarker(done) => {
                if let Some(at) = li_at.take() {
                    out.replace_range(at..at + 4, if done { "<li class=\"checked\">" } else { "<li class=\"unchecked\">" });
                }
            }
            Event::Start(Tag::Table(_)) => out.push_str("<table border=\"1\" cellpadding=\"4\">"),
            Event::End(TagEnd::Table) => out.push_str("</table>"),
            Event::Start(Tag::TableHead) => {
                table_head = true;
                out.push_str("<tr>")
            }
            Event::End(TagEnd::TableHead) => {
                table_head = false;
                out.push_str("</tr>")
            }
            Event::Start(Tag::TableRow) => out.push_str("<tr>"),
            Event::End(TagEnd::TableRow) => out.push_str("</tr>"),
            Event::Start(Tag::TableCell) => out.push_str(if table_head { "<th>" } else { "<td>" }),
            Event::End(TagEnd::TableCell) => out.push_str(if table_head { "</th>" } else { "</td>" }),
            Event::Start(Tag::Strong) => out.push_str("<b>"),
            Event::End(TagEnd::Strong) => out.push_str("</b>"),
            Event::Start(Tag::Emphasis) => out.push_str("<i>"),
            Event::End(TagEnd::Emphasis) => out.push_str("</i>"),
            Event::Start(Tag::Strikethrough) => out.push_str("<s>"),
            Event::End(TagEnd::Strikethrough) => out.push_str("</s>"),
            Event::Start(Tag::Link { dest_url, .. }) => {
                in_link = true;
                out.push_str(&format!("<a href=\"{}\">", esc_html(&dest_url)))
            }
            Event::End(TagEnd::Link) => {
                in_link = false;
                out.push_str("</a>")
            }
            Event::Start(Tag::Image { dest_url, .. }) => image_alt = Some((String::new(), dest_url.to_string())),
            Event::Code(t) => out.push_str(&format!("<code>{}</code>", esc_html(&t))),
            Event::Text(t) => {
                if in_code {
                    code.push_str(&t)
                } else if in_link {
                    out.push_str(&esc_html(&t))
                } else {
                    // bare web addresses show as links, as they will be in the document
                    let mut at = 0;
                    for (a, e) in crate::md::bare_urls(&t) {
                        out.push_str(&esc_html(&t[at..a]));
                        out.push_str(&format!("<a href=\"{0}\">{0}</a>", esc_html(&t[a..e])));
                        at = e;
                    }
                    out.push_str(&esc_html(&t[at..]));
                }
            }
            Event::SoftBreak => out.push(' '),
            Event::HardBreak => out.push_str("<br />"),
            Event::Rule => out.push_str("<hr />"),
            Event::Html(h) | Event::InlineHtml(h) => out.push_str(&esc_html(&h)),
            _ => {}
        }
    }
    out
}

// ── block styles ─────────────────────────────────────────────────────────────

/// Qt's HTML importer drops empty headings, list items and quotes, so a block
/// restyled while empty (typing "# " on a new line) holds a zero-width space
/// until text is typed; `to_md` removes it.
const ZWSP: &str = "\u{200B}";

/// Plain text of inline Markdown (for code blocks, which hold no marks).
fn plain_of(inline: &str) -> String {
    let mut s = String::new();
    for ev in Parser::new_ext(inline, Options::ENABLE_STRIKETHROUGH) {
        match ev {
            Event::Text(t) | Event::Code(t) => s.push_str(&t),
            Event::SoftBreak | Event::HardBreak => s.push(' '),
            _ => {}
        }
    }
    s
}

/// The block kind at Qt text block `index` (as `kind_at` in the view names them).
fn kind_of(b: &Block) -> &'static str {
    match b {
        Block::Heading(1, _) => "h1",
        Block::Heading(2, _) => "h2",
        Block::Heading(_, _) => "h3",
        Block::Quote(_) => "quote",
        Block::Item { task: Some(true), .. } => "done",
        Block::Item { task: Some(false), .. } => "task",
        Block::Item { ordered: true, .. } => "ol",
        Block::Item { .. } => "ul",
        Block::Code(_) => "code",
        Block::Table(_) | Block::Columns { .. } => "table",
        Block::Hr => "hr",
        _ => "p",
    }
}

/// Where the view's text piece `index` is.
#[derive(Debug, PartialEq)]
enum Loc {
    /// Top-level block `i` (line `line` of a code block).
    Top { i: usize, line: usize },
    /// Block `j` (line `line`) of column `col` of the columns block `i`.
    Cell { i: usize, col: usize, j: usize, line: usize },
    /// A table cell, or the empty piece Qt keeps around a table.
    Fixed,
}

/// Where the cursor is, as the view reports it: the number of its text piece, and for
/// each top-level table (in order) whether the piece right after it is empty. Qt keeps
/// an empty text block after a table that it does not write out as HTML, unless text
/// was typed into it (then that block is the next paragraph). Written "7" or "7:10".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct At {
    pub index: usize,
    pub gaps: Vec<bool>,
}

impl At {
    pub fn parse(s: &str) -> At {
        let (n, flags) = s.trim().split_once(':').unwrap_or((s.trim(), ""));
        At { index: n.parse().unwrap_or(usize::MAX), gaps: flags.chars().map(|c| c != '0').collect() }
    }
}

impl From<usize> for At {
    fn from(index: usize) -> At {
        At { index, gaps: vec![] }
    }
}

fn locate(blocks: &[Block], at: &At) -> Option<Loc> {
    let index = at.index;
    // A document that starts with a table has an empty text block before it in Qt.
    let mut n = usize::from(matches!(blocks.first(), Some(Block::Table(_) | Block::Columns { .. })));
    if index < n {
        return Some(Loc::Fixed);
    }
    let mut tables = 0;
    for (i, b) in blocks.iter().enumerate() {
        let k = b.units();
        let gap = if matches!(b, Block::Table(_) | Block::Columns { .. }) {
            tables += 1;
            usize::from(at.gaps.get(tables - 1).copied().unwrap_or(true))
        } else {
            0
        };
        if index >= n + k + gap {
            n += k + gap;
            continue;
        }
        if index >= n + k {
            return Some(Loc::Fixed); // the empty piece after a table
        }
        let off = index - n;
        return Some(match b {
            Block::Columns { cols, .. } => {
                let mut m = 0;
                for (col, c) in cols.iter().enumerate() {
                    let ck = c.iter().map(Block::units).sum::<usize>().max(1);
                    if off < m + ck {
                        let mut r = off - m;
                        for (j, inner) in c.iter().enumerate() {
                            let u = inner.units();
                            if r < u {
                                return Some(match inner {
                                    Block::Table(_) | Block::Columns { .. } => Loc::Fixed,
                                    _ => Loc::Cell { i, col, j, line: r },
                                });
                            }
                            r -= u;
                        }
                        return Some(Loc::Fixed); // an empty cell
                    }
                    m += ck;
                }
                Loc::Fixed
            }
            Block::Table(_) => Loc::Fixed,
            _ => Loc::Top { i, line: off },
        });
    }
    None
}

/// The block kind at the view's text piece `index` ("p", "h1".., "ul", "ol", "task",
/// "done", "quote", "code", "hr", "table" in a table cell, or "" when out of range).
pub fn kind_at(html: &str, at: impl Into<At>) -> String {
    let blocks = blocks_from_html(html);
    match locate(&blocks, &at.into()) {
        Some(Loc::Top { i, .. }) => kind_of(&blocks[i]).to_string(),
        Some(Loc::Cell { i, col, j, .. }) => match &blocks[i] {
            Block::Columns { cols, .. } => kind_of(&cols[col][j]).to_string(),
            _ => String::new(),
        },
        Some(Loc::Fixed) => "table".into(),
        None => String::new(),
    }
}

/// Change the kind of the text block at `index` and return the document as HTML
/// for Qt. `kind` is "p", "h1", "h2", "h3", "ul", "ol", "task", "done", "quote" or "code",
/// or "indent" / "outdent" for a list item (Tab / Shift+Tab).
/// Blocks inside columns can be restyled; table cells cannot.
pub fn set_block(html: &str, at: impl Into<At>, kind: &str) -> Result<String, String> {
    let mut blocks = blocks_from_html(html);
    match locate(&blocks, &at.into()).ok_or("the cursor is not in a text block this editor can restyle")? {
        Loc::Top { i, line } => restyle(&mut blocks, i, line, kind)?,
        Loc::Cell { i, col, j, line } => {
            if kind == "code" {
                return Err("code blocks cannot go inside columns".into());
            }
            let Block::Columns { cols, .. } = &mut blocks[i] else { unreachable!() };
            restyle(&mut cols[col], j, line, kind)?;
        }
        Loc::Fixed => return Err("tables keep their layout".into()),
    }
    // Keep empty text blocks as empty paragraphs so the view's block count still lines up.
    Ok(to_html_blocks(&blocks))
}

/// Restyle block `i` (line `line` of a code block) of one run of blocks.
fn restyle(blocks: &mut Vec<Block>, i: usize, line: usize, kind: &str) -> Result<(), String> {
    let inline_of = |b: &Block| -> String {
        match b {
            Block::Heading(_, t) | Block::Para(t) | Block::Quote(t) => t.clone(),
            Block::Item { inline, .. } => inline.clone(),
            Block::Code(lines) => escape_md(lines.get(line).map(String::as_str).unwrap_or("")),
            _ => String::new(),
        }
    };
    let old = blocks[i].clone();
    // Tab / Shift+Tab in a list: one level deeper (at most one below the item above) or
    // shallower (out of the list from the top level)
    if kind == "indent" || kind == "outdent" {
        let Block::Item { list, ordered, depth, task, inline } = old else {
            return Err("only list items can be indented".into());
        };
        let inline = if inline.trim().is_empty() { ZWSP.to_string() } else { inline };
        let above = i.checked_sub(1).and_then(|p| match blocks.get(p) {
            Some(Block::Item { depth, .. }) => Some(*depth),
            _ => None,
        });
        blocks[i] = if kind == "indent" {
            let deepest = above.map(|d| d + 1).unwrap_or(0);
            Block::Item { list, ordered, depth: (depth + 1).min(deepest).max(depth), task, inline }
        } else if depth > 0 {
            Block::Item { list, ordered, depth: depth - 1, task, inline }
        } else {
            Block::Para(inline)
        };
        return Ok(());
    }
    let (depth, list) = match &old {
        Block::Item { depth, list, .. } => (*depth, *list),
        // a new list unless it joins the one just above
        _ => (
            0,
            match i.checked_sub(1).and_then(|p| blocks.get(p)) {
                Some(Block::Item { list, .. }) => *list,
                _ => usize::MAX - i,
            },
        ),
    };
    let mut text = inline_of(&old);
    // (a plain paragraph too: Enter after a heading restyles the new, empty line)
    if text.trim().is_empty() && kind != "code" {
        text = ZWSP.to_string();
    }
    let new = match kind {
        "h1" => Block::Heading(1, text),
        "h2" => Block::Heading(2, text),
        "h3" => Block::Heading(3, text),
        "quote" => Block::Quote(text),
        "ul" => Block::Item { list, ordered: false, depth, task: None, inline: text },
        "ol" => Block::Item { list, ordered: true, depth, task: None, inline: text },
        "task" => Block::Item { list, ordered: false, depth, task: Some(false), inline: text },
        "done" => Block::Item { list, ordered: false, depth, task: Some(true), inline: text },
        "code" => Block::Code(vec![plain_of(&text)]),
        "p" => Block::Para(text),
        other => return Err(format!("unknown block style {other}")),
    };
    match &old {
        // one line out of a code block: split the block around it
        Block::Code(lines) if lines.len() > 1 && kind != "code" => {
            let before: Vec<String> = lines[..line].to_vec();
            let after: Vec<String> = lines[line + 1..].to_vec();
            let mut repl = vec![];
            if !before.is_empty() {
                repl.push(Block::Code(before));
            }
            repl.push(new);
            if !after.is_empty() {
                repl.push(Block::Code(after));
            }
            blocks.splice(i..=i, repl);
        }
        _ => blocks[i] = new,
    }
    Ok(())
}

fn to_html_blocks(blocks: &[Block]) -> String {
    // Empty blocks are dropped by Markdown; write them as empty paragraphs instead.
    let mut html = String::from("<html><body>");
    let mut run: Vec<Block> = vec![];
    let flush = |run: &mut Vec<Block>, html: &mut String| {
        if !run.is_empty() {
            let inner = to_html(&blocks_to_md(run));
            html.push_str(inner.trim_start_matches("<html><body>").trim_end_matches("</body></html>"));
            run.clear();
        }
    };
    for b in blocks {
        if matches!(b, Block::Empty) {
            flush(&mut run, &mut html);
            html.push_str(QT_EMPTY_P);
        } else {
            run.push(b.clone());
        }
    }
    flush(&mut run, &mut html);
    html.push_str("</body></html>");
    html
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What Qt 6.9 writes for a document (captured from TextArea.text in RichText mode).
    const QT_HTML: &str = r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.0//EN" "http://www.w3.org/TR/REC-html40/strict.dtd">
<html><head><meta name="qrichtext" content="1" /><meta charset="utf-8" /><style type="text/css">
p, li { white-space: pre-wrap; }
hr { height: 1px; border-width: 0; }
li.unchecked::marker { content: "\2610"; }
li.checked::marker { content: "\2612"; }
</style></head><body style=" font-family:'Sans Serif'; font-size:9pt; font-weight:400; font-style:normal;">
<h1 style=" margin-top:18px; margin-bottom:12px; margin-left:0px; margin-right:0px; -qt-block-indent:0; text-indent:0px;"><span style=" font-size:xx-large; font-weight:700;">Title</span></h1>
<p style=" margin-top:12px; margin-bottom:12px; margin-left:0px; margin-right:0px; -qt-block-indent:0; text-indent:0px;">Some <span style=" font-weight:700;">bold</span>, <span style=" font-style:italic;">it</span>, <span style=" text-decoration: line-through;">gone</span>, <span style=" font-family:'monospace';">code</span> and <a href="https://x.io"><span style=" text-decoration: underline; color:#0000ff;">link</span></a>.</p>
<ul style="margin-top: 0px; margin-bottom: 0px; margin-left: 0px; margin-right: 0px; -qt-list-indent: 1;">
<li style=" margin-top:12px; margin-bottom:0px; margin-left:0px; margin-right:0px; -qt-block-indent:0; text-indent:0px;">one</li>
<li style=" margin-top:0px; margin-bottom:0px; margin-left:0px; margin-right:0px; -qt-block-indent:0; text-indent:0px;">two
<ul style="margin-top: 0px; margin-bottom: 0px; margin-left: 0px; margin-right: 0px; -qt-list-indent: 2;">
<li style=" margin-top:0px; margin-bottom:12px; margin-left:0px; margin-right:0px; -qt-block-indent:0; text-indent:0px;">nested</li></ul></li></ul>
<ol style="margin-top: 0px; margin-bottom: 0px; margin-left: 0px; margin-right: 0px; -qt-list-indent: 1;">
<li style=" margin-top:12px; margin-bottom:12px; margin-left:0px; margin-right:0px; -qt-block-indent:0; text-indent:0px;">first</li></ol>
<ul style="margin-top: 0px; margin-bottom: 0px; margin-left: 0px; margin-right: 0px; -qt-list-indent: 1;">
<li class="unchecked" style=" margin-top:12px; margin-bottom:0px; margin-left:0px; margin-right:0px; -qt-block-indent:0; text-indent:0px;">task</li>
<li class="checked" style=" margin-top:0px; margin-bottom:12px; margin-left:0px; margin-right:0px; -qt-block-indent:0; text-indent:0px;">done</li></ul>
<p style=" margin-top:12px; margin-bottom:12px; margin-left:40px; margin-right:40px; -qt-block-indent:0; text-indent:0px;">quote</p>
<pre style=" margin-top:12px; margin-bottom:0px; margin-left:0px; margin-right:0px; -qt-block-indent:0; text-indent:0px;"><span style=" font-family:'monospace';">line1</span></pre>
<pre style=" margin-top:0px; margin-bottom:12px; margin-left:0px; margin-right:0px; -qt-block-indent:0; text-indent:0px;"><span style=" font-family:'monospace';">line2</span></pre>
<table border="0" style=" margin-top:2px; margin-bottom:2px; margin-left:0px; margin-right:0px;" cellspacing="2" cellpadding="0">
<tr>
<td>
<p align="center" style=" margin-top:0px; margin-bottom:0px; margin-left:0px; margin-right:0px; -qt-block-indent:0; text-indent:0px;"><span style=" font-weight:700;">a</span></p></td>
<td>
<p align="center" style=" margin-top:0px; margin-bottom:0px; margin-left:0px; margin-right:0px; -qt-block-indent:0; text-indent:0px;"><span style=" font-weight:700;">b</span></p></td></tr>
<tr>
<td>
<p style=" margin-top:0px; margin-bottom:0px; margin-left:0px; margin-right:0px; -qt-block-indent:0; text-indent:0px;">1 &lt; 2 &amp; x*y</p></td>
<td>
<p style=" margin-top:0px; margin-bottom:0px; margin-left:0px; margin-right:0px; -qt-block-indent:0; text-indent:0px;">2</p></td></tr></table>
<p style="-qt-paragraph-type:empty; margin-top:0px; margin-bottom:0px; margin-left:0px; margin-right:0px; -qt-block-indent:0; text-indent:0px;"><br /></p>
<p style=" margin-top:12px; margin-bottom:12px; margin-left:0px; margin-right:0px; -qt-block-indent:0; text-indent:0px;">end</p></body></html>"#;

    #[test]
    fn qt_html_to_markdown() {
        let md = to_md(QT_HTML);
        assert_eq!(
            md,
            "# Title\n\nSome **bold**, *it*, ~~gone~~, `code` and [link](https://x.io).\n\n- one\n- two\n    - nested\n\n1. first\n\n- [ ] task\n- [x] done\n\n> quote\n\n```\nline1\nline2\n```\n\n| a | b |\n| --- | --- |\n| 1 \\< 2 & x\\*y | 2 |\n\n&nbsp;\n\nend\n"
        );
    }

    #[test]
    fn markdown_survives_the_round_trip() {
        // to_html -> (what Qt would write is modelled by to_md reading our own HTML) -> to_md
        let md = "# Title\n\nSome **bold** and [a link](https://logos.co).\n\n- one\n- [ ] task\n- [x] done\n\n1. first\n2. second\n\n> quoted\n\n```\nlet x = 1;\n```\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n";
        let back = to_md(&to_html(md));
        let render = |m: &str| serde_json::to_string(&crate::doc::tabs(&crate::doc::load(&crate::md::to_yjs(m)).unwrap())).unwrap();
        assert_eq!(render(&back), render(md), "{back}");
    }

    #[test]
    fn block_styles_change_one_block() {
        // index counts the view's text pieces (Qt's plain text, probed):
        // Title=0 Some=1 one=2 two=3 nested=4 first=5 task=6 done=7 quote=8 line1=9 line2=10,
        // the table's cells 11-14 and the piece after it 15, the empty paragraph 16, end=17
        assert_eq!(kind_at(QT_HTML, 0), "h1");
        assert_eq!(kind_at(QT_HTML, 4), "ul");
        assert_eq!(kind_at(QT_HTML, 6), "task");
        assert_eq!(kind_at(QT_HTML, 8), "quote");
        assert_eq!(kind_at(QT_HTML, 10), "code");
        assert_eq!(kind_at(QT_HTML, 12), "table");
        assert_eq!(kind_at(QT_HTML, 15), "table");
        assert_eq!(kind_at(QT_HTML, 17), "p");
        let md = |h: String| to_md(&h);
        assert!(md(set_block(QT_HTML, 1, "h2").unwrap()).contains("\n## Some bold, *it*"));
        assert!(md(set_block(QT_HTML, 0, "p").unwrap()).starts_with("Title\n\nSome"));
        assert!(md(set_block(QT_HTML, 6, "done").unwrap()).contains("- [x] task\n- [x] done"));
        assert!(md(set_block(QT_HTML, 17, "task").unwrap()).ends_with("- [ ] end\n"));
        // a line out of a code block splits it
        let split = md(set_block(QT_HTML, 9, "p").unwrap());
        assert!(split.contains("> quote\n\nline1\n\n```\nline2\n```"), "{split}");
        // nested item keeps its depth
        assert!(md(set_block(QT_HTML, 4, "task").unwrap()).contains("    - [ ] nested"));
        assert!(set_block(QT_HTML, 99, "p").is_err());
        assert!(set_block(QT_HTML, 13, "h1").is_err(), "table cells keep their layout");
    }

    fn bridge() -> Images {
        Images {
            src_of: Box::new(|r: &ImageRef| Some(format!("file:///cache/{}.png", if r.local.is_empty() { r.cid.clone() } else { r.local.replace('/', "_") }))),
            url_of: Box::new(|src: &str| {
                let name = src.rsplit('/').next()?.trim_end_matches(".png");
                Some(if name.starts_with("Qm") {
                    ImageRef { cid: name.into(), key: "k".into(), nonce: "n".into(), tag: "t".into(), ..Default::default() }.to_url()
                } else {
                    ImageRef { local: name.replacen('_', "/", 1), ..Default::default() }.to_url()
                })
            }),
        }
    }

    #[test]
    fn columns_and_images_cross_the_editor() {
        let img = ImageRef { cid: "QmPic".into(), key: "k".into(), nonce: "n".into(), tag: "t".into(), ..Default::default() };
        let md = format!("## Beside\n\n::: columns align-right\n\n- one\n- **two**\n\n::: column\n\n![a lambda]({})\n\n:::\n\nAfter\n", img.to_url());
        let (html, back) = with_images(bridge(), || {
            let h = to_html(&md);
            let b = to_md(&h);
            (h, b)
        });
        assert!(html.contains("<table border=\"0\" width=\"100%\"") && html.contains("width=\"67%\"") && html.contains("src=\"file:///cache/QmPic.png\""), "{html}");
        assert!(back.contains("::: columns align-right") && back.contains("- **two**") && back.contains("::: column"), "{back}");
        let refs: Vec<ImageRef> = back.split("](").skip(1).filter_map(|r| ImageRef::from_url(r.split(')').next().unwrap())).collect();
        assert_eq!(refs.len(), 1, "{back}");
        assert_eq!(refs[0].cid, "QmPic");
        assert!(back.trim_end().ends_with("After"), "{back}");
    }

    /// What Qt 6.9 writes back for the editor's columns and an image (captured with a
    /// TextArea in RichText mode; styles trimmed).
    #[test]
    fn qt_columns_and_images() {
        let qt = r#"<html><body>
<p>Before</p>
<table border="0" width="100%" cellspacing="2" cellpadding="6">
<tr>
<td width="66%">
<p>Left <span style=" font-weight:700;">text</span></p>
<ul>
<li>one</li></ul></td>
<td width="34%">
<p><img src="file:///cache/QmPic.png" width="300" /></p></td></tr></table>
<table border="1" cellspacing="2" cellpadding="4">
<tr>
<td>
<p align="center"><span style=" font-weight:700;">a</span></p></td></tr>
<tr>
<td>
<p>1</p></td></tr></table>
<p>After <img src="file:///cache/d1_x.png" width="120" /></p></body></html>"#;
        let md = with_images(bridge(), || to_md(qt));
        assert!(md.starts_with("Before\n\n::: columns align-right\n\nLeft **text**\n\n- one\n\n::: column\n\n![](fvimg:"), "{md}");
        assert!(md.contains("| a |\n| --- |\n| 1 |"), "a real table stays a table: {md}");
        assert!(md.contains("After ![](fvimg:"), "{md}");
        // without a bridge (no image support) images drop out, text stays
        assert!(!to_md(qt).contains("fvimg"));
    }

    /// Text pieces as the view counts them: Qt's plain text of
    /// tests/fixtures/qt_columns.html (captured from a TextArea in RichText mode) is
    /// `Test pres⏎A little image test▸This i cool:)⏎Yay!▸[image: pic]◂⏎Test▸Hello world how are you ?▸Right text◂⏎fdfd`
    /// (⏎ U+2029, ▸ U+FDD0, ◂ U+FDD1), and the one that starts with columns `▸L▸R◂`.
    #[test]
    fn a_tab_indent_survives() {
        let md = to_md("<html><body><p>\u{a0}\u{a0}\u{a0}\u{a0}indented</p></body></html>");
        assert_eq!(md.trim_end(), "\u{a0}\u{a0}\u{a0}\u{a0}indented");
        assert!(to_md(&to_html(&md)).starts_with("\u{a0}\u{a0}\u{a0}\u{a0}indented"));
    }

    #[test]
    fn shift_enter_is_a_line_break() {
        let qt = "<html><body><p>line one<br />line two</p></body></html>";
        let md = to_md(qt);
        assert_eq!(md.trim(), "line one\\\nline two");
        // and back: the editor gets the break again, the preview too
        assert!(to_html(&md).contains("<br />"), "{}", to_html(&md));
        // bold across the break stays bold on both lines
        let md = to_md("<html><body><p><span style=\" font-weight:700;\">a<br />b</span></p></body></html>");
        assert_eq!(md.trim(), "**a**\\\n**b**");
        // a break with no text on one side is nothing (Qt's empty lines are <br />)
        assert_eq!(to_md("<html><body><p><br />a</p></body></html>").trim(), "a");
        assert_eq!(to_md("<html><body><p>a<br /></p></body></html>").trim(), "a");
    }

    #[test]
    fn code_keeps_its_indentation_in_the_editor() {
        // as Qt writes a code block: one <pre> per line, spaces kept
        let p = |t: &str| format!("<pre style=\" margin-top:0px; margin-bottom:0px;\"><span style=\" font-family:'monospace';\">{t}</span></pre>");
        let qt = format!("<html><body>{}{}{}{}</body></html>", p("fn a() {"), p("    b();"), p("        c();"), p("}"));
        assert_eq!(to_md(&qt).trim(), "```\nfn a() {\n    b();\n        c();\n}\n```");
        let qt2 = format!("<html><body>{}{}</body></html>", p("x"), p("&nbsp;&nbsp;&nbsp;&nbsp;y"));
        assert_eq!(to_md(&qt2).trim(), "```\nx\n    y\n```");
    }

    #[test]
    fn an_empty_line_in_a_quote_is_the_quotes() {
        let qt = "<html><body><p style=\" margin-top:12px; margin-bottom:12px; margin-left:40px; margin-right:40px;\">q</p>\
                  <p style=\"-qt-paragraph-type:empty; margin-top:12px; margin-bottom:12px; margin-left:40px; margin-right:40px;\"><br /></p></body></html>";
        assert_eq!(kind_at(qt, 1usize), "quote");
        // Enter there again: the line becomes plain text and the quote ends
        let md = to_md(&set_block(qt, 1usize, "p").unwrap());
        assert_eq!(md.trim_end(), "> q\n\n&nbsp;");
    }

    #[test]
    fn tab_nests_list_items_and_shift_tab_lifts_them() {
        let html = to_html("- one\n- two\n- three\n");
        let once = set_block(&html, 1usize, "indent").unwrap();
        assert_eq!(to_md(&once).trim(), "- one\n    - two\n- three");
        // at most one level below the item above
        let twice = set_block(&once, 1usize, "indent").unwrap();
        assert_eq!(to_md(&twice).trim(), "- one\n    - two\n- three");
        let back = set_block(&once, 1usize, "outdent").unwrap();
        assert_eq!(to_md(&back).trim(), "- one\n- two\n- three");
        // the first item cannot go deeper; out of the list from the top level
        assert_eq!(to_md(&set_block(&html, 0usize, "indent").unwrap()).trim(), "- one\n- two\n- three");
        let out = to_md(&set_block(&html, 2usize, "outdent").unwrap());
        assert_eq!(out.trim(), "- one\n- two\n\nthree");
        assert!(set_block(&to_html("text\n"), 0usize, "indent").is_err());
    }

    #[test]
    fn blocks_inside_columns_are_found_and_restyled() {
        let qt = include_str!("../tests/fixtures/qt_columns.html");
        let kinds: Vec<String> = (0..12).map(|k| kind_at(qt, k)).collect();
        assert_eq!(kinds, ["h1", "h1", "ul", "ul", "p", "table", "h2", "p", "p", "table", "p", ""], "{kinds:?}");

        // a paragraph in a column becomes a list item, and the columns stay
        let md = to_md(&set_block(qt, 7, "ul").unwrap());
        assert!(md.contains("## Test\n\n::: columns align-center\n\n- Hello world how are you ?\n\n::: column\n\nRight text\n\n:::"), "{md}");
        // a list item in a column becomes a heading; its neighbour stays a list item
        let md = to_md(&set_block(qt, 3, "h2").unwrap());
        assert!(md.contains("- This i cool:)\n\n## Yay!\n\n::: column"), "{md}");
        // the text between two columns blocks
        let md = to_md(&set_block(qt, 6, "h1").unwrap());
        assert!(md.contains(":::\n\n# Test\n\n::: columns"), "{md}");
        // the piece after a table cannot be restyled; neither can code go into a column
        assert!(set_block(qt, 5, "h1").is_err());
        assert!(set_block(qt, 7, "code").is_err());
        // after the restyle the view's count still lines up (the same pieces, one changed)
        let again = set_block(qt, 7, "ul").unwrap();
        assert_eq!(kind_at(&again, 7), "ul");
        assert_eq!(kind_at(&again, 10), "p");

        let first = include_str!("../tests/fixtures/qt_columns_first.html");
        let kinds: Vec<String> = (0..5).map(|k| kind_at(first, k)).collect();
        assert_eq!(kinds, ["table", "p", "p", "table", ""], "{kinds:?}");
        let md = to_md(&set_block(first, 2, "h3").unwrap());
        assert!(md.contains("L\n\n::: column\n\n### R"), "{md}");
    }

    /// Text typed straight after a table goes into the empty block Qt keeps there, and
    /// Qt writes it as the next paragraph: the view says so with the table's flag.
    /// (Qt's export below, styles trimmed; its plain text is `AB▸CD▸GH◂Z⏎IJ`.)
    #[test]
    fn text_typed_after_columns_is_the_next_block() {
        let qt = r#"<html><body><p>AB</p><table border="0" width="100%" cellspacing="2" cellpadding="0"><tr><td><p>CD</p></td><td><p>GH</p></td></tr></table><p style=" margin-top:0px; margin-bottom:0px;">Z</p><p>IJ</p></body></html>"#;
        let kinds: Vec<String> = (0..6).map(|k| kind_at(qt, At::parse(&format!("{k}:0")))).collect();
        assert_eq!(kinds, ["p", "p", "p", "p", "p", ""], "{kinds:?}");
        let md = to_md(&set_block(qt, At::parse("3:0"), "h2").unwrap());
        assert!(md.contains(":::\n\n## Z\n\nIJ"), "{md}");
        // without the flag the empty piece is assumed: piece 3 is that piece
        assert_eq!(kind_at(qt, 3), "table");
        assert_eq!(At::parse("12:101"), At { index: 12, gaps: vec![true, false, true] });
        assert_eq!(At::parse("x").index, usize::MAX);
    }

    /// A pasted address shows as a link in the editor and is written back bare.
    #[test]
    fn bare_addresses_are_links_in_the_editor() {
        let html = to_html("Deck: https://logos.co/deck and [named](https://x.io)\n");
        assert!(html.contains(r#"<a href="https://logos.co/deck">https://logos.co/deck</a>"#), "{html}");
        assert_eq!(to_md(&html), "Deck: https://logos.co/deck and [named](https://x.io)\n");
        // what Qt writes back for it (link tinted by the view)
        let qt = r#"<html><body><p>Deck: <a href="https://logos.co/deck"><span style=" color:#ff8800;">https://logos.co/deck</span></a></p></body></html>"#;
        assert_eq!(to_md(qt), "Deck: https://logos.co/deck\n");
        // an address Markdown would misread keeps the explicit form
        assert_eq!(to_md(r#"<p><a href="https://x.io/a_b_c">https://x.io/a_b_c</a></p>"#), "[https://x.io/a\\_b\\_c](https://x.io/a_b_c)\n");
    }

    /// Qt keeps no code language: diagrams are recognised, other languages come back
    /// from the draft as it was.
    #[test]
    fn code_languages_survive_the_visual_editor() {
        let md = "# T\n\n```mermaid\nflowchart LR\n  A --> B\n```\n\nand\n\n```rust\nfn main() {}\n```\n";
        // as Qt writes it back: one <pre> per code line, no language anywhere
        let qt = "<html><body><h1>T</h1><pre>flowchart LR</pre><pre>  A --&gt; B</pre><p>and</p><pre>fn main() {}</pre></body></html>";
        let back = to_md(qt);
        assert!(back.contains("```mermaid\nflowchart LR\n  A --> B\n```"), "{back}");
        assert!(back.contains("```\nfn main() {}"), "Qt lost it: {back}");
        assert_eq!(restore_code_langs(&back, md), md);
        // edited in the editor: same place, same number of blocks
        let edited = back.replace("fn main() {}", "fn main() { run() }");
        assert!(restore_code_langs(&edited, md).contains("```rust\nfn main() { run() }"));
        // a diagram typed in the visual editor (no language at all) is still a diagram
        let typed = to_md("<html><body><pre>sequenceDiagram</pre><pre>  A-&gt;&gt;B: hi</pre></body></html>");
        assert!(typed.starts_with("```mermaid\nsequenceDiagram"), "{typed}");
    }

    /// Empty lines made in the editor survive as `&nbsp;` paragraphs and come back empty
    /// (vpavlin: two empty lines were missing on the other side of a shared draft).
    #[test]
    fn empty_lines_survive_the_round_trip() {
        let qt = r#"<html><body><p>A</p><p style="-qt-paragraph-type:empty; margin-top:0px;"><br /></p><p style="-qt-paragraph-type:empty;"><br /></p><p>B</p></body></html>"#;
        let md = to_md(qt);
        assert_eq!(md, "A\n\n&nbsp;\n\n&nbsp;\n\nB\n");
        let html = to_html(&md);
        assert_eq!(html.matches(QT_EMPTY_P).count(), 2, "{html}");
        assert!(!html.contains('\u{a0}'), "no hidden character in the editor: {html}");
        assert_eq!(to_md(&html), md);
        // and Fileverse/preview: empty-looking paragraphs, not slide content
        let doc = crate::doc::load(&crate::md::to_yjs(&md)).unwrap();
        let tab = &crate::doc::tabs(&doc)[0];
        assert_eq!(tab.slides.len(), 1);
    }

    #[test]
    fn an_empty_block_keeps_its_new_style() {
        let html = "<html><body><p>first</p><p style=\"-qt-paragraph-type:empty;\"><br /></p></body></html>";
        for (kind, tag) in [("h1", "<h1>"), ("ul", "<li>"), ("quote", "<blockquote>"), ("task", "<li class=\"unchecked\">")] {
            let h = set_block(html, 1, kind).unwrap();
            assert!(h.contains(tag) && h.contains('\u{200B}'), "{kind}: {h}");
            // what Qt writes back with a few words typed after the placeholder
            assert!(!to_md(&h.replace('\u{200B}', "\u{200B}typed")).contains('\u{200B}'));
        }
        assert_eq!(to_md(&set_block(html, 1, "h1").unwrap().replace('\u{200B}', "\u{200B}Title")), "first\n\n# Title\n");
    }

    #[test]
    fn text_that_looks_like_markup_stays_text() {
        let html = "<html><body><p># not a heading</p><p>1. not a list</p><p>a *b* [c]</p></body></html>";
        let md = to_md(html);
        assert_eq!(md, "\\# not a heading\n\n1\\. not a list\n\na \\*b\\* \\[c\\]\n");
        let back = to_md(&to_html(&md));
        assert_eq!(back, md);
    }
}

