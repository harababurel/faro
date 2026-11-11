//! Occlusion-aware rendering for terminal image protocols when floating
//! overlays (`Ctrl+P`, menus, input completions) sit over part of the screen.

use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::Modifier;
use ratatui_image::sliced::{SignedPosition, SlicedProtocol};

use crate::app::image_cache::{subtract_rects, visible_rect};
use crate::theme::ColorDepth;

/// The Unicode character Kitty reserves for virtual image placements.
const KITTY_PLACEHOLDER: char = '\u{10EEEE}';

/// Draw `protocol` at `position` inside `clip_area`, leaving every cell inside
/// `occluders` untouched. Returns `false` only when `protocol` cannot clip
/// around a horizontal cut (mid-row Sixel or iTerm2 overlap).
pub(super) fn draw_sliced_occluded(
    protocol: &SlicedProtocol,
    position: SignedPosition,
    clip_area: Rect,
    occluders: &[Rect],
    buf: &mut Buffer,
    colors: ColorDepth,
) -> bool {
    let size = protocol.size();
    let left = u16::try_from(position.x.max(0)).unwrap_or(0);
    let Some(vis) = visible_rect(left, position.y, size.width, size.height, clip_area) else {
        return true;
    };
    let overlapping: Vec<Rect> = occluders
        .iter()
        .copied()
        .filter(|o| o.intersects(vis))
        .collect();
    if overlapping.is_empty() {
        super::render_sliced(protocol, position, clip_area, buf, colors);
        return true;
    }
    let uncovered = subtract_rects(vis, &overlapping);
    if uncovered.is_empty() {
        // Completely hidden under an overlay: skip rendering so a newly
        // decoded Kitty image preserves its one-time transmit sequence until
        // at least one cell is visible.
        return true;
    }

    let img_screen_x = i32::from(clip_area.x) + i32::from(position.x);
    let img_screen_y = i32::from(clip_area.y) + i32::from(position.y);
    let rel_pos = SignedPosition {
        x: i16::try_from(img_screen_x - i32::from(vis.x)).unwrap_or(0),
        y: i16::try_from(img_screen_y - i32::from(vis.y)).unwrap_or(0),
    };

    match protocol {
        SlicedProtocol::Kitty(_) => {
            let mut temp = Buffer::empty(vis);
            super::render_sliced(protocol, rel_pos, vis, &mut temp, colors);
            copy_kitty_unoccluded(&temp, vis, img_screen_x, &overlapping, buf);
            true
        }
        SlicedProtocol::Halfblocks(_) => {
            let mut temp = Buffer::empty(vis);
            copy_cells(buf, vis, &mut temp);
            super::render_sliced(protocol, rel_pos, vis, &mut temp, colors);
            for part in uncovered {
                copy_cells(&temp, part, buf);
            }
            true
        }
        SlicedProtocol::Sliced(_) => {
            // iTerm2 renders independent 1-row slices: whole uncovered rows
            // can still draw cleanly, while a mid-row cut falls back.
            let mid_row_cut = (vis.top()..vis.bottom()).any(|sy| {
                let any_hit = (vis.left()..vis.right()).any(|sx| {
                    overlapping
                        .iter()
                        .any(|o| o.contains(Position::new(sx, sy)))
                });
                let all_hit = (vis.left()..vis.right()).all(|sx| {
                    overlapping
                        .iter()
                        .any(|o| o.contains(Position::new(sx, sy)))
                });
                any_hit && !all_hit
            });
            if mid_row_cut {
                return false;
            }
            let mut temp = Buffer::empty(vis);
            copy_cells(buf, vis, &mut temp);
            super::render_sliced(protocol, rel_pos, vis, &mut temp, colors);
            for part in uncovered {
                copy_cells(&temp, part, buf);
            }
            true
        }
        SlicedProtocol::Sixel(_) => {
            // Sixel draws a single contiguous full-width band: if overlays
            // only cover top or bottom rows, render the remaining band.
            if let [band] = uncovered.as_slice()
                && band.width == vis.width
            {
                let band_pos = SignedPosition {
                    x: i16::try_from(img_screen_x - i32::from(band.x)).unwrap_or(0),
                    y: i16::try_from(img_screen_y - i32::from(band.y)).unwrap_or(0),
                };
                super::render_sliced(protocol, band_pos, *band, buf, colors);
                return true;
            }
            false
        }
    }
}

fn copy_cells(src: &Buffer, area: Rect, dst: &mut Buffer) {
    for sy in area.top()..area.bottom() {
        for sx in area.left()..area.right() {
            if let (Some(from), Some(to)) = (src.cell((sx, sy)), dst.cell_mut((sx, sy))) {
                *to = from.clone();
            }
        }
    }
}

/// Copy uncovered Kitty placeholder cells from `temp` (`vis`) into `buf`,
/// rescuing any one-time `\x1b_G…` transmit sequence from `(vis.x, vis.y)`
/// onto the first uncovered cell and re-anchoring `(row, col, id_msb)`
/// diacritics whenever an overlay cuts across the left of a run.
fn copy_kitty_unoccluded(
    temp: &Buffer,
    vis: Rect,
    img_screen_x: i32,
    overlapping: &[Rect],
    buf: &mut Buffer,
) {
    let mut upload_prefix: Option<&str> = temp.cell((vis.x, vis.y)).and_then(|cell| {
        let sym = cell.symbol();
        let idx = sym.find(KITTY_PLACEHOLDER)?;
        (idx > 0).then(|| sym.get(..idx)).flatten()
    });

    for sy in vis.top()..vis.bottom() {
        let Some(first_cell) = temp.cell((vis.x, sy)) else {
            continue;
        };
        let first_sym = first_cell.symbol();
        let Some(p_idx) = first_sym.find(KITTY_PLACEHOLDER) else {
            continue;
        };
        let Some(tail) = first_sym.get(p_idx + KITTY_PLACEHOLDER.len_utf8()..) else {
            continue;
        };
        let mut marks = tail.chars();
        let (Some(row_mark), Some(_col0_mark), Some(id_mark)) =
            (marks.next(), marks.next(), marks.next())
        else {
            continue;
        };

        let mut prev_drawn = false;
        for sx in vis.left()..vis.right() {
            let covered = overlapping
                .iter()
                .any(|o| o.contains(Position::new(sx, sy)));
            if covered {
                prev_drawn = false;
                continue;
            }
            let Some(src) = temp.cell((sx, sy)) else {
                prev_drawn = false;
                continue;
            };
            if !src.symbol().contains(KITTY_PLACEHOLDER) {
                prev_drawn = false;
                continue;
            }
            let Some(col_mark) = usize::try_from(i32::from(sx) - img_screen_x)
                .ok()
                .and_then(|col| KITTY_DIACRITICS.get(col).copied())
            else {
                prev_drawn = false;
                continue;
            };

            let mut out = String::new();
            if let Some(prefix) = upload_prefix.take() {
                out.push_str(prefix);
            }
            out.push(KITTY_PLACEHOLDER);
            if !prev_drawn {
                out.push(row_mark);
                out.push(col_mark);
                out.push(id_mark);
            }
            if let Some(dst) = buf.cell_mut((sx, sy)) {
                dst.modifier = Modifier::empty();
                dst.set_symbol(&out).set_fg(src.fg);
                dst.diff_option = src.diff_option;
            }
            prev_drawn = true;
        }
    }
}

/// Official Kitty Unicode-placeholder row/column combining diacritics table
/// (<https://sw.kovidgoyal.net/kitty/graphics-protocol/#unicode-placeholders>).
static KITTY_DIACRITICS: [char; 297] = [
    '\u{305}',
    '\u{30D}',
    '\u{30E}',
    '\u{310}',
    '\u{312}',
    '\u{33D}',
    '\u{33E}',
    '\u{33F}',
    '\u{346}',
    '\u{34A}',
    '\u{34B}',
    '\u{34C}',
    '\u{350}',
    '\u{351}',
    '\u{352}',
    '\u{357}',
    '\u{35B}',
    '\u{363}',
    '\u{364}',
    '\u{365}',
    '\u{366}',
    '\u{367}',
    '\u{368}',
    '\u{369}',
    '\u{36A}',
    '\u{36B}',
    '\u{36C}',
    '\u{36D}',
    '\u{36E}',
    '\u{36F}',
    '\u{483}',
    '\u{484}',
    '\u{485}',
    '\u{486}',
    '\u{487}',
    '\u{592}',
    '\u{593}',
    '\u{594}',
    '\u{595}',
    '\u{597}',
    '\u{598}',
    '\u{599}',
    '\u{59C}',
    '\u{59D}',
    '\u{59E}',
    '\u{59F}',
    '\u{5A0}',
    '\u{5A1}',
    '\u{5A8}',
    '\u{5A9}',
    '\u{5AB}',
    '\u{5AC}',
    '\u{5AF}',
    '\u{5C4}',
    '\u{610}',
    '\u{611}',
    '\u{612}',
    '\u{613}',
    '\u{614}',
    '\u{615}',
    '\u{616}',
    '\u{617}',
    '\u{657}',
    '\u{658}',
    '\u{659}',
    '\u{65A}',
    '\u{65B}',
    '\u{65D}',
    '\u{65E}',
    '\u{6D6}',
    '\u{6D7}',
    '\u{6D8}',
    '\u{6D9}',
    '\u{6DA}',
    '\u{6DB}',
    '\u{6DC}',
    '\u{6DF}',
    '\u{6E0}',
    '\u{6E1}',
    '\u{6E2}',
    '\u{6E4}',
    '\u{6E7}',
    '\u{6E8}',
    '\u{6EB}',
    '\u{6EC}',
    '\u{730}',
    '\u{732}',
    '\u{733}',
    '\u{735}',
    '\u{736}',
    '\u{73A}',
    '\u{73D}',
    '\u{73F}',
    '\u{740}',
    '\u{741}',
    '\u{743}',
    '\u{745}',
    '\u{747}',
    '\u{749}',
    '\u{74A}',
    '\u{7EB}',
    '\u{7EC}',
    '\u{7ED}',
    '\u{7EE}',
    '\u{7EF}',
    '\u{7F0}',
    '\u{7F1}',
    '\u{7F3}',
    '\u{816}',
    '\u{817}',
    '\u{818}',
    '\u{819}',
    '\u{81B}',
    '\u{81C}',
    '\u{81D}',
    '\u{81E}',
    '\u{81F}',
    '\u{820}',
    '\u{821}',
    '\u{822}',
    '\u{823}',
    '\u{825}',
    '\u{826}',
    '\u{827}',
    '\u{829}',
    '\u{82A}',
    '\u{82B}',
    '\u{82C}',
    '\u{82D}',
    '\u{951}',
    '\u{953}',
    '\u{954}',
    '\u{F82}',
    '\u{F83}',
    '\u{F86}',
    '\u{F87}',
    '\u{135D}',
    '\u{135E}',
    '\u{135F}',
    '\u{17DD}',
    '\u{193A}',
    '\u{1A17}',
    '\u{1A75}',
    '\u{1A76}',
    '\u{1A77}',
    '\u{1A78}',
    '\u{1A79}',
    '\u{1A7A}',
    '\u{1A7B}',
    '\u{1A7C}',
    '\u{1B6B}',
    '\u{1B6D}',
    '\u{1B6E}',
    '\u{1B6F}',
    '\u{1B70}',
    '\u{1B71}',
    '\u{1B72}',
    '\u{1B73}',
    '\u{1CD0}',
    '\u{1CD1}',
    '\u{1CD2}',
    '\u{1CDA}',
    '\u{1CDB}',
    '\u{1CE0}',
    '\u{1DC0}',
    '\u{1DC1}',
    '\u{1DC3}',
    '\u{1DC4}',
    '\u{1DC5}',
    '\u{1DC6}',
    '\u{1DC7}',
    '\u{1DC8}',
    '\u{1DC9}',
    '\u{1DCB}',
    '\u{1DCC}',
    '\u{1DD1}',
    '\u{1DD2}',
    '\u{1DD3}',
    '\u{1DD4}',
    '\u{1DD5}',
    '\u{1DD6}',
    '\u{1DD7}',
    '\u{1DD8}',
    '\u{1DD9}',
    '\u{1DDA}',
    '\u{1DDB}',
    '\u{1DDC}',
    '\u{1DDD}',
    '\u{1DDE}',
    '\u{1DDF}',
    '\u{1DE0}',
    '\u{1DE1}',
    '\u{1DE2}',
    '\u{1DE3}',
    '\u{1DE4}',
    '\u{1DE5}',
    '\u{1DE6}',
    '\u{1DFE}',
    '\u{20D0}',
    '\u{20D1}',
    '\u{20D4}',
    '\u{20D5}',
    '\u{20D6}',
    '\u{20D7}',
    '\u{20DB}',
    '\u{20DC}',
    '\u{20E1}',
    '\u{20E7}',
    '\u{20E9}',
    '\u{20F0}',
    '\u{2CEF}',
    '\u{2CF0}',
    '\u{2CF1}',
    '\u{2DE0}',
    '\u{2DE1}',
    '\u{2DE2}',
    '\u{2DE3}',
    '\u{2DE4}',
    '\u{2DE5}',
    '\u{2DE6}',
    '\u{2DE7}',
    '\u{2DE8}',
    '\u{2DE9}',
    '\u{2DEA}',
    '\u{2DEB}',
    '\u{2DEC}',
    '\u{2DED}',
    '\u{2DEE}',
    '\u{2DEF}',
    '\u{2DF0}',
    '\u{2DF1}',
    '\u{2DF2}',
    '\u{2DF3}',
    '\u{2DF4}',
    '\u{2DF5}',
    '\u{2DF6}',
    '\u{2DF7}',
    '\u{2DF8}',
    '\u{2DF9}',
    '\u{2DFA}',
    '\u{2DFB}',
    '\u{2DFC}',
    '\u{2DFD}',
    '\u{2DFE}',
    '\u{2DFF}',
    '\u{A66F}',
    '\u{A67C}',
    '\u{A67D}',
    '\u{A6F0}',
    '\u{A6F1}',
    '\u{A8E0}',
    '\u{A8E1}',
    '\u{A8E2}',
    '\u{A8E3}',
    '\u{A8E4}',
    '\u{A8E5}',
    '\u{A8E6}',
    '\u{A8E7}',
    '\u{A8E8}',
    '\u{A8E9}',
    '\u{A8EA}',
    '\u{A8EB}',
    '\u{A8EC}',
    '\u{A8ED}',
    '\u{A8EE}',
    '\u{A8EF}',
    '\u{A8F0}',
    '\u{A8F1}',
    '\u{AAB0}',
    '\u{AAB2}',
    '\u{AAB3}',
    '\u{AAB7}',
    '\u{AAB8}',
    '\u{AABE}',
    '\u{AABF}',
    '\u{AAC1}',
    '\u{FE20}',
    '\u{FE21}',
    '\u{FE22}',
    '\u{FE23}',
    '\u{FE24}',
    '\u{FE25}',
    '\u{FE26}',
    '\u{10A0F}',
    '\u{10A38}',
    '\u{1D185}',
    '\u{1D186}',
    '\u{1D187}',
    '\u{1D188}',
    '\u{1D189}',
    '\u{1D1AA}',
    '\u{1D1AB}',
    '\u{1D1AC}',
    '\u{1D1AD}',
    '\u{1D242}',
    '\u{1D243}',
    '\u{1D244}',
];

#[cfg(test)]
mod tests {
    use image::{DynamicImage, RgbaImage};
    use ratatui::layout::Size;
    use ratatui_image::{FontSize, Resize, picker::ProtocolType};

    use super::*;

    fn kitty_slice(width: u16, height: u16) -> SlicedProtocol {
        let mut picker = super::super::sized_picker(FontSize::new(10, 20));
        picker.set_protocol_type(ProtocolType::Kitty);
        let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            u32::from(width) * 10,
            u32::from(height) * 20,
            image::Rgba([80, 120, 200, 255]),
        ));
        SlicedProtocol::new_with_resize(&picker, img, Size::new(width, height), Resize::Fit(None))
            .unwrap()
    }

    #[test]
    fn full_occlusion_preserves_kitty_first_transmit_for_later_frame() {
        let slice = kitty_slice(4, 2);
        let area = Rect::new(0, 0, 10, 6);
        let mut hidden_buf = Buffer::empty(area);
        assert!(draw_sliced_occluded(
            &slice,
            SignedPosition { x: 2, y: 1 },
            area,
            &[Rect::new(0, 0, 10, 6)],
            &mut hidden_buf,
            ColorDepth::Truecolor,
        ));
        assert!(
            !hidden_buf[(2, 1)].symbol().contains(KITTY_PLACEHOLDER),
            "fully occluded frame leaves overlay cell untouched"
        );

        // Next frame without the overlay must still emit the one-time upload!
        let mut shown_buf = Buffer::empty(area);
        assert!(draw_sliced_occluded(
            &slice,
            SignedPosition { x: 2, y: 1 },
            area,
            &[],
            &mut shown_buf,
            ColorDepth::Truecolor,
        ));
        assert!(
            shown_buf[(2, 1)].symbol().contains("_G"),
            "one-time transmit sequence was preserved until visible"
        );
    }

    #[test]
    fn partial_kitty_occlusion_rescues_transmit_and_reanchors_columns() {
        let slice = kitty_slice(6, 2);
        let area = Rect::new(0, 0, 10, 4);
        let mut buf = Buffer::empty(area);
        for x in 0..10 {
            for y in 0..4 {
                buf[(x, y)].set_symbol("X");
            }
        }
        // Overlay covers the left 3 columns (x = 1..4) of a 6-col image at x = 1..7, y = 1..3.
        let overlay = Rect::new(1, 1, 3, 2);
        assert!(draw_sliced_occluded(
            &slice,
            SignedPosition { x: 1, y: 1 },
            area,
            &[overlay],
            &mut buf,
            ColorDepth::Truecolor,
        ));
        // Covered cells stay "X".
        for x in 1..4 {
            assert_eq!(buf[(x, 1)].symbol(), "X");
            assert_eq!(buf[(x, 2)].symbol(), "X");
        }
        // First uncovered cell (x = 4, image col = 3, row = 0) rescued the upload
        // AND carries explicit (row=0, col=3, id_msb) diacritics!
        let first_vis = buf[(4, 1)].symbol();
        assert!(
            first_vis.contains("_G"),
            "rescued transmit seq: {first_vis:?}"
        );
        let p_idx = first_vis.find(KITTY_PLACEHOLDER).unwrap();
        let marks: Vec<char> = first_vis[p_idx + KITTY_PLACEHOLDER.len_utf8()..]
            .chars()
            .collect();
        assert_eq!(marks[0], KITTY_DIACRITICS[0], "row 0 diacritic");
        assert_eq!(marks[1], KITTY_DIACRITICS[3], "col 3 diacritic");

        // Subsequent uncovered cell on the same row (x = 5) inherits position (no marks).
        assert_eq!(buf[(5, 1)].symbol(), KITTY_PLACEHOLDER.to_string());

        // Second row's first uncovered cell (x = 4, y = 2, image row = 1, col = 3)
        // also re-anchors at (row=1, col=3).
        let row1_vis = buf[(4, 2)].symbol();
        let r1_marks: Vec<char> = row1_vis[KITTY_PLACEHOLDER.len_utf8()..].chars().collect();
        assert_eq!(r1_marks[0], KITTY_DIACRITICS[1], "row 1 diacritic");
        assert_eq!(r1_marks[1], KITTY_DIACRITICS[3], "col 3 diacritic");
    }
}
