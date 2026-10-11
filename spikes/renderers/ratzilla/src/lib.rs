//! M13 Phase 0 spike (throwaway): draw an exported faro frame through Ratzilla's
//! WebGL2 backend (beamterm), with a dynamic font atlas from the same webfont the
//! Canvas 2D prototype uses.

use std::cell::RefCell;
use std::num::NonZeroU16;

use ratatui::Terminal;
use ratatui::buffer::CellDiffOption;
use ratatui::style::{Color, Modifier};
use ratzilla::CellSized;
use ratzilla::backend::webgl2::{FontAtlasConfig, WebGl2Backend, WebGl2BackendOptions};
use serde_json::Value;
use wasm_bindgen::prelude::*;

thread_local! {
    static TERM: RefCell<Option<Terminal<WebGl2Backend>>> = const { RefCell::new(None) };
}

fn js(e: impl std::fmt::Debug) -> JsValue {
    JsValue::from_str(&format!("{e:?}"))
}

/// Create the backend sized for `cols` × `rows` cells of `font` at `size` CSS px.
#[wasm_bindgen]
pub fn start(font: &str, size: f32, cols: u32, rows: u32, dynamic: bool) -> Result<String, JsValue> {
    let dpr = web_sys::window().map_or(1.0, |w| w.device_pixel_ratio());
    let mut options = WebGl2BackendOptions::new()
        .grid_id("grid")
        .size((cols * 10, rows * 20));
    if dynamic {
        options = options.font_atlas_config(FontAtlasConfig::dynamic(
            &[font, "Noto Color Emoji", "monospace"],
            size,
        ));
    }
    let mut backend = WebGl2Backend::new_with_options(options).map_err(js)?;
    let (cw, ch) = backend.cell_size_px();
    let (w, h) = (
        (f64::from(cols) * f64::from(cw) / dpr).ceil() as u32,
        (f64::from(rows) * f64::from(ch) / dpr).ceil() as u32,
    );
    backend.set_size(w, h).map_err(js)?;
    let terminal = Terminal::new(backend).map_err(js)?;
    let area = terminal.size().map_err(js)?;
    TERM.with(|t| *t.borrow_mut() = Some(terminal));
    Ok(format!(
        "cell {cw}x{ch}px dpr {dpr} canvas {w}x{h} grid {}x{}",
        area.width, area.height
    ))
}

fn color(v: &Value, default: Color) -> Color {
    let Some(s) = v.as_str() else { return default };
    let hex = |i: usize| u8::from_str_radix(s.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0);
    if s.starts_with('#') && s.len() == 7 {
        Color::Rgb(hex(1), hex(3), hex(5))
    } else {
        default
    }
}

/// Draw one exported frame; returns the time `Terminal::draw` took, in ms.
#[wasm_bindgen]
pub fn draw(json: &str) -> Result<f64, JsValue> {
    let frame: Value = serde_json::from_str(json).map_err(js)?;
    let w = frame["w"].as_u64().unwrap_or(0) as usize;
    let dfg = color(&frame["theme"]["fg"], Color::White);
    let dbg = color(&frame["theme"]["bg"], Color::Black);
    let cells = frame["cells"].as_array().cloned().unwrap_or_default();
    let perf = web_sys::window().and_then(|w| w.performance());
    let t0 = perf.as_ref().map_or(0.0, web_sys::Performance::now);
    TERM.with(|t| {
        let mut t = t.borrow_mut();
        let term = t.as_mut().ok_or("not started")?;
        term.draw(|f| {
            let buf = f.buffer_mut();
            let bw = buf.area.width as usize;
            for (i, c) in cells.iter().enumerate() {
                let (x, y) = (i % w, i / w);
                if x >= bw || y >= buf.area.height as usize {
                    continue;
                }
                let cell = &mut buf.content[y * bw + x];
                cell.reset();
                cell.set_symbol(c[0].as_str().unwrap_or(" "));
                cell.fg = color(&c[2], dfg);
                cell.bg = color(&c[3], dbg);
                let flags = c[4].as_u64().unwrap_or(0);
                let mut m = Modifier::empty();
                for (bit, modifier) in [
                    (1, Modifier::BOLD),
                    (2, Modifier::DIM),
                    (4, Modifier::ITALIC),
                    (8, Modifier::UNDERLINED),
                    (16, Modifier::REVERSED),
                    (32, Modifier::CROSSED_OUT),
                ] {
                    if flags & bit != 0 {
                        m |= modifier;
                    }
                }
                cell.modifier = m;
                let width = c[1].as_u64().unwrap_or(1);
                if width > 1 {
                    cell.diff_option = CellDiffOption::ForcedWidth(
                        NonZeroU16::new(width as u16).unwrap_or(NonZeroU16::MIN),
                    );
                }
            }
        })
        .map_err(js)?;
        Ok::<(), JsValue>(())
    })?;
    Ok(perf.as_ref().map_or(0.0, web_sys::Performance::now) - t0)
}
