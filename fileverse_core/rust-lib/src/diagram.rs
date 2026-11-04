//! Diagrams in documents: a code block in the `mermaid` language (as dDocs stores and
//! draws them) is rendered here to SVG, without a browser, by mermaid-rs-renderer.
//! The view shows the SVG through Qt's SVG image plugin, which has two limits this
//! module works around: it ignores per-line `tspan` positions (each line becomes its
//! own `text`), and it cannot draw HTML labels (this renderer does not emit them).

use mermaid_rs_renderer::{render_with_options, RenderOptions, Theme};
use serde::Deserialize;

/// Longest diagram source rendered (a guard against pathological input).
pub const MAX_SOURCE: usize = 20_000;

/// The colours to draw with, as the view's theme names them (`#rrggbb`). Missing ones
/// fall back to the Logos dark theme.
#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct Colors {
    pub background: String,
    pub surface: String,
    pub text: String,
    pub text_secondary: String,
    pub border: String,
    pub primary: String,
    /// One font family the view has loaded (Qt's SVG renderer takes a single name, not
    /// a CSS fallback list).
    pub font: String,
}

fn pick(v: &str, fallback: &str) -> String {
    let ok = v.len() <= 9 && v.starts_with('#') && v[1..].chars().all(|c| c.is_ascii_hexdigit()) && matches!(v.len(), 4 | 7 | 9);
    // Qt writes colours with alpha as #aarrggbb: keep the colour, drop the alpha
    if ok && v.len() == 9 {
        return format!("#{}", &v[3..]);
    }
    if ok { v.to_string() } else { fallback.to_string() }
}

fn theme(c: &Colors) -> Theme {
    let bg = pick(&c.background, "#262626");
    let surface = pick(&c.surface, "#343434");
    let text = pick(&c.text, "#FFFFFF");
    let soft = pick(&c.text_secondary, "#A4A4A4");
    let border = pick(&c.border, "#434343");
    let primary = pick(&c.primary, "#ED7B58");
    let mut t = Theme::dark();
    let font = c.font.trim();
    let font_ok = !font.is_empty() && font.len() <= 64 && font.chars().all(|ch| ch.is_alphanumeric() || ch == ' ' || ch == '-' || ch == '_');
    t.font_family = if font_ok { font.to_string() } else { "sans-serif".into() };
    t.font_size = 15.0;
    t.background = bg.clone();
    t.primary_color = surface.clone();
    t.primary_text_color = text.clone();
    t.primary_border_color = primary.clone();
    t.line_color = soft.clone();
    t.secondary_color = border.clone();
    t.tertiary_color = bg.clone();
    t.edge_label_background = bg.clone();
    t.cluster_background = bg.clone();
    t.cluster_border = border.clone();
    t.sequence_actor_fill = surface.clone();
    t.sequence_actor_border = primary.clone();
    t.sequence_actor_line = border.clone();
    t.sequence_note_fill = surface;
    t.sequence_note_border = primary.clone();
    t.sequence_activation_fill = border.clone();
    t.sequence_activation_border = soft;
    t.text_color = text;
    t
}

/// Whether a code block's text is a Mermaid diagram (its first line names a diagram
/// type), for code that arrives without a language.
pub fn looks_like_mermaid(code: &str) -> bool {
    const KINDS: &[&str] = &[
        "flowchart", "graph", "sequenceDiagram", "classDiagram", "stateDiagram", "stateDiagram-v2", "erDiagram", "gantt", "pie",
        "journey", "mindmap", "timeline", "gitGraph", "quadrantChart", "requirementDiagram", "C4Context", "C4Container",
        "C4Component", "sankey-beta", "xychart-beta", "block-beta", "architecture-beta", "packet-beta", "kanban",
    ];
    let first = code.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with("%%")).unwrap_or("");
    let word = first.split_whitespace().next().unwrap_or("");
    KINDS.contains(&word)
}

/// A rendered diagram: SVG Qt can draw, and its size in SVG units.
#[derive(Debug, Clone, PartialEq)]
pub struct Svg {
    pub svg: String,
    pub width: f64,
    pub height: f64,
}

/// Render Mermaid source to SVG. An error says why (unsupported or malformed).
pub fn render(source: &str, colors: &Colors) -> Result<Svg, String> {
    let src = source.trim();
    if src.is_empty() {
        return Err("empty diagram".into());
    }
    if src.len() > MAX_SOURCE {
        return Err(format!("diagram too long (over {MAX_SOURCE} characters)"));
    }
    let opts = RenderOptions { theme: theme(colors), ..RenderOptions::default() };
    let owned = src.to_string();
    // the renderer is young: a panic in it must not take Basecamp's module down
    let svg = std::panic::catch_unwind(move || render_with_options(&owned, opts))
        .map_err(|_| "the diagram renderer failed on this diagram".to_string())?
        .map_err(|e| format!("not a diagram this renderer knows: {e}"))?;
    let svg = flatten_tspans(&svg);
    let (width, height) = size_of(&svg).ok_or("the renderer returned no size")?;
    Ok(Svg { svg, width, height })
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let k = format!(" {name}=\"");
    let s = tag.find(&k)? + k.len();
    let e = tag[s..].find('"')? + s;
    Some(&tag[s..e])
}

fn set_attr(tag: &str, name: &str, v: &str) -> String {
    let k = format!(" {name}=\"");
    match tag.find(&k) {
        Some(s0) => {
            let s = s0 + k.len();
            let e = tag[s..].find('"').map_or(tag.len(), |e| e + s);
            format!("{}{}{}", &tag[..s], v, &tag[e..])
        }
        None => tag.replacen("<text", &format!("<text {name}=\"{v}\""), 1),
    }
}

/// The SVG's width and height (from `viewBox`, else `width`/`height`).
fn size_of(svg: &str) -> Option<(f64, f64)> {
    let open = &svg[svg.find("<svg")?..];
    let open = &open[..open.find('>')?];
    if let Some(vb) = attr(open, "viewBox") {
        let n: Vec<f64> = vb.split([' ', ',']).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect();
        if n.len() == 4 && n[2] > 0.0 && n[3] > 0.0 {
            return Some((n[2], n[3]));
        }
    }
    let num = |v: &str| v.trim_end_matches("px").parse::<f64>().ok();
    Some((num(attr(open, "width")?)?, num(attr(open, "height")?)?))
}

/// Qt's SVG renderer ignores the `x`/`dy` of each `tspan`, so a wrapped label comes
/// out as one run-together line: give every line its own `text` at its own place.
pub fn flatten_tspans(svg: &str) -> String {
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    while let Some(i) = rest.find("<text") {
        out.push_str(&rest[..i]);
        let r = &rest[i..];
        let (Some(open_end), Some(close)) = (r.find('>'), r.find("</text>")) else {
            out.push_str(r);
            return out;
        };
        let open = &r[..open_end];
        let body = &r[open_end + 1..close];
        if open.ends_with('/') || !body.contains("<tspan") {
            out.push_str(&r[..close + 7]);
        } else {
            let mut y: f64 = attr(open, "y").and_then(|v| v.parse().ok()).unwrap_or(0.0);
            let mut b = body;
            while let Some(ts) = b.find("<tspan") {
                let Some(te) = b[ts..].find('>').map(|e| e + ts) else { break };
                let Some(ce) = b[te..].find("</tspan>").map(|e| e + te) else { break };
                let tag = &b[ts..te];
                y += attr(tag, "dy").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                let mut t = set_attr(open, "y", &format!("{y:.2}"));
                if let Some(x) = attr(tag, "x") {
                    t = set_attr(&t, "x", x);
                }
                out.push_str(&t);
                out.push('>');
                out.push_str(&b[te + 1..ce]);
                out.push_str("</text>");
                b = &b[ce + 8..];
            }
        }
        rest = &r[close + 7..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flowcharts_and_sequences_render_for_qt() {
        let flow = "flowchart LR\n  ui[fileverse_ui<br/>QML view] --> core[fileverse_core]\n  core --> ipfs[(ipfs_module)]\n";
        let d = render(flow, &Colors::default()).unwrap();
        assert!(d.width > 100.0 && d.height > 20.0, "{d:?}");
        assert!(!d.svg.contains("<tspan"), "lines are separate text elements");
        assert!(!d.svg.contains("foreignObject"));
        assert!(d.svg.contains(">QML view</text>") && d.svg.contains(">fileverse_ui</text>"), "{}", d.svg);
        assert!(d.svg.contains("#ED7B58"), "drawn in the theme's accent");
        assert!(d.svg.contains(r#"font-family="sans-serif""#), "one family Qt understands");
        let d = render(flow, &Colors { font: "Public Sans".into(), ..Colors::default() }).unwrap();
        assert!(d.svg.contains(r#"font-family="Public Sans""#));
        let d = render(flow, &Colors { font: "x\"/><script>".into(), ..Colors::default() }).unwrap();
        assert!(!d.svg.contains("<script>"));

        let seq = "sequenceDiagram\n  participant A as You\n  participant B as Core\n  A->>B: open(link)\n  B-->>A: document\n";
        let d = render(seq, &Colors { primary: "#00ff00".into(), ..Colors::default() }).unwrap();
        assert!(d.svg.contains(">open(link)</text>") && d.svg.contains("#00ff00"), "{}", d.svg);
    }

    #[test]
    fn bad_input_is_an_error_not_a_crash() {
        assert!(render("", &Colors::default()).is_err());
        assert!(render(&"a".repeat(MAX_SOURCE + 1), &Colors::default()).is_err());
        // whatever the renderer makes of nonsense, it must not panic through
        let _ = render("flowchart LR\n  A --> \n ((( [[ ", &Colors::default());
        let _ = render("not a diagram at all", &Colors::default());
    }

    #[test]
    fn wrapped_labels_become_separate_lines() {
        let svg = r##"<svg viewBox="0 0 10 10"><text x="5" y="10" fill="#fff"><tspan x="5" dy="0">a</tspan><tspan x="5" dy="21">b</tspan></text><text x="1" y="2">c</text></svg>"##;
        let f = flatten_tspans(svg);
        assert_eq!(f, r##"<svg viewBox="0 0 10 10"><text x="5" y="10.00" fill="#fff">a</text><text x="5" y="31.00" fill="#fff">b</text><text x="1" y="2">c</text></svg>"##);
        assert_eq!(size_of(&f), Some((10.0, 10.0)));
        assert_eq!(pick("#80ED7B58", "#000"), "#ED7B58");
        assert_eq!(pick("red;x", "#000"), "#000");
    }
}
