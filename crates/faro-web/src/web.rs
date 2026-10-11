//! What the page calls (wasm-bindgen).

use std::cell::RefCell;
use std::rc::Rc;

use faro_core::rt;
use tokio::sync::mpsc;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::dom::{CanvasPage, Listeners};
use crate::live::{self, Shell};
use crate::media::Audio;
use crate::paint::Painter;
use crate::session::{Session, visitor_mock};

/// The demo running in a canvas. Dropping it (`demo.free()`) stops it. The
/// canvas sends `faro-bell` events when faro rings and `faro-exit` when it
/// quits (free it and start again); `document.title` follows faro's window
/// title.
#[wasm_bindgen]
pub struct Demo {
    shell: Rc<RefCell<Shell>>,
    listeners: Option<Listeners>,
    task: rt::Task,
}

/// Start the demo in `canvas`: a `columns × rows` grid in `font` at `size`
/// CSS pixels (the font must be loaded first). `0` for either: as many as
/// fit the canvas's parent, following its size.
#[wasm_bindgen]
pub async fn start(
    canvas: HtmlCanvasElement,
    font: String,
    size: f64,
    columns: u16,
    rows: u16,
) -> Result<Demo, JsValue> {
    let page = Rc::new(CanvasPage {
        canvas: canvas.clone(),
        audio: Audio::default(),
    });
    let (session, inbox) = Session::start(&visitor_mock(), page)
        .await
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    let painter = Painter::new(canvas.clone(), &font, size)?;
    let fixed = (columns > 0 && rows > 0).then_some((columns, rows));
    let mut shell = Shell::new(session, painter, font, size, fixed);
    let focused = canvas
        .owner_document()
        .and_then(|d| d.active_element())
        .is_some_and(|a| a == **canvas);
    if !focused {
        shell.unfocus();
    }
    let (page, page_rx) = mpsc::unbounded_channel();
    shell.set_page(page.clone());
    let shell = Rc::new(RefCell::new(shell));
    let listeners = Listeners::attach(&canvas, &page, Rc::downgrade(&shell))?;
    let task = rt::spawn(live::run(Rc::clone(&shell), page_rx, inbox));
    shell.borrow_mut().draw();
    Ok(Demo {
        shell,
        listeners: Some(listeners),
        task,
    })
}

#[wasm_bindgen]
impl Demo {
    /// Draw now; returns the milliseconds it took.
    pub fn draw(&self) -> f64 {
        let started = now_ms();
        self.shell.borrow_mut().draw();
        now_ms() - started
    }

    /// Paint every row again (to measure the renderer); returns the
    /// milliseconds it took.
    pub fn repaint(&self) -> f64 {
        let started = now_ms();
        self.shell.borrow_mut().repaint();
        now_ms() - started
    }

    /// Where faro is, for tests: focus, the open chat's selection, the
    /// notice.
    pub fn inspect(&self) -> String {
        let shell = self.shell.borrow();
        let state = shell.state();
        let selected = state
            .open
            .as_ref()
            .and_then(|open| open.selected)
            .map(|s| s.message.sent_at.as_millis());
        let notice = state.notice.as_ref().map(|n| n.text.as_str());
        format!(
            "focus={:?} selected={selected:?} playing={} notice={notice:?}",
            state.focus,
            state.playing.is_some()
        )
    }

    /// The screen as text, one line per row (for tests).
    pub fn text(&self) -> String {
        faro_app::ui::text::grid(self.shell.borrow().buffer())
    }
}

impl Drop for Demo {
    fn drop(&mut self) {
        self.listeners = None;
        self.task.abort();
    }
}

fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or(0.0, |p| p.now())
}
