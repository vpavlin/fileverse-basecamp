//! Markdown -> a dDoc's Yjs state, in the ProseMirror shape dDocs uses (every
//! top-level block wrapped in a `dBlock`, marks as Yjs text attributes:
//! `bold: {}`, `link: {href}`, ...). What the editor saves is therefore read by
//! this viewer's own renderer, and by anything else that reads dDocs.

use std::collections::HashMap;
use std::sync::Arc;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use yrs::types::Attrs;

use crate::doc::ImageRef;
use yrs::{Any, Doc, ReadTxn, Text, Transact, Xml, XmlElementPrelim, XmlElementRef, XmlFragment, XmlTextPrelim, XmlTextRef};

#[derive(Default, Clone)]
struct Marks {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    link: Option<String>,
}

impl Marks {
    /// Every mark, set or explicitly null: a Yjs insert without attributes would
    /// inherit the formatting of the run before it.
    fn attrs(&self) -> Attrs {
        let mut a: Attrs = HashMap::new();
        let flag = |on: bool| if on { Any::Map(Arc::new(HashMap::new())) } else { Any::Null };
        a.insert("bold".into(), flag(self.bold));
        a.insert("italic".into(), flag(self.italic));
        a.insert("strike".into(), flag(self.strike));
        a.insert("code".into(), flag(self.code));
        a.insert(
            "link".into(),
            match &self.link {
                Some(h) => {
                    let mut m = HashMap::new();
                    m.insert("href".to_string(), Any::String(h.as_str().into()));
                    Any::Map(Arc::new(m))
                }
                None => Any::Null,
            },
        );
        a
    }
}

enum Node {
    Frag,
    El(XmlElementRef),
    /// A paragraph opened for inline content that Markdown left bare (tight list items).
    Implicit(XmlElementRef),
}

struct Builder<'a> {
    txn: yrs::TransactionMut<'a>,
    frag: yrs::XmlFragmentRef,
    stack: Vec<Node>,
    text: Option<XmlTextRef>,
    marks: Marks,
    code_lang: String,
    code_buf: Option<String>,
    in_table_head: bool,
    /// Where top-level blocks go: the fragment (each in a dBlock) or a column.
    root: Option<XmlElementRef>,
    /// An `fvimg:` image being read: its reference and alt text so far.
    image: Option<(ImageRef, String)>,
}

/// This app's Markdown dialect adds columns (dDocs' image beside text):
///
/// ```text
/// ::: columns align-right
/// left column
/// ::: column
/// right column
/// :::
/// ```
#[derive(Debug, PartialEq)]
pub enum Segment {
    Md(String),
    Columns { layout: String, cols: Vec<String> },
}

/// Split Markdown into plain parts and column blocks (top level, outside code fences).
pub fn segments(md: &str) -> Vec<Segment> {
    let mut out = vec![];
    let mut plain = String::new();
    let mut cols: Option<(String, Vec<String>)> = None;
    let mut fence = false;
    for line in md.split_inclusive('\n') {
        let t = line.trim();
        if t.starts_with("```") {
            fence = !fence;
        }
        if !fence {
            let word = t.strip_prefix(":::").map(|r| r.trim());
            match (word, cols.as_mut()) {
                (Some(w), None) if w == "columns" || w.starts_with("columns ") => {
                    if !plain.trim().is_empty() {
                        out.push(Segment::Md(std::mem::take(&mut plain)));
                    }
                    plain.clear();
                    let layout = w.strip_prefix("columns").unwrap_or("").trim();
                    cols = Some((if layout.is_empty() { "align-center".into() } else { layout.to_string() }, vec![String::new()]));
                    continue;
                }
                (Some("column"), Some((_, c))) => {
                    c.push(String::new());
                    continue;
                }
                (Some(""), Some(_)) => {
                    let (layout, c) = cols.take().unwrap();
                    out.push(Segment::Columns { layout, cols: c });
                    continue;
                }
                _ => {}
            }
        }
        // A line of only "===" is a slide break (fileverse-ddocs#550) wherever it is: not
        // the underline of a heading, so it gets blank lines around it.
        let brk;
        let line = if !fence && t.len() >= 3 && t.chars().all(|c| c == '=') {
            brk = format!("\n{t}\n\n");
            brk.as_str()
        } else {
            line
        };
        match cols.as_mut() {
            Some((_, c)) => c.last_mut().unwrap().push_str(line),
            None => plain.push_str(line),
        }
    }
    if let Some((layout, c)) = cols {
        out.push(Segment::Columns { layout, cols: c });
    }
    if !plain.trim().is_empty() {
        out.push(Segment::Md(plain));
    }
    out
}

impl<'a> Builder<'a> {
    fn close_implicit(&mut self) {
        while let Some(Node::Implicit(_)) = self.stack.last() {
            self.stack.pop();
            self.text = None;
        }
    }

    fn push_el(&mut self, tag: &str) -> XmlElementRef {
        self.close_implicit();
        self.text = None;
        let el = match self.stack.last() {
            Some(Node::El(p)) => p.push_back(&mut self.txn, XmlElementPrelim::empty(tag)),
            _ => self.top_level(tag),
        };
        self.stack.push(Node::El(el.clone()));
        el
    }

    /// A new top-level block: in a column as is, in the document wrapped in a dBlock.
    fn top_level(&mut self, tag: &str) -> XmlElementRef {
        match self.root.clone() {
            Some(col) => col.push_back(&mut self.txn, XmlElementPrelim::empty(tag)),
            None => {
                let d = self.frag.push_back(&mut self.txn, XmlElementPrelim::empty("dBlock"));
                d.push_back(&mut self.txn, XmlElementPrelim::empty(tag))
            }
        }
    }

    /// An image is a block of its own in dDocs (`resizableMedia`), placed after the
    /// paragraph it was written in.
    fn media(&mut self, r: &ImageRef, alt: &str) {
        self.close_implicit();
        self.text = None;
        let container = self.stack.iter().rev().find_map(|n| match n {
            Node::El(e) if !matches!(e.tag().as_ref(), "paragraph" | "heading") => Some(e.clone()),
            _ => None,
        });
        let el = match container {
            Some(c) => c.push_back(&mut self.txn, XmlElementPrelim::empty("resizableMedia")),
            None => self.top_level("resizableMedia"),
        };
        let mut attrs: Vec<(&str, String)> = vec![];
        if r.local.is_empty() {
            attrs.extend([("ipfsHash", r.cid.clone()), ("encryptionKey", r.key.clone()), ("nonce", r.nonce.clone()), ("authTag", r.tag.clone())]);
        } else {
            attrs.push(("localRef", r.local.clone()));
        }
        attrs.push(("mimeType", r.mime.clone()));
        attrs.push(("width", if r.width.is_empty() { "100%".into() } else { r.width.clone() }));
        attrs.push(("dataAlign", if r.align.is_empty() { "center".into() } else { r.align.clone() }));
        attrs.push(("media-type", "img".into()));
        attrs.push(("caption", alt.to_string()));
        for (k, v) in attrs {
            if !v.is_empty() {
                el.insert_attribute(&mut self.txn, k, v);
            }
        }
    }

    fn pop(&mut self) {
        self.close_implicit();
        self.text = None;
        self.stack.pop();
    }

    fn current(&self) -> Option<XmlElementRef> {
        match self.stack.last() {
            Some(Node::El(e)) | Some(Node::Implicit(e)) => Some(e.clone()),
            _ => None,
        }
    }

    /// Inline content needs a block to live in; a bare list item gets a paragraph.
    fn ensure_inline_parent(&mut self) {
        let need = match self.current() {
            None => true,
            Some(e) => matches!(e.tag().as_ref(), "listItem" | "taskItem" | "blockquote" | "tableCell" | "tableHeader"),
        };
        if need {
            let el = match self.stack.last() {
                Some(Node::El(p)) => p.push_back(&mut self.txn, XmlElementPrelim::empty("paragraph")),
                _ => self.top_level("paragraph"),
            };
            self.text = None;
            self.stack.push(Node::Implicit(el));
        }
    }

    fn insert(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        self.ensure_inline_parent();
        if self.text.is_none() {
            let p = self.current().unwrap();
            self.text = Some(p.push_back(&mut self.txn, XmlTextPrelim::new("")));
        }
        let t = self.text.clone().unwrap();
        let at = t.len(&self.txn);
        let a = self.marks.attrs();
        t.insert_with_attributes(&mut self.txn, at, s, a);
    }

    fn hard_break(&mut self) {
        self.ensure_inline_parent();
        let p = self.current().unwrap();
        p.push_back(&mut self.txn, XmlElementPrelim::empty("hardBreak"));
        self.text = None;
    }
}

fn level(h: HeadingLevel) -> u8 {
    match h {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Build a new dDoc (one tab, `default`) from Markdown. Returns the Yjs update.
pub fn to_yjs(markdown: &str) -> Vec<u8> {
    let doc = Doc::new();
    let frag = doc.get_or_insert_xml_fragment("default");
    {
        let txn = doc.transact_mut();
        let mut b = Builder {
            txn,
            frag: frag.clone(),
            stack: vec![Node::Frag],
            text: None,
            marks: Marks::default(),
            code_lang: String::new(),
            code_buf: None,
            in_table_head: false,
            root: None,
            image: None,
        };
        for seg in segments(markdown) {
            match seg {
                Segment::Md(m) => {
                    b.root = None;
                    b.feed(&m);
                }
                Segment::Columns { layout, cols } => {
                    b.root = None;
                    let d = frag.push_back(&mut b.txn, XmlElementPrelim::empty("dBlock"));
                    let c = d.push_back(&mut b.txn, XmlElementPrelim::empty("columns"));
                    c.insert_attribute(&mut b.txn, "layout", layout);
                    for col in cols {
                        let el = c.push_back(&mut b.txn, XmlElementPrelim::empty("column"));
                        b.root = Some(el);
                        b.feed(&col);
                    }
                    b.root = None;
                }
            }
        }
    }
    let txn = doc.transact();
    txn.encode_state_as_update_v1(&yrs::StateVector::default())
}

/// The parser's events with one text event per run of text (it may split a run, e.g.
/// at a "_" inside a web address).
pub(crate) fn merged_events(markdown: &str, opts: Options) -> Vec<Event<'_>> {
    let mut events: Vec<Event> = vec![];
    for ev in Parser::new_ext(markdown, opts) {
        match (events.last_mut(), ev) {
            (Some(Event::Text(prev)), Event::Text(t)) => *prev = format!("{prev}{t}").into(),
            (_, ev) => events.push(ev),
        }
    }
    events
}

/// Bare web addresses in a run of text, as byte ranges: `http://` or `https://` up to
/// whitespace, without trailing punctuation (a closing parenthesis stays when the
/// address opened one).
pub(crate) fn bare_urls(t: &str) -> Vec<(usize, usize)> {
    let mut out = vec![];
    let mut from = 0;
    while let Some(off) = ["https://", "http://"].iter().filter_map(|p| t[from..].find(p)).min() {
        let start = from + off;
        let ok_before = t[..start].chars().next_back().map_or(true, |c| !c.is_alphanumeric());
        let mut end = t[start..].find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '`')).map_or(t.len(), |e| start + e);
        loop {
            let last = t[start..end].chars().next_back();
            match last {
                Some('.' | ',' | ';' | ':' | '!' | '?' | '\'' | '*' | '_' | ']') => end -= 1,
                Some(')') if t[start..end].matches('(').count() < t[start..end].matches(')').count() => end -= 1,
                _ => break,
            }
        }
        let scheme = if t[start..].starts_with("https://") { 8 } else { 7 };
        if ok_before && end > start + scheme {
            out.push((start, end));
        }
        from = end.max(start + scheme);
    }
    out
}

impl Builder<'_> {
    /// Text with its bare web addresses as links (as Fileverse's editor makes them),
    /// unless it is already inside a link or code.
    fn insert_linked(&mut self, t: &str) {
        if self.marks.link.is_some() || self.marks.code {
            return self.insert(t);
        }
        let mut at = 0;
        for (s, e) in bare_urls(t) {
            if s > at {
                self.insert(&t[at..s]);
            }
            self.marks.link = Some(t[s..e].to_string());
            self.insert(&t[s..e]);
            self.marks.link = None;
            at = e;
        }
        if at < t.len() {
            self.insert(&t[at..]);
        }
    }

    /// Add one run of Markdown at the builder's root.
    fn feed(&mut self, markdown: &str) {
        let b = self;
        b.stack = vec![Node::Frag];
        b.text = None;
        b.marks = Marks::default();
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let events = merged_events(markdown, opts);
    // A list is a task list when its first item starts with a task marker.
    let is_task_list = |i: usize| -> bool {
        matches!(events.get(i + 1), Some(Event::Start(Tag::Item)))
            && matches!(
                events.get(i + 2),
                Some(Event::TaskListMarker(_))
            ) || (matches!(events.get(i + 2), Some(Event::Start(Tag::Paragraph)))
                && matches!(events.get(i + 3), Some(Event::TaskListMarker(_))))
    };
    let mut task_lists: Vec<bool> = vec![];
    for (i, ev) in events.iter().cloned().enumerate() {
        if let Some(buf) = b.code_buf.as_mut() {
            match ev {
                Event::Text(t) => {
                    buf.push_str(&t);
                    continue;
                }
                Event::End(TagEnd::CodeBlock) => {}
                _ => continue,
            }
        }
        match ev {
            Event::Start(Tag::Paragraph) => {
                b.push_el("paragraph");
            }
            Event::End(TagEnd::Paragraph) => b.pop(),
            Event::Start(Tag::Heading { level: l, .. }) => {
                let el = b.push_el("heading");
                el.insert_attribute(&mut b.txn, "level", level(l).to_string());
            }
            Event::End(TagEnd::Heading(_)) => b.pop(),
            Event::Start(Tag::BlockQuote(_)) => {
                b.push_el("blockquote");
            }
            Event::End(TagEnd::BlockQuote(_)) => b.pop(),
            Event::Start(Tag::List(start)) => {
                let task = start.is_none() && is_task_list(i);
                task_lists.push(task);
                let tag = if task { "taskList" } else if start.is_some() { "orderedList" } else { "bulletList" };
                let el = b.push_el(tag);
                if let Some(n) = start {
                    el.insert_attribute(&mut b.txn, "start", n.to_string());
                }
            }
            Event::End(TagEnd::List(_)) => {
                task_lists.pop();
                b.pop();
            }
            Event::Start(Tag::Item) => {
                let task = task_lists.last().copied().unwrap_or(false);
                b.push_el(if task { "taskItem" } else { "listItem" });
            }
            Event::End(TagEnd::Item) => b.pop(),
            Event::TaskListMarker(done) => {
                // the marker follows the item (or its first paragraph) start
                for n in b.stack.iter().rev() {
                    if let Node::El(e) = n {
                        if e.tag().as_ref() == "taskItem" {
                            e.insert_attribute(&mut b.txn, "checked", done.to_string());
                            break;
                        }
                    }
                }
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                b.code_lang = match kind {
                    CodeBlockKind::Fenced(l) => l.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                b.code_buf = Some(String::new());
            }
            Event::End(TagEnd::CodeBlock) => {
                let code = b.code_buf.take().unwrap_or_default();
                let el = b.push_el("codeBlock");
                // a fence without a language that holds a Mermaid diagram is one (dDocs
                // draws a code block as a diagram only when its language says so)
                if b.code_lang.trim().is_empty() && crate::diagram::looks_like_mermaid(&code) {
                    b.code_lang = "mermaid".into();
                }
                if !b.code_lang.is_empty() {
                    el.insert_attribute(&mut b.txn, "language", b.code_lang.clone());
                }
                let t = el.push_back(&mut b.txn, XmlTextPrelim::new(""));
                t.insert(&mut b.txn, 0, code.trim_end_matches('\n'));
                b.pop();
            }
            Event::Rule => {
                b.push_el("horizontalRule");
                b.pop();
            }
            Event::Start(Tag::Table(_)) => {
                b.push_el("table");
            }
            Event::End(TagEnd::Table) => b.pop(),
            Event::Start(Tag::TableHead) => {
                b.in_table_head = true;
                b.push_el("tableRow");
            }
            Event::End(TagEnd::TableHead) => {
                b.in_table_head = false;
                b.pop();
            }
            Event::Start(Tag::TableRow) => {
                b.push_el("tableRow");
            }
            Event::End(TagEnd::TableRow) => b.pop(),
            Event::Start(Tag::TableCell) => {
                let tag = if b.in_table_head { "tableHeader" } else { "tableCell" };
                b.push_el(tag);
                b.push_el("paragraph");
            }
            Event::End(TagEnd::TableCell) => {
                b.pop();
                b.pop();
            }
            Event::Start(Tag::Strong) => b.marks.bold = true,
            Event::End(TagEnd::Strong) => b.marks.bold = false,
            Event::Start(Tag::Emphasis) => b.marks.italic = true,
            Event::End(TagEnd::Emphasis) => b.marks.italic = false,
            Event::Start(Tag::Strikethrough) => b.marks.strike = true,
            Event::End(TagEnd::Strikethrough) => b.marks.strike = false,
            Event::Start(Tag::Link { dest_url, .. }) => b.marks.link = Some(dest_url.to_string()),
            Event::End(TagEnd::Link) => b.marks.link = None,
            Event::Start(Tag::Image { dest_url, .. }) => match ImageRef::from_url(&dest_url) {
                Some(r) => b.image = Some((r, String::new())),
                // other images are not fetched: keep the reference as a link
                None => b.marks.link = Some(dest_url.to_string()),
            },
            Event::End(TagEnd::Image) => match b.image.take() {
                Some((r, alt)) => b.media(&r, &alt),
                None => b.marks.link = None,
            },
            Event::Code(c) => {
                b.marks.code = true;
                b.insert(&c);
                b.marks.code = false;
            }
            Event::Text(t) if b.image.is_some() => b.image.as_mut().unwrap().1.push_str(&t),
            Event::Text(t) => b.insert_linked(&t),
            Event::SoftBreak => b.insert(" "),
            Event::HardBreak => b.hard_break(),
            Event::Html(h) | Event::InlineHtml(h) => b.insert(&h),
            _ => {}
        }
    }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn bare_web_addresses_become_links() {
        let t = "See https://logos.co/a_b. Or (http://x.io/p(1)), not xhttps://no, https://";
        let found: Vec<&str> = bare_urls(t).into_iter().map(|(a, e)| &t[a..e]).collect();
        assert_eq!(found, ["https://logos.co/a_b", "http://x.io/p(1)"]);
        let doc = crate::doc::load(&to_yjs("Slides at https://logos.co/a_b and [a link](https://x.io), `https://code`\n")).unwrap();
        let html = &crate::doc::tabs(&doc)[0].blocks[0].html;
        assert!(html.contains(r#"<a href="https://logos.co/a_b">https://logos.co/a_b</a> and "#), "{html}");
        assert!(html.contains(r#"<a href="https://x.io">a link</a>"#), "{html}");
        assert!(!html.contains(r#"href="https://code""#), "{html}");
    }

    use super::*;
    use crate::doc;

    fn render(md: &str) -> Vec<crate::doc::Block> {
        let d = doc::load(&to_yjs(md)).unwrap();
        doc::tabs(&d).remove(0).blocks
    }

    #[test]
    fn column_blocks_are_split_out() {
        let md = "intro\n\n::: columns align-right\n\nleft\n\n::: column\n\nright\n\n:::\n\nafter\n\n```\n::: columns\n```\n";
        let s = segments(md);
        assert_eq!(s.len(), 3, "{s:?}");
        assert_eq!(s[1], Segment::Columns { layout: "align-right".into(), cols: vec!["\nleft\n\n".into(), "\nright\n\n".into()] });
        // inside a code fence it is just text
        assert!(matches!(&s[2], Segment::Md(m) if m.contains("::: columns")));
    }

    #[test]
    fn columns_and_images_survive_a_round_trip() {
        let img = ImageRef { cid: "bafkreidybxhs4ymbvt53hvxmz3f6nekg5gfit7c7pl7pul7fhfedsibm2a".into(), key: "a2V5".into(), nonce: "bm9uY2U".into(), tag: "dGFn".into(), mime: "image/png".into(), ..Default::default() };
        let local = ImageRef { local: "d1/abc.png".into(), mime: "image/png".into(), ..Default::default() };
        let md = format!("## Beside\n\n::: columns align-right\n\n- one\n- two\n\n::: column\n\n![a lambda]({})\n\n:::\n\n![mine]({})\n", img.to_url(), local.to_url());
        let d = doc::load(&to_yjs(&md)).unwrap();
        let tabs = doc::tabs(&d);
        let b = &tabs[0].blocks;
        let cols = b.iter().find(|x| x.t == "columns").expect("columns block");
        assert_eq!((cols.layout.as_str(), cols.cols.len()), ("align-right", 2));
        assert_eq!(cols.cols[0].iter().filter(|x| x.t == "li").count(), 2);
        let pic = cols.cols[1].iter().find(|x| x.t == "image").expect("image in the second column");
        assert_eq!(pic.img.as_ref().unwrap().cid, img.cid);
        assert_eq!(pic.text, "a lambda");
        let mine = b.iter().find(|x| x.t == "image").expect("a local image");
        assert_eq!(mine.img.as_ref().unwrap().local, "d1/abc.png");
        // and back to Markdown: same columns, same images
        let back = doc::markdown("", &tabs);
        assert!(back.contains("::: columns align-right") && back.contains("::: column"), "{back}");
        let refs: Vec<ImageRef> = back.split("](").skip(1).filter_map(|r| ImageRef::from_url(r.split(')').next().unwrap())).collect();
        assert_eq!(refs.iter().map(|r| (r.cid.as_str(), r.local.as_str())).collect::<Vec<_>>(), vec![(img.cid.as_str(), ""), ("", "d1/abc.png")]);
        assert!(!back.contains("\n\n\n\n"), "no piles of blank lines: {back:?}");
        let again = doc::tabs(&doc::load(&to_yjs(&back)).unwrap());
        assert_eq!(serde_json::to_string(&again[0].blocks).unwrap(), serde_json::to_string(&tabs[0].blocks).unwrap());
    }

    #[test]
    fn image_urls() {
        let r = ImageRef { cid: "Qm123".into(), key: "k".into(), nonce: "n".into(), tag: "t".into(), ..Default::default() };
        assert_eq!(ImageRef::from_url(&r.to_url()), Some(r));
        assert_eq!(ImageRef::from_url("fvimg:bm9wZQ"), None);
        assert_eq!(ImageRef::from_url("https://example.com/x.png"), None);
        // other images stay links and are not fetched
        let b = render("![x](https://example.com/x.png)\n");
        assert!(b.iter().all(|x| x.t != "image"));
    }

    #[test]
    fn round_trips_through_the_renderer() {
        let md = "# Title\n\nSome **bold**, _it_, `code`, ~~gone~~ and a [link](https://logos.co).\n\n## Part\n\n- one\n- two\n  - nested\n\n1. first\n2. second\n\n> quoted\n\n```rust\nfn x() {}\n```\n\n---\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n- [x] done\n- [ ] todo\n";
        let b = render(md);
        let kinds: Vec<&str> = b.iter().map(|x| x.t.as_str()).collect();
        assert_eq!(kinds, vec!["h1", "p", "h2", "li", "li", "li", "oli", "oli", "p", "code", "hr", "table", "task", "task"]);
        assert_eq!(b[1].html, "Some <b>bold</b>, <i>it</i>, <code>code</code>, <s>gone</s> and a <a href=\"https://logos.co\">link</a>.");
        assert_eq!(b[5].depth, 1);
        assert_eq!(b[7].num, Some(2));
        assert!(b[8].quote);
        assert_eq!(b[9].text, "fn x() {}");
        assert_eq!(b[9].lang, "rust");
        assert_eq!(b[11].rows, vec![vec!["a".to_string(), "b".into()], vec!["1".into(), "2".into()]]);
        assert_eq!(b[12].checked, Some(true));
        assert_eq!(b[13].checked, Some(false));
    }

    #[test]
    fn markup_in_the_source_stays_text() {
        let b = render("<img src=x onerror=alert(1)> hi");
        assert!(b[0].html.starts_with("&lt;img"), "{}", b[0].html);
    }

    #[test]
    fn markdown_out_then_in_is_stable() {
        let md = "## Heading\n\nText with **bold**.\n\n- a\n- b\n";
        let d1 = doc::load(&to_yjs(md)).unwrap();
        let out1 = doc::markdown("", &doc::tabs(&d1));
        let d2 = doc::load(&to_yjs(&out1)).unwrap();
        let out2 = doc::markdown("", &doc::tabs(&d2));
        assert_eq!(out1, out2);
    }
}
