//! The poster in the panel (WEBSITE.md §3.2, §6.4). Chunk 3 generates the
//! real frame as `site/generated/poster.svg`; until that exists this
//! stands in, drawn in `currentColor` so it fits both page schemes.

/// A small placeholder with the right shape for `.frame`: a `viewBox` in
/// faro's cell grid (~120×34, WEBSITE.md §4.4), a `<title>` for screen
/// readers, and one line of text.
pub const FALLBACK: &str = concat!(
    "<svg class=\"poster\" viewBox=\"0 0 120 34\" role=\"img\" ",
    "xmlns=\"http://www.w3.org/2000/svg\">",
    "<title>faro demo: poster coming soon</title>",
    "<text x=\"60\" y=\"17\" text-anchor=\"middle\" fill=\"currentColor\" ",
    "font-size=\"3\">faro demo: poster coming soon</text></svg>",
);
