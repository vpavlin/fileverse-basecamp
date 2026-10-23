//! Markdown -> a dDoc's Yjs state, in the ProseMirror shape dDocs uses (every
//! top-level block wrapped in a `dBlock`, marks as Yjs text attributes:
//! `bold: {}`, `link: {href}`, ...). What the editor saves is therefore read by
//! this viewer's own renderer, and by anything else that reads dDocs.

use std::collections::HashMap;
use std::sync::Arc;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use yrs::types::Attrs;
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
            _ => {
                // top level: dDocs wraps every block in a dBlock
                let d = self.frag.push_back(&mut self.txn, XmlElementPrelim::empty("dBlock"));
                d.push_back(&mut self.txn, XmlElementPrelim::empty(tag))
            }
        };
        self.stack.push(Node::El(el.clone()));
        el
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
                _ => {
                    let d = self.frag.push_back(&mut self.txn, XmlElementPrelim::empty("dBlock"));
                    d.push_back(&mut self.txn, XmlElementPrelim::empty("paragraph"))
                }
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
            frag,
            stack: vec![Node::Frag],
            text: None,
            marks: Marks::default(),
            code_lang: String::new(),
            code_buf: None,
            in_table_head: false,
        };
        let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
        let events: Vec<Event> = Parser::new_ext(markdown, opts).collect();
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
                Event::Start(Tag::Image { dest_url, .. }) => {
                    // Images are not uploaded by this editor; keep the reference as a link.
                    b.marks.link = Some(dest_url.to_string());
                }
                Event::End(TagEnd::Image) => b.marks.link = None,
                Event::Code(c) => {
                    b.marks.code = true;
                    b.insert(&c);
                    b.marks.code = false;
                }
                Event::Text(t) => b.insert(&t),
                Event::SoftBreak => b.insert(" "),
                Event::HardBreak => b.hard_break(),
                Event::Html(h) | Event::InlineHtml(h) => b.insert(&h),
                _ => {}
            }
        }
    }
    let txn = doc.transact();
    txn.encode_state_as_update_v1(&yrs::StateVector::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc;

    fn render(md: &str) -> Vec<crate::doc::Block> {
        let d = doc::load(&to_yjs(md)).unwrap();
        doc::tabs(&d).remove(0).blocks
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
