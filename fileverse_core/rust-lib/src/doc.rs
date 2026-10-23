//! A dDoc's Yjs state -> a flat list of blocks the QML view can draw, plus a
//! Markdown export.
//!
//! Every string that reaches the view is escaped here, and the only markup in a
//! block's `html` is markup this file wrote (`<b>`, `<i>`, `<u>`, `<s>`, `<code>`,
//! `<br>`, and `<a href>` limited to http/https/mailto). Nothing the author wrote
//! can become a tag, and images are never fetched: they are encrypted uploads, and
//! loading a remote URL from a document would tell its author who opened it.

use std::collections::HashMap;

use serde::Serialize;
use yrs::types::text::YChange;
use yrs::types::Attrs;
use yrs::{Any, Doc, GetString, Out, ReadTxn, Text, Transact, Update, Xml, XmlFragment, XmlOut};
use yrs::updates::decoder::Decode;

#[derive(Clone, Debug, Default, Serialize)]
pub struct Block {
    /// h1..h6, p, li, oli, task, code, hr, image, table, embed, pagebreak
    pub t: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub depth: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub quote: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub callout: bool,
    /// Escaped, view-safe rich text (Qt StyledText subset).
    pub html: String,
    /// The same text, plain.
    pub text: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub lang: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<Vec<String>>,
    #[serde(skip_serializing, default)]
    md: String,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

#[derive(Clone, Debug, Serialize)]
pub struct Tab {
    pub id: String,
    pub name: String,
    pub depth: u32,
    pub blocks: Vec<Block>,
    /// The tab as a slide deck, the way dDocs' presentation mode cuts it.
    pub slides: Vec<Slide>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Slide {
    /// `title` (a lone H1), `image`, or `content`.
    pub kind: String,
    /// Indices into the tab's `blocks`.
    pub blocks: Vec<usize>,
}

// Limits from fileverse-ddocs `utils/md-to-slides.ts` (processMarkdownContent).
const SLIDE_MAX_CHARS: usize = 1000;
const SLIDE_MAX_WORDS: usize = 250;
const SLIDE_MAX_LINES: usize = 8;

fn block_lines(b: &Block) -> usize {
    match b.t.as_str() {
        "table" => b.rows.len() + 1,
        _ => b.text.matches('\n').count() + 1,
    }
}

/// Cut blocks into slides with dDocs' presentation-mode rules: a page break (or a
/// `===` line, the md2slides marker) ends a slide; an H1 is a title slide on its
/// own; an H2 starts a slide; an image gets a slide; otherwise content flows until
/// a slide would pass 8 lines, 1000 characters or 250 words.
pub fn slides(blocks: &[Block]) -> Vec<Slide> {
    let mut out: Vec<Slide> = vec![];
    let mut cur: Vec<usize> = vec![];
    let (mut chars, mut words, mut lines) = (0usize, 0usize, 0usize);
    let flush = |cur: &mut Vec<usize>, out: &mut Vec<Slide>, c: &mut usize, w: &mut usize, l: &mut usize| {
        if !cur.is_empty() {
            out.push(Slide { kind: "content".into(), blocks: std::mem::take(cur) });
        }
        *c = 0;
        *w = 0;
        *l = 0;
    };
    for (i, b) in blocks.iter().enumerate() {
        let empty = b.text.trim().is_empty() && b.rows.is_empty();
        match b.t.as_str() {
            "pagebreak" => flush(&mut cur, &mut out, &mut chars, &mut words, &mut lines),
            "p" if b.text.trim().starts_with("===") => flush(&mut cur, &mut out, &mut chars, &mut words, &mut lines),
            _ if empty && b.t != "hr" && b.t != "image" => {}
            "h1" => {
                flush(&mut cur, &mut out, &mut chars, &mut words, &mut lines);
                out.push(Slide { kind: "title".into(), blocks: vec![i] });
            }
            "h2" => {
                flush(&mut cur, &mut out, &mut chars, &mut words, &mut lines);
                cur.push(i);
                chars = b.text.len();
                words = b.text.split_whitespace().count();
                lines = 1;
            }
            "image" => {
                if cur.is_empty() {
                    out.push(Slide { kind: "image".into(), blocks: vec![i] });
                } else {
                    cur.push(i);
                }
                flush(&mut cur, &mut out, &mut chars, &mut words, &mut lines);
            }
            _ => {
                let (c, w, l) = (b.text.chars().count(), b.text.split_whitespace().count(), block_lines(b));
                let over = chars + c > SLIDE_MAX_CHARS || words + w > SLIDE_MAX_WORDS || lines + l > SLIDE_MAX_LINES;
                if over && !cur.is_empty() {
                    // Keep a slide's H2 with what follows it when the H2 is all there is.
                    let carry = cur.len() == 1 && blocks[cur[0]].t == "h2";
                    if !carry {
                        // A heading never ends a slide: it moves on with its content.
                        let last = *cur.last().unwrap();
                        let orphan = cur.len() > 1 && matches!(blocks[last].t.as_str(), "h3" | "h4" | "h5" | "h6");
                        if orphan {
                            cur.pop();
                        }
                        flush(&mut cur, &mut out, &mut chars, &mut words, &mut lines);
                        if orphan {
                            cur.push(last);
                            chars = blocks[last].text.chars().count();
                            words = blocks[last].text.split_whitespace().count();
                            lines = 1;
                        }
                    }
                }
                cur.push(i);
                chars += c;
                words += w;
                lines += l;
            }
        }
    }
    flush(&mut cur, &mut out, &mut chars, &mut words, &mut lines);
    out
}

pub fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c => o.push(c),
        }
    }
    o
}

fn safe_href(h: &str) -> Option<String> {
    let l = h.trim().to_ascii_lowercase();
    (l.starts_with("https://") || l.starts_with("http://") || l.starts_with("mailto:"))
        .then(|| escape(h.trim()))
}

#[derive(Default)]
struct Inline {
    html: String,
    text: String,
    md: String,
}

impl Inline {
    fn br(&mut self) {
        self.html.push_str("<br>");
        self.text.push('\n');
        self.md.push_str("  \n");
    }

    fn span(&mut self, s: &str, attrs: Option<&Attrs>) {
        if s.is_empty() {
            return;
        }
        let has = |k: &str| attrs.map(|a| a.contains_key(k)).unwrap_or(false);
        let href = attrs
            .and_then(|a| a.get("link"))
            .and_then(|v| match v {
                Any::Map(m) => m.get("href").and_then(|h| match h {
                    Any::String(s) => Some(s.to_string()),
                    _ => None,
                }),
                _ => None,
            });
        let mut open = String::new();
        let mut close = String::new();
        let mut md_open = String::new();
        let mut md_close = String::new();
        let mut wrap = |o: &str, c: &str, mo: &str, mc: &str| {
            open.push_str(o);
            close.insert_str(0, c);
            md_open.push_str(mo);
            md_close.insert_str(0, mc);
        };
        if has("code") {
            wrap("<code>", "</code>", "`", "`");
        }
        if has("bold") {
            wrap("<b>", "</b>", "**", "**");
        }
        if has("italic") {
            wrap("<i>", "</i>", "_", "_");
        }
        if has("underline") {
            wrap("<u>", "</u>", "", "");
        }
        if has("strike") {
            wrap("<s>", "</s>", "~~", "~~");
        }
        let body = escape(s).replace('\n', "<br>");
        match href.as_deref().and_then(safe_href) {
            Some(h) => {
                self.html.push_str(&format!("<a href=\"{h}\">{open}{body}{close}</a>"));
                self.md.push_str(&format!("[{md_open}{s}{md_close}]({})", href.unwrap()));
            }
            None => {
                self.html.push_str(&format!("{open}{body}{close}"));
                // Markdown delimiters must hug the text: `**bold **` is not bold.
                let core = s.trim();
                if core.is_empty() || md_open.is_empty() {
                    self.md.push_str(s);
                } else {
                    let lead = &s[..s.len() - s.trim_start().len()];
                    let trail = &s[s.trim_end().len()..];
                    self.md.push_str(&format!("{lead}{md_open}{core}{md_close}{trail}"));
                }
            }
        }
        self.text.push_str(s);
    }
}

fn attr<T: ReadTxn>(txn: &T, el: &yrs::XmlElementRef, k: &str) -> Option<String> {
    el.get_attribute(txn, k)
}

fn inline_of<T: ReadTxn>(txn: &T, node: &XmlOut, out: &mut Inline) {
    match node {
        XmlOut::Text(t) => {
            for d in t.diff(txn, YChange::identity) {
                // Embeds inside text (non-string inserts) carry no readable text.
                let s = match &d.insert {
                    Out::Any(Any::String(s)) => s.to_string(),
                    _ => String::new(),
                };
                out.span(&s, d.attributes.as_deref());
            }
        }
        XmlOut::Element(e) => match e.tag().as_ref() {
            "hardBreak" => out.br(),
            "mention" | "emoji" => {
                let label = attr(txn, e, "label")
                    .or_else(|| attr(txn, e, "name"))
                    .unwrap_or_default();
                out.span(&label, None);
            }
            "footnoteRef" => out.span("*", None),
            "mathInline" | "inlineMath" => {
                let l = attr(txn, e, "latex").unwrap_or_default();
                let mut a = Attrs::new();
                a.insert("code".into(), Any::Bool(true));
                out.span(&l, Some(&a));
            }
            _ => {
                for c in e.children(txn) {
                    inline_of(txn, &c, out);
                }
            }
        },
        XmlOut::Fragment(f) => {
            for c in f.children(txn) {
                inline_of(txn, &c, out);
            }
        }
    }
}

struct Ctx {
    depth: u32,
    quote: bool,
    callout: bool,
}

fn trim_breaks(mut inl: Inline) -> Inline {
    while let Some(r) = inl.html.strip_prefix("<br>") {
        inl.html = r.to_string();
    }
    while let Some(r) = inl.html.strip_suffix("<br>") {
        inl.html = r.to_string();
    }
    inl.text = inl.text.trim_matches('\n').to_string();
    inl.md = inl.md.trim_start_matches("  \n").trim_end_matches("  \n").to_string();
    inl
}

fn push<'a>(blocks: &'a mut Vec<Block>, ctx: &Ctx, t: &str, inl: Inline) -> &'a mut Block {
    let inl = if t == "code" { inl } else { trim_breaks(inl) };
    blocks.push(Block {
        t: t.into(),
        depth: ctx.depth,
        quote: ctx.quote,
        callout: ctx.callout,
        html: inl.html,
        text: inl.text,
        md: inl.md,
        ..Default::default()
    });
    blocks.last_mut().unwrap()
}

fn walk<T: ReadTxn>(txn: &T, node: &XmlOut, ctx: &Ctx, blocks: &mut Vec<Block>) {
    let XmlOut::Element(e) = node else {
        if let XmlOut::Text(_) = node {
            let mut i = Inline::default();
            inline_of(txn, node, &mut i);
            if !i.text.trim().is_empty() {
                push(blocks, ctx, "p", i);
            }
        }
        return;
    };
    let tag = e.tag().to_string();
    let kids = || e.children(txn).collect::<Vec<_>>();
    match tag.as_str() {
        "heading" => {
            let lvl = attr(txn, e, "level").and_then(|l| l.parse::<f64>().ok()).unwrap_or(1.0) as u32;
            let mut i = Inline::default();
            for c in kids() {
                inline_of(txn, &c, &mut i);
            }
            push(blocks, ctx, &format!("h{}", lvl.clamp(1, 6)), i);
        }
        "paragraph" => {
            let mut i = Inline::default();
            for c in kids() {
                inline_of(txn, &c, &mut i);
            }
            push(blocks, ctx, "p", i);
        }
        "bulletList" | "orderedList" | "taskList" => {
            let ordered = tag == "orderedList";
            let mut n = attr(txn, e, "start").and_then(|s| s.parse::<f64>().ok()).unwrap_or(1.0) as u32;
            for item in kids() {
                let XmlOut::Element(li) = &item else { continue };
                let checked = (tag == "taskList")
                    .then(|| attr(txn, li, "checked").map(|c| c == "true").unwrap_or(false));
                let mut first = true;
                for part in li.children(txn) {
                    let is_list = matches!(&part, XmlOut::Element(p) if p.tag().ends_with("List"));
                    if first && !is_list {
                        let mut i = Inline::default();
                        inline_of(txn, &part, &mut i);
                        let t = if ordered { "oli" } else if checked.is_some() { "task" } else { "li" };
                        let b = push(blocks, ctx, t, i);
                        b.num = ordered.then_some(n);
                        b.checked = checked;
                        first = false;
                    } else {
                        let inner = Ctx { depth: ctx.depth + 1, quote: ctx.quote, callout: ctx.callout };
                        walk(txn, &part, &inner, blocks);
                    }
                }
                n += 1;
            }
        }
        "blockquote" => {
            let inner = Ctx { depth: ctx.depth, quote: true, callout: ctx.callout };
            for c in kids() {
                walk(txn, &c, &inner, blocks);
            }
        }
        "callout" => {
            let inner = Ctx { depth: ctx.depth, quote: ctx.quote, callout: true };
            for c in kids() {
                walk(txn, &c, &inner, blocks);
            }
        }
        "codeBlock" => {
            let mut text = String::new();
            for c in kids() {
                if let XmlOut::Text(t) = c {
                    text.push_str(&t.get_string(txn));
                }
            }
            let b = push(
                blocks,
                ctx,
                "code",
                Inline { html: escape(&text).replace('\n', "<br>"), md: text.clone(), text },
            );
            b.lang = attr(txn, e, "language").unwrap_or_default();
        }
        "horizontalRule" => {
            push(blocks, ctx, "hr", Inline::default());
        }
        "pageBreak" => {
            push(blocks, ctx, "pagebreak", Inline::default());
        }
        "image" | "resizableMedia" => {
            let caption = attr(txn, e, "caption").or_else(|| attr(txn, e, "alt")).unwrap_or_default();
            let i = Inline { html: escape(&caption), text: caption.clone(), md: String::new() };
            push(blocks, ctx, "image", i);
        }
        "iframe" | "embeddedTweet" | "actionButton" | "linkPreview" => {
            let src = attr(txn, e, "src")
                .or_else(|| attr(txn, e, "url"))
                .or_else(|| attr(txn, e, "href"))
                .unwrap_or_default();
            let i = Inline { html: escape(&src), text: src.clone(), md: src };
            push(blocks, ctx, "embed", i);
        }
        "table" => {
            let mut rows = vec![];
            let mut md_rows = vec![];
            for row in kids() {
                let XmlOut::Element(r) = &row else { continue };
                let mut cells = vec![];
                let mut md_cells = vec![];
                for cell in r.children(txn) {
                    let mut i = Inline::default();
                    let mut first = true;
                    if let XmlOut::Element(c) = &cell {
                        for p in c.children(txn) {
                            if !first {
                                i.br();
                            }
                            inline_of(txn, &p, &mut i);
                            first = false;
                        }
                    }
                    md_cells.push(i.md.replace('|', "\\|").replace("  \n", " "));
                    cells.push(i.html);
                }
                rows.push(cells);
                md_rows.push(md_cells);
            }
            let mut md = String::new();
            for (k, r) in md_rows.iter().enumerate() {
                md.push_str(&format!("| {} |\n", r.join(" | ")));
                if k == 0 {
                    md.push_str(&format!("|{}\n", " --- |".repeat(r.len())));
                }
            }
            let b = push(blocks, ctx, "table", Inline { md, ..Default::default() });
            b.rows = rows;
        }
        _ => {
            // dBlock, columns, column, details, footnotes and anything new:
            // transparent containers. Unknown content is shown, not dropped.
            for c in kids() {
                walk(txn, &c, ctx, blocks);
            }
        }
    }
}

pub fn blocks_of_fragment<T: ReadTxn>(txn: &T, frag: &yrs::XmlFragmentRef) -> Vec<Block> {
    let ctx = Ctx { depth: 0, quote: false, callout: false };
    let mut blocks = vec![];
    for c in frag.children(txn) {
        walk(txn, &c, &ctx, &mut blocks);
    }
    // An empty paragraph between blocks is spacing, a run of them is noise.
    blocks.dedup_by(|b, a| b.t == "p" && a.t == "p" && b.text.is_empty() && a.text.is_empty());
    blocks
}

fn map_strings<T: ReadTxn>(txn: &T, m: &yrs::MapRef) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for (k, v) in yrs::Map::iter(m, txn) {
        if let Out::Any(Any::String(s)) = v {
            out.insert(k.to_string(), s.to_string());
        } else if let Out::Any(Any::Bool(true)) = v {
            out.insert(k.to_string(), "true".into());
        }
    }
    out
}

pub fn load(update: &[u8]) -> Result<Doc, String> {
    let doc = Doc::new();
    {
        let u = Update::decode_v1(update).map_err(|e| format!("not a Yjs update: {e}"))?;
        let mut txn = doc.transact_mut();
        txn.apply_update(u).map_err(|e| format!("Yjs update failed to apply: {e}"))?;
    }
    Ok(doc)
}

/// The document's tabs in reading order (docs/TABS_SPEC.md §1), each rendered.
pub fn tabs(doc: &Doc) -> Vec<Tab> {
    // Root types must be declared before the read transaction is taken.
    let order_arr = doc.get_or_insert_array("tabs_order");
    let name_map = doc.get_or_insert_map("tabs_name_registry");
    let parent_map = doc.get_or_insert_map("tabs_parent_registry");
    let txn = doc.transact();
    let mut order: Vec<String> = yrs::Array::iter(&order_arr, &txn)
        .filter_map(|v| match v {
            Out::Any(Any::String(s)) => Some(s.to_string()),
            _ => None,
        })
        .collect();
    let names = map_strings(&txn, &name_map);
    let parents = map_strings(&txn, &parent_map);
    drop(txn);
    order.retain(|id| names.contains_key(id) || id == "default");
    if order.is_empty() {
        order.push("default".into());
    }
    let mut seen = std::collections::HashSet::new();
    order.retain(|id| seen.insert(id.clone()));
    let depth_of = |id: &str| {
        let mut d = 0;
        let mut cur = id.to_string();
        while let Some(p) = parents.get(&cur) {
            d += 1;
            if d > 3 {
                break;
            }
            cur = p.clone();
        }
        d
    };
    let frags: Vec<_> = order.iter().map(|id| (id.clone(), doc.get_or_insert_xml_fragment(id.as_str()))).collect();
    let txn = doc.transact();
    frags
        .into_iter()
        .map(|(id, f)| {
            let blocks = blocks_of_fragment(&txn, &f);
            Tab {
                name: names.get(&id).cloned().unwrap_or_else(|| if id == "default" { "Tab 1".into() } else { id.clone() }),
                depth: depth_of(&id),
                slides: slides(&blocks),
                blocks,
                id,
            }
        })
        .collect()
}

pub fn markdown(title: &str, tabs: &[Tab]) -> String {
    let mut md = String::new();
    if !title.is_empty() {
        md.push_str(&format!("# {title}\n\n"));
    }
    for tab in tabs {
        if tabs.len() > 1 {
            md.push_str(&format!("{} {}\n\n", "#".repeat((tab.depth as usize + 2).min(4)), tab.name));
        }
        for b in &tab.blocks {
            let indent = "  ".repeat(b.depth as usize);
            let q = if b.quote || b.callout { "> " } else { "" };
            let line = match b.t.as_str() {
                h if h.starts_with('h') && h.len() == 2 => format!("{} {}", "#".repeat(h[1..].parse().unwrap_or(1)), b.md),
                "li" => format!("{indent}- {}", b.md),
                "oli" => format!("{indent}{}. {}", b.num.unwrap_or(1), b.md),
                "task" => format!("{indent}- [{}] {}", if b.checked == Some(true) { "x" } else { " " }, b.md),
                "code" => format!("```{}\n{}\n```", b.lang, b.md),
                "hr" | "pagebreak" => "---".into(),
                "image" => format!("*[image{}]*", if b.text.is_empty() { String::new() } else { format!(": {}", b.text) }),
                "embed" => format!("<{}>", b.md),
                "table" => b.md.trim_end().to_string(),
                _ => b.md.clone(),
            };
            let is_item = |t: &str| matches!(t, "li" | "oli" | "task");
            // A list must end with a blank line, or the next block joins its last item.
            if !is_item(&b.t) && md.ends_with('\n') && !md.ends_with("\n\n") && !md.is_empty() {
                md.push('\n');
            }
            md.push_str(q);
            md.push_str(&line);
            md.push_str(if is_item(&b.t) { "\n" } else { "\n\n" });
        }
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(t: &str, text: &str) -> Block {
        Block { t: t.into(), text: text.into(), html: escape(text), ..Default::default() }
    }

    fn kinds(s: &[Slide]) -> Vec<(String, Vec<usize>)> {
        s.iter().map(|x| (x.kind.clone(), x.blocks.clone())).collect()
    }

    #[test]
    fn ddocs_presentation_rules() {
        let blocks = vec![
            b("h1", "Deck"),          // 0 title slide
            b("h2", "Why"),           // 1 starts a slide
            b("p", "one"),            // 2
            b("li", "two"),           // 3
            b("pagebreak", ""),       // 4 ends it
            b("p", "after break"),    // 5
            b("image", ""),           // 6 joins, then ends the slide
            b("image", "solo"),       // 7 its own slide
            b("p", ""),               // 8 empty: skipped
            b("h2", "Next"),          // 9
            b("p", "==="),            // 10 md2slides break
            b("p", "tail"),           // 11
        ];
        assert_eq!(
            kinds(&slides(&blocks)),
            vec![
                ("title".into(), vec![0]),
                ("content".into(), vec![1, 2, 3]),
                ("content".into(), vec![5, 6]),
                ("image".into(), vec![7]),
                ("content".into(), vec![9]),
                ("content".into(), vec![11]),
            ]
        );
    }

    #[test]
    fn long_content_splits_at_eight_lines() {
        let mut blocks = vec![b("h2", "Many")];
        for i in 0..12 {
            blocks.push(b("li", &format!("item {i}")));
        }
        let s = slides(&blocks);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].blocks, (0..8).collect::<Vec<_>>());
        assert_eq!(s[1].blocks, (8..13).collect::<Vec<_>>());
    }

    #[test]
    fn a_heading_does_not_end_a_slide() {
        let mut blocks = vec![b("h2", "Part")];
        for i in 0..6 {
            blocks.push(b("p", &format!("line {i}")));
        }
        blocks.push(b("h3", "Usage")); // 7: would be the 8th line, its code the 9th
        blocks.push(Block { t: "code".into(), text: "a\nb".into(), ..Default::default() });
        let s = slides(&blocks);
        assert_eq!(s[0].blocks, (0..7).collect::<Vec<_>>());
        assert_eq!(s[1].blocks, vec![7, 8]);
    }

    #[test]
    fn markdown_marks_hug_text_and_lists_end() {
        let mut bold = Attrs::new();
        bold.insert("bold".into(), Any::Bool(true));
        let mut i = Inline::default();
        i.span("Read data ", Some(&bold));
        i.span("- rest", None);
        assert_eq!(i.md, "**Read data** - rest");
        let blocks = vec![
            Block { t: "li".into(), md: "one".into(), text: "one".into(), ..Default::default() },
            Block { t: "h3".into(), md: "Next".into(), text: "Next".into(), ..Default::default() },
        ];
        let tabs = vec![Tab { id: "default".into(), name: "T".into(), depth: 0, slides: vec![], blocks }];
        assert_eq!(markdown("", &tabs), "- one\n\n### Next\n\n");
    }

    #[test]
    fn long_paragraphs_split_by_words() {
        let para = vec!["word"; 200].join(" ");
        let blocks = vec![b("p", &para), b("p", &para)];
        assert_eq!(slides(&blocks).len(), 2);
    }
}
