//! What the page calls (wasm-bindgen).

use faro_app::app::image_cache::NoImages;
use faro_app::app::state::State;
use faro_app::theme::Theme;
use faro_app::ui::{self, View};
use ratatui::Terminal;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::backend::CanvasBackend;
use crate::demo::first_state;
use crate::paint::Painter;

/// The demo running in a canvas.
#[wasm_bindgen]
pub struct Demo {
    terminal: Terminal<CanvasBackend>,
    painter: Painter,
    state: State,
    theme: Theme,
}

/// Start the demo in `canvas`: a `columns × rows` grid in `font` at `size`
/// CSS pixels (the font must be loaded first). `0` for either: as many as fit
/// the canvas's parent.
#[wasm_bindgen]
pub async fn start(
    canvas: HtmlCanvasElement,
    font: String,
    size: f64,
    columns: u16,
    rows: u16,
) -> Result<Demo, JsValue> {
    let state = first_state()
        .await
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    let mut painter = Painter::new(canvas, &font, size)?;
    let theme = Theme::default();
    painter.set_defaults(theme.text, theme.background);
    let (columns, rows) = if columns == 0 || rows == 0 {
        let parent = painter.parent_size();
        painter.grid_for(parent.0, parent.1)
    } else {
        (columns, rows)
    };
    painter.resize(columns, rows);
    let mut backend = CanvasBackend::new(columns, rows);
    backend.set_cell_size(painter.cell_size());
    let Ok(terminal) = Terminal::new(backend);
    let mut demo = Demo {
        terminal,
        painter,
        state,
        theme,
    };
    demo.draw();
    Ok(demo)
}

#[wasm_bindgen]
impl Demo {
    /// Draw what changed; returns the milliseconds it took.
    pub fn draw(&mut self) -> f64 {
        let started = now_ms();
        let tz = jiff::tz::TimeZone::UTC;
        let view = View {
            theme: &self.theme,
            tz: &tz,
            now: jiff::Timestamp::now(),
            logo_time: None,
            images: &NoImages,
        };
        let state = &self.state;
        let Ok(_) = self.terminal.draw(|frame| {
            ui::render(frame, state, &view);
        });
        let backend = self.terminal.backend_mut();
        let rows = backend.take_dirty();
        let cursor = backend.cursor();
        self.painter.paint(backend.buffer(), &rows, cursor);
        now_ms() - started
    }

    /// Repaint every row (for measuring); returns the milliseconds it took.
    pub fn repaint(&mut self) -> f64 {
        let started = now_ms();
        let backend = self.terminal.backend_mut();
        backend.mark_all();
        let rows = backend.take_dirty();
        let cursor = backend.cursor();
        self.painter.paint(backend.buffer(), &rows, cursor);
        now_ms() - started
    }
}

fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or(0.0, |p| p.now())
}
