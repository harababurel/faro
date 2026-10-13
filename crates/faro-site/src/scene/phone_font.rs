//! Outlining the phone frames' text (M13 4.5b, fonts): an external SVG
//! loaded as an image is an isolated document — it can neither use the
//! page's webfont nor fetch or embed one of its own. So each frame ships
//! with its Latin text already converted to `<path>` outlines, drawn
//! privately with `skrifa` from the same `JetBrains Mono` subsets `super`
//! loads for the OG image, and only the glyphs those subsets lack (emoji)
//! stay as `<text>` — every browser draws those from its system emoji
//! font, at the original cell coordinates and with the page's font stack
//! as a stand-in.
//!
//! Each glyph is drawn once, unhinted, at the origin with the baseline at
//! y = 0, deduplicated into a `<defs>` block, and placed with `<use x y>`
//! at exactly the cell position `faro_web::svg` gave it: the tspan's `x`
//! plus one cell (`metrics::ADVANCE × font-size + letter-spacing`) per
//! display column, and the row's baseline `y`. Terminal layout is kept
//! cell for cell — the font's own advances never enter the calculation —
//! and a frame's ~1500 characters collapse to ~100 unique outlines, which
//! is what keeps the files inside the §6.6 budget (a whole-document
//! `usvg` conversion bakes every glyph in place and triples the size).
//!
//! Text is handled by grapheme cluster (`unicode-segmentation`, UAX #29),
//! not code point: multi-code-point emoji (skin tones, ZWJ sequences,
//! regional-indicator flags, keycaps, variation selectors), Indic
//! conjuncts and bases with combining marks stay whole — only a single
//! code point a face covers is outlined, and consecutive unsupported
//! clusters are kept as one text run, advancing by the whole run's
//! display width so the cells after it land exactly where `faro_web::svg`
//! put them. Bold and italic runs use the bundled Bold/Italic faces; a run
//! that is *both* draws with the Bold face, and `text-decoration`
//! survives on kept runs but isn't drawn for outlined ones — the phone
//! cast uses neither, so these are documented fallbacks, not a claim
//! that every modifier combination is covered. The root tag and `<title>`
//! are the exporter's own, untouched, so ids, roles and accessible names
//! survive. The desktop poster and the OG image never pass through here.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use faro_web::metrics;
use skrifa::GlyphId;
use skrifa::charmap::Charmap;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::pen::SvgPen;
use skrifa::outline::{DrawSettings, OutlineGlyphCollection};
use skrifa::raw::FontRef;
use unicode_segmentation::UnicodeSegmentation;

use super::{FONT_BOLD, FONT_ITALIC, FONT_REGULAR, num};

/// One bundled face at the frame's size. The charmap and outline
/// collection are made per use: they borrow the font, which the frame's
/// static bytes outlive anyway.
#[derive(Clone)]
struct Face {
    font: FontRef<'static>,
    size: Size,
}

/// Which face a tspan asks for.
const REGULAR: usize = 0;
const BOLD: usize = 1;
const ITALIC: usize = 2;

/// `svg` (a whole frame from `faro_web::svg`) with its supported text
/// outlined through deduplicated glyph defs, and its unsupported runs
/// (emoji) kept as text at the end. `prefix` names this frame's glyph ids
/// (`{prefix}-{n}`) so ids are unique per asset even if several end up
/// inlined in one document.
pub(super) fn outlined(svg: &str, prefix: &str) -> Result<String, String> {
    let root_end = svg.find('>').ok_or("the frame has no root tag")?;
    let (root, rest) = svg.split_at(root_end + 1);
    let body = rest.strip_suffix("</svg>").ok_or("the frame is unclosed")?;
    let size = Size::new(attr(body, "font-size").unwrap_or(16.0));
    let faces = faces(size)?;
    let advance = cell_advance(body);
    let mut glyphs: BTreeMap<(usize, u32), String> = BTreeMap::new();
    let mut ids: BTreeMap<(usize, u32), usize> = BTreeMap::new();
    let (kept, body) = split_text(body, &faces, advance, prefix, &mut glyphs, &mut ids)?;
    let mut defs = String::new();
    for (&key, &id) in &ids {
        if let Some(d) = glyphs.get(&key) {
            let _ = write!(defs, r#"<path id="{prefix}-{id}" d="{d}"/>"#);
        }
    }
    let defs = if defs.is_empty() {
        defs
    } else {
        format!("<defs>{defs}</defs>")
    };
    Ok(format!("{root}{body}{defs}{kept}</svg>"))
}

/// The three bundled faces at the frame's size, from static font bytes.
fn faces(size: Size) -> Result<Vec<Face>, String> {
    [FONT_REGULAR, FONT_BOLD, FONT_ITALIC]
        .map(|bytes| {
            FontRef::new(bytes)
                .map(|font| Face { font, size })
                .map_err(|_| "the bundled font won't load for outlining".to_owned())
        })
        .into_iter()
        .collect()
}

/// The width one display cell adds to a tspan's `x`: the font's advance at
/// the frame's size plus the text group's `letter-spacing`, exactly what
/// `faro_web::metrics::letter_spacing` made up.
fn cell_advance(body: &str) -> f32 {
    let size = attr(body, "font-size").unwrap_or(16.0);
    let spacing = attr(body, "letter-spacing").unwrap_or(0.0);
    let advance = f64::from(metrics::ADVANCE) * f64::from(size) + f64::from(spacing);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "cell widths are small; f32 is exact enough"
    )]
    let advance = advance as f32;
    advance
}

/// The `name="value"` number in `text`, if any.
fn attr(text: &str, name: &str) -> Option<f32> {
    let open = format!(r#"{name}=""#);
    let start = text.find(&open)? + open.len();
    let end = text[start..].find('"')? + start;
    text[start..end].parse().ok()
}

/// The raw `name="value"` string in `text`, if any.
fn attr_raw<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let open = format!(r#"{name}=""#);
    let start = text.find(&open)? + open.len();
    let end = text[start..].find('"')? + start;
    Some(&text[start..end])
}

/// Replace every `<text>` block in `body` with `<use>` references to the
/// deduplicated glyph defs; runs the faces can't draw (emoji) come back
/// in one kept group. Returns `(kept, body with the text blocks replaced)`.
fn split_text(
    body: &str,
    faces: &[Face],
    advance: f32,
    prefix: &str,
    glyphs: &mut BTreeMap<(usize, u32), String>,
    ids: &mut BTreeMap<(usize, u32), usize>,
) -> Result<(String, String), String> {
    let group = group_attrs(body)?;
    let mut main = String::new();
    let mut kept_rows: Vec<(String, String)> = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("<text ") {
        let end = rest[start..]
            .find("</text>")
            .ok_or("a text block is unclosed")?
            + start
            + "</text>".len();
        main.push_str(&rest[..start]);
        rewrite_block(
            &rest[start..end],
            faces,
            advance,
            prefix,
            glyphs,
            ids,
            &mut main,
            &mut kept_rows,
        )?;
        rest = &rest[end..];
    }
    main.push_str(rest);
    let kept = if kept_rows.is_empty() {
        String::new()
    } else {
        let mut rows = String::new();
        for (y, tspans) in &kept_rows {
            let _ = write!(
                rows,
                r#"<text y="{y}" xml:space="preserve">{tspans}</text>"#
            );
        }
        format!("<g {group}>{rows}</g>")
    };
    Ok((kept, main))
}

/// The text group's attributes (`font-family`, `font-size`,
/// `letter-spacing`), reused verbatim for the kept group.
fn group_attrs(body: &str) -> Result<String, String> {
    let start = body
        .find("<g font-family=")
        .ok_or("the frame has no text group")?;
    let gt = body[start..]
        .find('>')
        .ok_or("the text group tag is unclosed")?
        + start;
    Ok(body[start + "<g ".len()..gt].to_owned())
}

/// Rewrite one `<text y="…">…</text>` block: supported tspans become
/// `<use>` elements appended to `main`, unsupported ones go to
/// `kept_rows` (grouped by the block's `y`).
#[expect(
    clippy::too_many_arguments,
    reason = "the shared glyph table travels with its frame's prefix"
)]
fn rewrite_block(
    block: &str,
    faces: &[Face],
    advance: f32,
    prefix: &str,
    glyphs: &mut BTreeMap<(usize, u32), String>,
    ids: &mut BTreeMap<(usize, u32), usize>,
    main: &mut String,
    kept_rows: &mut Vec<(String, String)>,
) -> Result<(), String> {
    let open_end = block.find('>').ok_or("a text tag is unclosed")? + 1;
    let y = attr_raw(&block[..open_end], "y").ok_or("a text block has no y")?;
    let mut kept = String::new();
    let mut uses = String::new();
    let mut rest = &block[open_end..block.len() - "</text>".len()];
    while let Some(start) = rest.find("<tspan ") {
        let attr_end = rest[start..].find('>').ok_or("a tspan tag is unclosed")? + start;
        let close = rest[start..]
            .find("</tspan>")
            .ok_or("a tspan is unclosed")?
            + start
            + "</tspan>".len();
        let attrs = &rest[start + "<tspan ".len()..attr_end];
        let content = &rest[attr_end + 1..close - "</tspan>".len()];
        rewrite_tspan(
            attrs, content, y, faces, advance, prefix, glyphs, ids, &mut uses, &mut kept,
        )?;
        rest = &rest[close..];
    }
    if !kept.is_empty() {
        if let Some((_, row)) = kept_rows.iter_mut().find(|(row_y, _)| *row_y == y) {
            row.push_str(&kept);
        } else {
            kept_rows.push((y.to_owned(), kept));
        }
    }
    main.push_str(&uses);
    Ok(())
}

/// Outline one tspan's supported clusters into `uses` (one `<use>` per
/// glyph, deduplicated) and move the rest to `kept` as text, one tspan
/// per run of consecutive unsupported clusters.
#[expect(
    clippy::too_many_arguments,
    reason = "the shared glyph table travels with its frame's prefix"
)]
fn rewrite_tspan(
    attrs: &str,
    content: &str,
    y: &str,
    faces: &[Face],
    advance: f32,
    prefix: &str,
    glyphs: &mut BTreeMap<(usize, u32), String>,
    ids: &mut BTreeMap<(usize, u32), usize>,
    uses: &mut String,
    kept: &mut String,
) -> Result<(), String> {
    // A run that is both bold and italic draws with the Bold face: no
    // BoldItalic subset is bundled (documented fallback).
    let face_key = if attrs.contains(r#"font-weight="bold""#) {
        BOLD
    } else if attrs.contains(r#"font-style="italic""#) {
        ITALIC
    } else {
        REGULAR
    };
    let fill = attr_raw(attrs, "fill").unwrap_or("");
    let (x, tail) = split_x(attrs)?;
    let face = faces
        .get(face_key)
        .ok_or("a tspan asks for a face that isn't bundled")?;
    let charmap = Charmap::new(&face.font);
    let mut cells = 0;
    // Clusters, not code points: only a single code point the face covers
    // is outlined; everything else joins the current kept run, so an
    // emoji sequence or a combining mark is never stranded mid-cluster.
    let mut run: Option<(usize, usize, usize, String)> = None; // raw start, start cell, raw end, decoded
    for (cluster, start, end) in clusters(content) {
        let gid = (cluster.chars().count() == 1 && cluster != " ")
            .then(|| charmap.map(cluster.chars().next().unwrap_or('\u{0}')))
            .flatten()
            .filter(|gid| gid.to_u32() != 0);
        if let Some(gid) = gid {
            if let Some((rs, sc, re, dec)) = run.take() {
                flush_run(kept, content, &tail, x, advance, (rs, sc, re));
                cells += faro_app::ui::text::width(&dec);
            }
            let key = (face_key, gid.to_u32());
            let id = ids.get(&key).copied().unwrap_or_else(|| {
                let id = ids.len();
                ids.insert(key, id);
                glyphs.insert(key, outline(face, gid));
                id
            });
            let _ = write!(
                uses,
                r##"<use href="#{prefix}-{id}" x="{}" y="{y}" fill="{fill}"/>"##,
                cell_x(x, cells, advance)
            );
            cells += faro_app::ui::text::width(&cluster);
        } else {
            run = Some(match run {
                Some((rs, sc, _, mut dec)) => {
                    dec.push_str(&cluster);
                    (rs, sc, end, dec)
                }
                None => (start, cells, end, cluster),
            });
        }
    }
    if let Some((rs, sc, re, _)) = run {
        flush_run(kept, content, &tail, x, advance, (rs, sc, re));
    }
    Ok(())
}

/// End one kept run as one `<tspan>`: the raw slice — entities and all —
/// at the cell the run started on, with every attribute but `x` kept, so
/// a `text-decoration` survives on kept text.
fn flush_run(
    kept: &mut String,
    content: &str,
    tail: &str,
    x: f32,
    advance: f32,
    (start, start_cells, end): (usize, usize, usize),
) {
    let _ = write!(
        kept,
        r#"<tspan x="{}"{tail}>{}</tspan>"#,
        cell_x(x, start_cells, advance),
        &content[start..end]
    );
}

/// The tspan's `x` and its attributes without it, wherever `x` sits: the
/// exporter writes it first, but a reordered tag must still work and a
/// missing or malformed one must fail loudly, not default to 0.
fn split_x(attrs: &str) -> Result<(f32, String), String> {
    let open = r#"x=""#;
    let mut at = 0;
    let start = loop {
        let Some(pos) = attrs[at..].find(open).map(|p| p + at) else {
            return Err("a tspan has no x attribute".to_owned());
        };
        if pos == 0 || attrs[..pos].ends_with(' ') {
            break pos;
        }
        at = pos + open.len();
    };
    let value = attrs[start + open.len()..]
        .find('"')
        .ok_or("a tspan's x attribute is unclosed")?;
    let raw = &attrs[start + open.len()..start + open.len() + value];
    let x: f32 = raw
        .parse()
        .map_err(|_| format!(r#"a tspan's x="{raw}" isn't a number"#))?;
    let rest = format!(
        "{} {}",
        &attrs[..start],
        &attrs[start + open.len() + value + 1..]
    );
    let tail = rest.trim();
    Ok((
        x,
        if tail.is_empty() {
            String::new()
        } else {
            format!(" {tail}")
        },
    ))
}

/// The content's UAX #29 grapheme clusters over the *decoded* text, each
/// with the raw byte range it came from: an entity decodes to one
/// character, and the raw slice — `&amp;` and all — is what a kept run
/// writes back out, so no re-escaping is needed.
fn clusters(content: &str) -> Vec<(String, usize, usize)> {
    let mut decoded = String::new();
    let mut from: Vec<(usize, usize)> = Vec::new(); // each character's raw range
    let mut at = 0;
    while at < content.len() {
        let (ch, next) = next_char(content, at);
        decoded.push(ch);
        from.push((at, next));
        at = next;
    }
    let mut out = Vec::new();
    let mut ranges = from.iter();
    for g in UnicodeSegmentation::graphemes(decoded.as_str(), true) {
        let group: Vec<(usize, usize)> = ranges.by_ref().take(g.chars().count()).copied().collect();
        if let (Some(first), Some(last)) = (group.first(), group.last()) {
            out.push((g.to_owned(), first.0, last.1));
        }
    }
    out
}

/// One glyph's outline at the origin, baseline y = 0, two decimals.
fn outline(face: &Face, gid: GlyphId) -> String {
    let mut svg_pen = SvgPen::with_precision(2);
    let mut pen = FlipPen(&mut svg_pen);
    let outlines = OutlineGlyphCollection::new(&face.font);
    if let Some(glyph) = outlines.get(gid) {
        let _ = glyph.draw(
            DrawSettings::unhinted(face.size, LocationRef::new(&[])),
            &mut pen,
        );
    }
    svg_pen.to_string()
}

/// `skrifa` draws unhinted outlines in font space (y up); SVG counts y
/// down, so every point crosses the baseline on the way out.
struct FlipPen<'a>(&'a mut SvgPen);

impl skrifa::outline::pen::OutlinePen for FlipPen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, -y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, -y);
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to(cx0, -cy0, x, -y);
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.curve_to(cx0, -cy0, cx1, -cy1, x, -y);
    }

    fn close(&mut self) {
        self.0.close();
    }
}

/// The `x` of the cell `cells` columns after a run that starts at `x`.
#[expect(
    clippy::cast_precision_loss,
    reason = "cell counts are tiny; f32 is exact well past the frame width"
)]
fn cell_x(x: f32, cells: usize, advance: f32) -> String {
    num(x + cells as f32 * advance)
}

/// The character at `at` and where it ends, decoding the five entities
/// `faro_web::svg::escape` writes.
fn next_char(text: &str, at: usize) -> (char, usize) {
    for (entity, ch) in [
        ("&amp;", '&'),
        ("&lt;", '<'),
        ("&gt;", '>'),
        ("&quot;", '"'),
        ("&apos;", '\''),
    ] {
        if text[at..].starts_with(entity) {
            return (ch, at + entity.len());
        }
    }
    let ch = text[at..].chars().next().unwrap_or('\u{0}');
    (ch, at + ch.len_utf8())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = concat!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 40" width="100" height="40" role="img" aria-labelledby="poster-title-x">"##,
        r##"<title id="poster-title-x">a frame</title>"##,
        r##"<g font-family="&quot;JetBrains Mono&quot;, monospace" font-size="16" letter-spacing="0.4">"##,
        r##"<text y="15.2" xml:space="preserve"><tspan x="20" fill="#e6e6e6">Hi&amp;📄there</tspan>"##,
        r##"<tspan x="60" fill="#e6e6e6" font-style="italic">note</tspan></text>"##,
        r##"<text y="34.2" xml:space="preserve"><tspan x="0" fill="#f5b94a" font-weight="bold">📄</tspan></text>"##,
        "</g></svg>",
    );

    #[test]
    fn supported_text_becomes_paths_and_emoji_stays_text() {
        let out = outlined(SAMPLE, "x").expect("the sample outlines");
        let root_end = SAMPLE.find('>').expect("a root tag");
        assert!(
            out.starts_with(&SAMPLE[..=root_end]),
            "the root tag survives"
        );
        assert!(
            out.contains(r#"<title id="poster-title-x">a frame</title>"#),
            "the title survives"
        );
        assert!(out.contains("<defs>"), "glyph outlines are deduplicated");
        assert!(
            out.contains(r##"<use href="#x-"##),
            "glyphs are placed by use"
        );
        assert!(
            !out.contains(">Hi&amp;") && !out.contains(">there<") && !out.contains(">note<"),
            "no supported text is left as text an image-mode SVG can't render"
        );
        assert_eq!(out.matches(">📄<").count(), 2, "both emoji runs stay text");
        // The split 📄 follows "Hi&" (3 cells) at x 20: 20 + 3 × 10.
        assert!(
            out.contains(r##"<tspan x="50" fill="#e6e6e6">📄</tspan>"##),
            "kept runs keep their cells"
        );
        assert!(
            out.contains(r#"font-weight="bold">📄<"#),
            "kept runs keep their style"
        );
        // "there" starts after the emoji: cell 5 of the run, x = 20 + 50.
        assert!(
            out.contains(r##"<use href="#x-"##) && out.contains(r#"" x="70" y="15.2""#),
            "outlined segments keep their cells"
        );
        assert!(!out.contains("url("), "no font is fetched");
        assert!(!out.contains("@font-face"), "no font is embedded");
        assert!(out.ends_with("</svg>"));
    }

    #[test]
    fn outlines_are_deduplicated_and_prefixed() {
        let out = outlined(SAMPLE, "cd").expect("the sample outlines");
        // "note" repeats no glyph, "there" repeats 'e' and 'r': the defs
        // hold far fewer paths than there are uses.
        assert!(
            out.matches("<use ").count() > out.matches(r#"<path id="cd-"#).count(),
            "uses outnumber unique glyphs"
        );
        assert!(
            out.contains(r#"<path id="cd-"#),
            "ids carry the frame's prefix"
        );
    }

    #[test]
    fn outlining_is_deterministic() {
        assert_eq!(outlined(SAMPLE, "x"), outlined(SAMPLE, "x"));
    }

    /// A one-row frame around `tspan` for the cluster and attribute tests.
    fn one_row(tspan: &str) -> String {
        format!(
            concat!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 40">"#,
                r#"<title id="poster-title-c">c</title>"#,
                r#"<g font-family="mono" font-size="16" letter-spacing="0.4">"#,
                r#"<text y="15.2" xml:space="preserve">{}</text></g></svg>"#,
            ),
            tspan
        )
    }

    /// The cell x `outlined` should place the next glyph at after `text`.
    fn cells_after(text: &str) -> String {
        format!(r#"" x="{}" y="15.2""#, faro_app::ui::text::width(text) * 10)
    }

    #[test]
    fn multi_codepoint_emoji_stay_one_cluster() {
        let out = outlined(
            &one_row(r##"<tspan x="0" fill="#e6e6e6">a💃🏽🙅🏽‍♀️🇺🇸b</tspan>"##),
            "c",
        )
        .expect("outlines");
        assert!(
            out.contains(">💃🏽🙅🏽‍♀️🇺🇸</tspan>"),
            "the whole run stays one tspan: {out}"
        );
        assert_eq!(out.matches('\u{200d}').count(), 1, "the ZWJ isn't stranded");
        assert_eq!(
            out.matches('\u{fe0f}').count(),
            1,
            "the VS16 isn't stranded"
        );
        assert!(
            out.contains(&cells_after("a💃🏽🙅🏽‍♀️🇺🇸")),
            "b lands after the run's display width, not its code points: {out}"
        );
    }

    #[test]
    fn consecutive_unsupported_text_stays_one_run() {
        let out = outlined(
            &one_row(r##"<tspan x="0" fill="#e6e6e6">hi बिलकुल नहीं ok</tspan>"##),
            "c",
        )
        .expect("outlines");
        assert_eq!(
            out.matches("<tspan").count(),
            1,
            "the phrase (spaces and matras included) is one tspan: {out}"
        );
        assert!(out.contains("> बिलकुल नहीं </tspan>"), "{out}");
        assert!(
            out.contains(&cells_after("hi बिलकुल नहीं ")),
            "the o lands after the run's width: {out}"
        );
    }

    #[test]
    fn combining_marks_stay_with_their_base() {
        // e + U+0301, not a precomposed é: the point is that the mark
        // rides with its base instead of being outlined away.
        let tspan = "<tspan x=\"0\" fill=\"#e6e6e6\">e\u{301} ok</tspan>";
        let out = outlined(&one_row(tspan), "c").expect("outlines");
        assert!(
            out.contains(">e\u{301} </tspan>"),
            "the accented base is kept whole, not outlined bare: {out}"
        );
        assert_eq!(
            out.matches('\u{0301}').count(),
            1,
            "the mark isn't stranded"
        );
        assert!(
            out.contains(r#"" x="20" y="15.2""#),
            "the cluster takes one cell: {out}"
        );
    }

    #[test]
    fn attributes_may_come_in_any_order() {
        let out = outlined(
            &one_row(r##"<tspan fill="#e6e6e6" font-weight="bold" x="20">hi📄</tspan>"##),
            "c",
        )
        .expect("outlines");
        assert!(
            out.contains(r##"<use href="#c-0" x="20"##),
            "x is found wherever it sits: {out}"
        );
        assert!(
            out.contains(r##"<tspan x="40" fill="#e6e6e6" font-weight="bold">📄</tspan>"##),
            "the kept tspan keeps the other attrs: {out}"
        );
    }

    #[test]
    fn a_tspan_without_a_usable_x_fails() {
        let no_x = outlined(&one_row(r##"<tspan fill="#e6e6e6">hi</tspan>"##), "c");
        assert!(no_x.is_err(), "no x: an error, not a silent 0");
        let bad = outlined(
            &one_row(r##"<tspan x="abc" fill="#e6e6e6">hi</tspan>"##),
            "c",
        );
        assert!(bad.is_err(), "x isn't a number");
        let unclosed = outlined(&one_row(r##"<tspan x="20 fill="#e6e6e6">hi</tspan>"##), "c");
        assert!(unclosed.is_err(), "an unclosed x fails too");
    }

    #[test]
    fn keycaps_and_conjuncts_stay_one_cluster() {
        let out = outlined(
            &one_row(r##"<tspan x="0" fill="#e6e6e6">a1️⃣क्षb</tspan>"##),
            "c",
        )
        .expect("outlines");
        assert!(
            out.contains(">1️⃣क्ष</tspan>"),
            "the keycap and the conjunct stay whole, in one run: {out}"
        );
        assert!(
            out.contains(&cells_after("a1️⃣क्ष")),
            "b lands after the run's whole-string width: {out}"
        );
    }

    #[test]
    fn hangul_with_a_combining_mark_stays_one_cluster() {
        let tspan = "<tspan x=\"0\" fill=\"#e6e6e6\">한\u{301} ok</tspan>";
        let out = outlined(&one_row(tspan), "c").expect("outlines");
        assert!(
            out.contains(">한\u{301} </tspan>"),
            "the mark rides along: {out}"
        );
        assert_eq!(out.matches('\u{301}').count(), 1, "the mark isn't stranded");
        assert!(
            out.contains(&cells_after("한\u{301} ")),
            "the o lands after the run's width: {out}"
        );
    }

    #[test]
    fn decoration_survives_on_kept_runs() {
        let out = outlined(
            &one_row(r##"<tspan x="0" fill="#e6e6e6" text-decoration="underline">📄</tspan>"##),
            "c",
        )
        .expect("outlines");
        assert!(
            out.contains(r#"text-decoration="underline">📄</tspan>"#),
            "kept runs keep their decoration: {out}"
        );
    }

    #[test]
    fn bold_italic_falls_back_to_bold() {
        let both = outlined(
            &one_row(
                r##"<tspan x="0" fill="#e6e6e6" font-weight="bold" font-style="italic">hi</tspan>"##,
            ),
            "c",
        )
        .expect("outlines");
        let bold = outlined(
            &one_row(r##"<tspan x="0" fill="#e6e6e6" font-weight="bold">hi</tspan>"##),
            "c",
        )
        .expect("outlines");
        assert_eq!(both, bold, "no BoldItalic subset: bold wins, documented");
    }
}
