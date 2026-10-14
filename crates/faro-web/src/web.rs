//! What the page calls (wasm-bindgen).

use std::cell::RefCell;
use std::rc::Rc;

use faro_core::rt;
use tokio::sync::mpsc;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::demo::visitor_mock;
use crate::dom::{self, CanvasPage, Listeners};
use crate::live::{self, Shell};
use crate::media::Audio;
use crate::paint::Painter;
use crate::session::Session;

/// The demo running in a canvas. Dropping it (`demo.free()`) stops it. The
/// canvas sends `faro-bell` events when faro rings, `faro-exit` when it
/// quits (free it and start again), and `faro-settings` when the settings
/// its controls show change (its `detail` carries the values). The page
/// keeps its own title: faro's window title (`faro demo (N)`) is for
/// terminals, and a count rising in a visitor's tab is noise.
///
/// **Never call a `Demo` method from a synchronous handler of an event faro
/// itself dispatched** (`faro-settings`, `faro-exit`, canvas focus/blur):
/// faro dispatches those while the shell is borrowed, and a re-entrant call
/// panics on the borrow (D125). Read the event's `detail` instead, and
/// defer any `Demo` work to `requestAnimationFrame`/`setTimeout`. Ordinary
/// user-gesture handlers (clicks, keydown) are fine — no borrow is held
/// then.
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
    // The session starts without the keys (`configure_visitor`, D125); hand
    // them over only if the canvas already has them by the same predicate the
    // listeners use, so a genuinely focused canvas gains focus at start too.
    shell.focus(dom::focused(&canvas));
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

    /// Whether the start has settled: the opening chat is open with its
    /// history, or the visitor took over first. The page shows the canvas in
    /// the poster's place only then (D124).
    pub fn ready(&self) -> bool {
        self.shell.borrow().session.settled()
    }

    /// The theme faro draws with (the palette's preview, else the setting).
    pub fn theme(&self) -> String {
        self.shell.borrow().state().theme_name().to_owned()
    }

    /// Switch to the theme `name` without a notice, as the page does to
    /// follow its light/dark scheme; `false` if there's no such theme.
    pub fn set_theme(&self, name: &str) -> bool {
        self.shell.borrow_mut().set_theme(name)
    }

    /// Switch the theme the way the palette does — faro notices it and the
    /// choice lasts for the visit; `false` if there's no such theme. The
    /// page's theme chips (§4.5). Call from a click handler, never from an
    /// event faro itself dispatches (the shell is borrowed; D125).
    pub fn pick_theme(&self, name: &str) -> bool {
        self.shell.borrow_mut().pick_theme(name)
    }

    /// faro's reduce-motion setting, for the page's motion toggle (§4.6).
    pub fn reduce_motion(&self) -> bool {
        self.shell.borrow().reduce_motion()
    }

    /// Turn faro's motion off or on through faro's own settings path (§4.6).
    pub fn set_reduce_motion(&self, reduced: bool) {
        self.shell.borrow_mut().set_reduce_motion(reduced);
    }

    /// Open faro's palette at Themes (the "all…" chip, §4.5); no synthetic
    /// keys, so the keys, focus and any draft stay as they are (D125).
    pub fn open_themes(&self) {
        self.shell.borrow_mut().open_themes();
    }

    /// faro's configured theme, ignoring any palette preview: what the
    /// page's scheme follower compares against (D124) and what its chips
    /// bootstrap from (§4.5).
    pub fn configured_theme(&self) -> String {
        self.shell.borrow().configured_theme()
    }

    /// Turn faro's motion off or on quietly at start — no notice over the
    /// poster's picture and no visitor takeover (§4.6): for the system
    /// preference before the handover, not for a visitor's toggle (that's
    /// `set_reduce_motion`).
    pub fn configure_reduce_motion(&self, reduced: bool) {
        self.shell.borrow_mut().configure_reduce_motion(reduced);
    }

    /// Re-measure the font at `size` CSS px and redraw on the same grid
    /// (fullscreen or a resize, §4.4); `false` if the size isn't finite or
    /// is outside 13..=32 px, or the font couldn't be measured. The page
    /// picks the size (16 max normally, 24 fullscreen); faro never
    /// auto-fits, and the keys are untouched (a resize is not input, D125).
    pub fn set_font_size(&self, size: f64) -> bool {
        self.shell.borrow_mut().set_font_size(size)
    }

    /// A cell's CSS width and height as `[w, h]`, for the page's size
    /// checks (§4.4).
    pub fn cell_css(&self) -> js_sys::Array {
        let (width, height) = self.shell.borrow().cell_css();
        let array = js_sys::Array::new();
        array.push(&JsValue::from_f64(width));
        array.push(&JsValue::from_f64(height));
        array
    }

    /// Every theme as `[name, group]` pairs, for the page's theme menu
    /// (§4.5): faro's own, then OpenCode's dark and light ones.
    #[expect(
        clippy::unused_self,
        reason = "a method, so the page asks the demo it already holds"
    )]
    pub fn themes(&self) -> js_sys::Array {
        crate::themes::menu()
            .into_iter()
            .map(|(name, group)| {
                let pair = js_sys::Array::new();
                pair.push(&JsValue::from_str(&name));
                pair.push(&JsValue::from_str(group));
                JsValue::from(pair)
            })
            .collect()
    }

    /// Draw a whole frame from scratch, as after a resize or a theme switch:
    /// render, diff and paint every row (to measure); returns the
    /// milliseconds it took.
    pub fn frame(&self) -> f64 {
        let started = now_ms();
        self.shell.borrow_mut().full_frame();
        now_ms() - started
    }

    /// Paint every row again (to measure the renderer); returns the
    /// milliseconds it took.
    pub fn repaint(&self) -> f64 {
        let started = now_ms();
        self.shell.borrow_mut().repaint();
        now_ms() - started
    }

    /// Where faro is, for tests: whether it has the keys and a chat is open,
    /// plus the open chat's selection, playback and the notice.
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
            "focused={} open={} focus={:?} selected={selected:?} playing={} notice={notice:?}",
            state.env.focused,
            state.open.is_some(),
            state.focus,
            state.playing.is_some()
        )
    }

    /// How long the last frames took (JSON: median, 95th percentile and
    /// worst, in milliseconds, for the whole frame, rendering and painting).
    pub fn timings(&self) -> String {
        self.shell.borrow().timings.summary()
    }

    /// The screen as text, one line per row (for tests).
    pub fn text(&self) -> String {
        faro_app::ui::text::grid(self.shell.borrow().buffer())
    }
}

impl Drop for Demo {
    fn drop(&mut self) {
        // Stop any playing voice note first: nothing else will call `stop`
        // once the loop is gone, and `Audio::drop` closes the context.
        if let Ok(shell) = self.shell.try_borrow() {
            shell.session.router.page.stop();
        }
        self.listeners = None;
        self.task.abort();
    }
}

fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or(0.0, |p| p.now())
}
