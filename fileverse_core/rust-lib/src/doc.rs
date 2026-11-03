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
    /// `columns`: each column's own blocks, side by side.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cols: Vec<Vec<Block>>,
    /// `columns`: dDocs' layout, `align-center` (equal), `align-right` (wide left) or
    /// `align-left` (wide right).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub layout: String,
    /// `image`: where the encrypted image is and how to open it (fetched on request).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub img: Option<ImageRef>,
    #[serde(skip_serializing, default)]
    md: String,
}

/// An image a dDoc embeds: AES-GCM ciphertext on IPFS (`cid`), with the key, IV and
/// tag in the node's attributes (`encryptionKey`, `nonce`, `authTag`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, serde::Deserialize)]
pub struct ImageRef {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cid: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub nonce: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tag: String,
    #[serde(default)]
    pub mime: String,
    /// The node's width (`100%`, `320px`, …) and alignment, as dDocs stores them.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub width: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub align: String,
    /// A draft's own image, not uploaded yet: `<draft id>/<file>` in the drafts folder.
    /// Publishing encrypts it and replaces it with a `cid` reference.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub local: String,
}

/// Images in this app's Markdown: `![alt](fvimg:<base64url of the reference as JSON>)`.
pub const IMAGE_SCHEME: &str = "fvimg:";

impl ImageRef {
    pub fn to_url(&self) -> String {
        format!("{IMAGE_SCHEME}{}", crate::b64::encode_url(serde_json::to_string(self).unwrap_or_default().as_bytes()))
    }
    pub fn from_url(u: &str) -> Option<ImageRef> {
        let b = crate::b64::decode(u.strip_prefix(IMAGE_SCHEME)?).ok()?;
        let r: ImageRef = serde_json::from_slice(&b).ok()?;
        (!r.local.is_empty() || (!r.cid.is_empty() && !r.key.is_empty())).then_some(r)
    }
}

/// The IPFS CID of an image node: `ipfsHash` (possibly lane-prefixed) or the path of
/// `ipfsUrl`. Images in Fileverse's private lane are only served by Fileverse.
fn image_cid(hash: &str, url: &str) -> Option<String> {
    let looks_cid = |c: &str| (c.starts_with("Qm") && c.len() == 46) || (c.starts_with('b') && c.len() > 50);
    let h = hash.rsplit([':', '/']).next().unwrap_or("");
    if looks_cid(h) && !hash.starts_with("private") {
        return Some(h.to_string());
    }
    let p = url.split("/ipfs/").nth(1)?.split(['/', '?', '#']).next()?;
    looks_cid(p).then(|| p.to_string())
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

/// Cut blocks into slides at the document's structure, the way dDocs' direct-render
/// presentation does (fileverse-ddocs#550): a page break (or a `===` line, the
/// md2slides marker) ends a slide, and an H1 or H2 opens one with what follows packed
/// beneath it, so a title is never alone on a slide. How much fits on one slide is
/// decided by the view, which measures the real stage and breaks between blocks
/// (list items one by one; never inside columns or tables). `kind` is a layout hint:
/// `title` when a slide holds only headings, `image` for a lone image.
pub fn slides(blocks: &[Block]) -> Vec<Slide> {
    let mut out: Vec<Slide> = vec![];
    let mut cur: Vec<usize> = vec![];
    let kind = |cur: &[usize]| -> String {
        if cur.len() == 1 && blocks[cur[0]].t == "image" {
            "image".into()
        } else if cur.iter().all(|&i| is_heading(&blocks[i])) {
            "title".into()
        } else {
            "content".into()
        }
    };
    let flush = |cur: &mut Vec<usize>, out: &mut Vec<Slide>| {
        if !cur.is_empty() {
            out.push(Slide { kind: kind(cur), blocks: std::mem::take(cur) });
        }
    };
    for (i, b) in blocks.iter().enumerate() {
        let shows = !b.text.trim().is_empty() || !b.rows.is_empty() || matches!(b.t.as_str(), "hr" | "image" | "columns" | "code");
        match b.t.as_str() {
            "pagebreak" => flush(&mut cur, &mut out),
            "p" if b.text.trim().starts_with("===") => flush(&mut cur, &mut out),
            _ if !shows => {}
            "h1" | "h2" => {
                flush(&mut cur, &mut out);
                cur.push(i);
            }
            _ => cur.push(i),
        }
    }
    flush(&mut cur, &mut out);
    out
}

fn is_heading(b: &Block) -> bool {
    b.t.len() == 2 && b.t.starts_with('h') && b.t.as_bytes()[1].is_ascii_digit()
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
            let a = |k: &str| attr(txn, e, k).unwrap_or_default();
            let local = a("localRef");
            let img = if !local.is_empty() {
                Some(ImageRef { local, mime: a("mimeType"), width: a("width"), align: a("dataAlign"), ..Default::default() })
            } else {
                image_cid(&a("ipfsHash"), &a("ipfsUrl")).and_then(|cid| {
                let (key, nonce, tag) = (a("encryptionKey"), a("nonce"), a("authTag"));
                (!key.is_empty() && !nonce.is_empty() && !tag.is_empty()).then(|| ImageRef {
                    cid,
                    key,
                    nonce,
                    tag,
                    mime: a("mimeType"),
                    width: a("width"),
                    align: a("dataAlign"),
                    local: String::new(),
                })
            })
            };
            push(blocks, ctx, "image", i).img = img;
        }
        "columns" => {
            // Side by side (an image beside text): one block holding each column's blocks.
            let mut cols = vec![];
            let mut text = String::new();
            for c in kids() {
                let mut cb = vec![];
                let inner = Ctx { depth: 0, quote: false, callout: false };
                match &c {
                    XmlOut::Element(ce) if ce.tag().to_string() == "column" => {
                        for k in ce.children(txn) {
                            walk(txn, &k, &inner, &mut cb);
                        }
                    }
                    other => walk(txn, other, &inner, &mut cb),
                }
                for b in &cb {
                    text.push_str(&b.text);
                    text.push('\n');
                }
                cols.push(cb);
            }
            cols.retain(|c| !c.is_empty());
            if !cols.is_empty() {
                let b = push(blocks, ctx, "columns", Inline { text: text.trim_end().to_string(), ..Default::default() });
                b.layout = attr(txn, e, "layout").unwrap_or_else(|| "align-center".into());
                b.cols = cols;
            }
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
            // dBlock, column, details, footnotes and anything new:
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
        blocks_md(&tab.blocks, &mut md);
    }
    md
}

fn blocks_md(blocks: &[Block], md: &mut String) {
    {
        for b in blocks {
            if b.t == "columns" {
                // This app's Markdown keeps columns: `::: columns <layout>`, `::: column`, `:::`.
                if !md.is_empty() && !md.ends_with("\n\n") {
                    md.push('\n');
                }
                md.push_str(&format!("::: columns {}\n\n", if b.layout.is_empty() { "align-center" } else { &b.layout }));
                for (k, c) in b.cols.iter().enumerate() {
                    if k > 0 {
                        md.push_str("::: column\n\n");
                    }
                    blocks_md(c, md);
                    if !md.ends_with("\n\n") {
                        md.push('\n');
                    }
                }
                md.push_str(":::\n\n");
                continue;
            }
            // an empty paragraph (the line an image was written on) adds nothing
            if b.t == "p" && b.md.trim().is_empty() {
                continue;
            }
            let indent = "  ".repeat(b.depth as usize);
            let q = if b.quote || b.callout { "> " } else { "" };
            let line = match b.t.as_str() {
                h if h.starts_with('h') && h.len() == 2 => format!("{} {}", "#".repeat(h[1..].parse().unwrap_or(1)), b.md),
                "li" => format!("{indent}- {}", b.md),
                "oli" => format!("{indent}{}. {}", b.num.unwrap_or(1), b.md),
                "task" => format!("{indent}- [{}] {}", if b.checked == Some(true) { "x" } else { " " }, b.md),
                "code" => format!("```{}\n{}\n```", b.lang, b.md),
                "hr" | "pagebreak" => "---".into(),
                "image" => match &b.img {
                    Some(r) => format!("![{}]({})", b.text.replace([']', '['], ""), r.to_url()),
                    None => format!("*[image{}]*", if b.text.is_empty() { String::new() } else { format!(": {}", b.text) }),
                },
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
            b("h1", "Deck"),          // 0 opens a slide; what follows packs beneath
            b("p", "intro"),          // 1
            b("h2", "Why"),           // 2 opens the next
            b("p", "one"),            // 3
            b("li", "two"),           // 4
            b("pagebreak", ""),       // 5 ends it
            b("p", "after break"),    // 6
            b("image", ""),           // 7 stays with it (the view measures)
            b("pagebreak", ""),       // 8
            b("image", "solo"),       // 9 alone: an image slide
            b("p", ""),               // 10 empty: skipped
            b("h2", "Next"),          // 11 a heading alone: a title slide
            b("p", "==="),            // 12 md2slides break
            b("p", "tail"),           // 13
        ];
        assert_eq!(
            kinds(&slides(&blocks)),
            vec![
                ("content".into(), vec![0, 1]),
                ("content".into(), vec![2, 3, 4]),
                ("content".into(), vec![6, 7]),
                ("image".into(), vec![9]),
                ("title".into(), vec![11]),
                ("content".into(), vec![13]),
            ]
        );
    }

    /// dDocs' shape for an image beside text: dBlock > columns > column > (resizableMedia | paragraph).
    #[test]
    fn columns_keep_an_image_beside_text() {
        use yrs::{Text, Transact, XmlElementPrelim, XmlFragment, XmlTextPrelim};
        let doc = Doc::new();
        let frag = doc.get_or_insert_xml_fragment("default");
        {
            let mut txn = doc.transact_mut();
            let d = frag.push_back(&mut txn, XmlElementPrelim::empty("dBlock"));
            let h = d.push_back(&mut txn, XmlElementPrelim::empty("heading"));
            h.insert_attribute(&mut txn, "level", "2");
            let t = h.push_back(&mut txn, XmlTextPrelim::new(""));
            t.insert(&mut txn, 0, "Beside");
            let d = frag.push_back(&mut txn, XmlElementPrelim::empty("dBlock"));
            let cols = d.push_back(&mut txn, XmlElementPrelim::empty("columns"));
            cols.insert_attribute(&mut txn, "layout", "align-right");
            let c1 = cols.push_back(&mut txn, XmlElementPrelim::empty("column"));
            let p = c1.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
            let t = p.push_back(&mut txn, XmlTextPrelim::new(""));
            t.insert(&mut txn, 0, "Text on the left");
            let c2 = cols.push_back(&mut txn, XmlElementPrelim::empty("column"));
            let img = c2.push_back(&mut txn, XmlElementPrelim::empty("resizableMedia"));
            for (k, v) in [
                ("ipfsHash", "bafkreidybxhs4ymbvt53hvxmz3f6nekg5gfit7c7pl7pul7fhfedsibm2a"),
                ("encryptionKey", "a2V5"),
                ("nonce", "bm9uY2U="),
                ("authTag", "dGFn"),
                ("mimeType", "image/png"),
                ("width", "100%"),
            ] {
                img.insert_attribute(&mut txn, k, v);
            }
        }
        let tabs = tabs(&doc);
        let blocks = &tabs[0].blocks;
        assert_eq!(blocks.len(), 2, "{blocks:?}");
        let c = &blocks[1];
        assert_eq!((c.t.as_str(), c.layout.as_str(), c.cols.len()), ("columns", "align-right", 2));
        assert_eq!(c.cols[0][0].text, "Text on the left");
        let img = c.cols[1][0].img.as_ref().expect("image reference");
        assert_eq!((img.cid.as_str(), img.mime.as_str()), ("bafkreidybxhs4ymbvt53hvxmz3f6nekg5gfit7c7pl7pul7fhfedsibm2a", "image/png"));
        // the heading opens the slide and the columns pack beneath it
        assert_eq!(tabs[0].slides, vec![Slide { kind: "content".into(), blocks: vec![0, 1] }]);
        // Markdown has no columns: they follow each other
        assert!(markdown("", &tabs).contains("Text on the left"));
    }

    #[test]
    fn image_cids() {
        let cid = "bafkreidybxhs4ymbvt53hvxmz3f6nekg5gfit7c7pl7pul7fhfedsibm2a";
        assert_eq!(image_cid(cid, "").as_deref(), Some(cid));
        assert_eq!(image_cid("", &format!("https://ipfs.fileverse.io/ipfs/{cid}?x=1")).as_deref(), Some(cid));
        assert_eq!(image_cid(&format!("public:{cid}"), "").as_deref(), Some(cid));
        // Fileverse's private lane: only Fileverse serves it
        assert_eq!(image_cid(&format!("private:{cid}"), "https://x/private/gateway?cid=x"), None);
        assert_eq!(image_cid("", "https://s3.example/img.png"), None);
    }

    #[test]
    fn long_content_is_left_to_the_view() {
        // No line or word limits here: the view measures the stage and breaks between blocks.
        let mut blocks = vec![b("h2", "Many")];
        for i in 0..30 {
            blocks.push(b("li", &format!("item {i}")));
        }
        assert_eq!(slides(&blocks).len(), 1);
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


    /// `===` directly under a line of text would make that line a heading in Markdown;
    /// for slides it is a break, as on a line of its own (not inside code).
    #[test]
    fn a_line_of_equals_signs_is_a_slide_break() {
        let slides_of = |md: &str| tabs(&load(&crate::md::to_yjs(md)).unwrap())[0].slides.len();
        assert_eq!(slides_of("# A\n\nx\n\n===\n\ny\n"), 2);
        assert_eq!(slides_of("# A\n\nx\n===\ny\n"), 2);
        assert_eq!(slides_of("# A\n\n- x\n=====\n\ny\n"), 2);
        assert_eq!(slides_of("# A\n\n```\nx\n===\n```\n\ny\n"), 1);
        let b = &tabs(&load(&crate::md::to_yjs("# A\n\nx\n===\ny\n")).unwrap())[0].blocks;
        assert_eq!((b[1].t.as_str(), b[1].text.as_str()), ("p", "x"), "x stays a paragraph");
    }
}

