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
    /// An empty text block (Qt keeps them; Markdown drops them).
    Empty,
}

impl Block {
    /// How many of Qt's top-level text blocks this is: what the view counts with
    /// paragraph separators. A table's cells are separated differently, and a code
    /// block is one text block per line.
    fn qt_blocks(&self) -> usize {
        match self {
            Block::Code(lines) => lines.len().max(1),
            Block::Table(_) => 0,
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
}

impl InlineBuf {
    fn new() -> Self {
        InlineBuf { runs: vec![] }
    }
    fn push(&mut self, st: Style, link: Option<String>, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Some(last) = self.runs.last_mut() {
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
        while i < self.runs.len() {
            // a link spans consecutive runs with the same href
            if let Some(href) = &self.runs[i].1 {
                let mut j = i;
                let mut label = String::new();
                while j < self.runs.len() && self.runs[j].1.as_deref() == Some(href.as_str()) {
                    label.push_str(&mark(&self.runs[j].0, &self.runs[j].2, plain_bold_ok));
                    j += 1;
                }
                out.push_str(&format!("[{label}]({})", href.replace(' ', "%20").replace(')', "%29")));
                i = j;
                continue;
            }
            out.push_str(&mark(&self.runs[i].0, &self.runs[i].2, plain_bold_ok));
            i += 1;
        }
        out.trim().to_string()
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
    let mut blocks = vec![];
    // list stack: (ordered)
    let mut lists: Vec<bool> = vec![];
    let mut list_id = 0usize;
    let mut quote_depth = 0usize;
    let mut i = 0;
    // inside <body> only (Qt always writes one)
    if let Some(b) = toks.iter().position(|t| matches!(t, Tok::Open(n, _) if n == "body")) {
        i = b + 1;
    }
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
                let (buf, plain, next) = read_inline(&toks, i + 1, &name);
                i = next;
                let empty = css.contains("-qt-paragraph-type:empty") || plain.trim().is_empty();
                if name == "pre" {
                    code_run.push(plain.trim_end_matches('\n').to_string());
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
                    _ if empty => Block::Empty,
                    // Qt turns <blockquote> into a paragraph indented on both sides
                    _ if quote_depth > 0 || (css.contains("margin-left:40px") && css.contains("margin-right:40px")) => Block::Quote(buf.markdown(true)),
                    _ => Block::Para(buf.markdown(true)),
                };
                blocks.push(block);
            }
            Tok::Open(n, _) if n == "hr" => {
                flush_code(&mut code_run, &mut blocks);
                blocks.push(Block::Hr);
                i += 1;
            }
            Tok::Open(n, _) if n == "table" => {
                flush_code(&mut code_run, &mut blocks);
                let (rows, next) = read_table(&toks, i + 1);
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
                    buf.push(cur, link.clone(), " ");
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
        if matches!(b, Block::Empty) {
            continue;
        }
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
            Block::Code(lines) => out.push_str(&format!("```\n{}\n```", lines.join("\n"))),
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
            Block::Empty => {}
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

/// Qt's HTML -> Markdown.
pub fn to_md(html: &str) -> String {
    blocks_to_md(&blocks_from_html(html))
}

// ── Markdown -> HTML for Qt ──────────────────────────────────────────────────

fn esc_html(t: &str) -> String {
    t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Markdown -> HTML that Qt's importer turns into the same blocks `to_md` reads.
pub fn to_html(md: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_TASKLISTS);
    let mut out = String::from("<html><body>");
    let mut in_code = false;
    let mut code = String::new();
    let mut item_open: Vec<bool> = vec![];
    // Task markers arrive after the <li> start; remember where it was written.
    let mut li_at: Option<usize> = None;
    let mut table_head = false;
    let mut image_alt: Option<String> = None;
    for ev in Parser::new_ext(md, opts) {
        if let Some(alt) = image_alt.as_mut() {
            match ev {
                Event::End(TagEnd::Image) => {
                    let a = std::mem::take(alt);
                    image_alt = None;
                    out.push_str(&esc_html(&format!("[image{}]", if a.is_empty() { String::new() } else { format!(": {a}") })));
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
            Event::Start(Tag::Link { dest_url, .. }) => out.push_str(&format!("<a href=\"{}\">", esc_html(&dest_url))),
            Event::End(TagEnd::Link) => out.push_str("</a>"),
            Event::Start(Tag::Image { .. }) => image_alt = Some(String::new()),
            Event::Code(t) => out.push_str(&format!("<code>{}</code>", esc_html(&t))),
            Event::Text(t) => {
                if in_code {
                    code.push_str(&t)
                } else {
                    out.push_str(&esc_html(&t))
                }
            }
            Event::SoftBreak => out.push(' '),
            Event::HardBreak => out.push_str("<br />"),
            Event::Rule => out.push_str("<hr />"),
            Event::Html(h) | Event::InlineHtml(h) => out.push_str(&esc_html(&h)),
            _ => {}
        }
    }
    out.push_str("</body></html>");
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
        Block::Table(_) => "table",
        Block::Hr => "hr",
        _ => "p",
    }
}

/// Which block (and which line of a code block) holds Qt text block `index`.
fn locate(blocks: &[Block], index: usize) -> Option<(usize, usize)> {
    let mut n = 0;
    for (i, b) in blocks.iter().enumerate() {
        let k = b.qt_blocks();
        if k > 0 && index < n + k {
            return Some((i, index - n));
        }
        n += k;
    }
    None
}

/// The block kind at the view's text block `index` ("p", "h1".., "ul", "ol", "task",
/// "done", "quote", "code", "hr", or "" when out of range).
pub fn kind_at(html: &str, index: usize) -> String {
    let blocks = blocks_from_html(html);
    locate(&blocks, index).map(|(i, _)| kind_of(&blocks[i]).to_string()).unwrap_or_default()
}

/// Change the kind of the text block at `index` and return the document as HTML
/// for Qt. `kind` is "p", "h1", "h2", "h3", "ul", "ol", "task", "done", "quote" or "code".
pub fn set_block(html: &str, index: usize, kind: &str) -> Result<String, String> {
    let mut blocks = blocks_from_html(html);
    let (i, line) = locate(&blocks, index).ok_or("the cursor is not in a text block this editor can restyle")?;
    let inline_of = |b: &Block| -> String {
        match b {
            Block::Heading(_, t) | Block::Para(t) | Block::Quote(t) => t.clone(),
            Block::Item { inline, .. } => inline.clone(),
            Block::Code(lines) => escape_md(lines.get(line).map(String::as_str).unwrap_or("")),
            _ => String::new(),
        }
    };
    let old = blocks[i].clone();
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
    // Keep empty text blocks as empty paragraphs so the view's block count still lines up.
    Ok(to_html_blocks(&blocks))
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
            html.push_str("<p></p>");
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
            "# Title\n\nSome **bold**, *it*, ~~gone~~, `code` and [link](https://x.io).\n\n- one\n- two\n    - nested\n\n1. first\n\n- [ ] task\n- [x] done\n\n> quote\n\n```\nline1\nline2\n```\n\n| a | b |\n| --- | --- |\n| 1 \\< 2 & x\\*y | 2 |\n\nend\n"
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
        // index counts Qt text blocks: Title=0 Some=1 one=2 two=3 nested=4 first=5 task=6 done=7 quote=8 line1=9 line2=10, table=none, empty=11, end=12
        assert_eq!(kind_at(QT_HTML, 0), "h1");
        assert_eq!(kind_at(QT_HTML, 4), "ul");
        assert_eq!(kind_at(QT_HTML, 6), "task");
        assert_eq!(kind_at(QT_HTML, 8), "quote");
        assert_eq!(kind_at(QT_HTML, 10), "code");
        assert_eq!(kind_at(QT_HTML, 12), "p");
        let md = |h: String| to_md(&h);
        assert!(md(set_block(QT_HTML, 1, "h2").unwrap()).contains("\n## Some bold, *it*"));
        assert!(md(set_block(QT_HTML, 0, "p").unwrap()).starts_with("Title\n\nSome"));
        assert!(md(set_block(QT_HTML, 6, "done").unwrap()).contains("- [x] task\n- [x] done"));
        assert!(md(set_block(QT_HTML, 12, "task").unwrap()).ends_with("- [ ] end\n"));
        // a line out of a code block splits it
        let split = md(set_block(QT_HTML, 9, "p").unwrap());
        assert!(split.contains("> quote\n\nline1\n\n```\nline2\n```"), "{split}");
        // nested item keeps its depth
        assert!(md(set_block(QT_HTML, 4, "task").unwrap()).contains("    - [ ] nested"));
        assert!(set_block(QT_HTML, 99, "p").is_err());
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
