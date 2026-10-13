//! The demo's event loop (D114): the browser's version of the terminal app's
//! `app::run`. One `select!` over the page's input, the backend's events,
//! the workers' results, the next animation frame, the reducer's deadline
//! and the logo's timer. Frames are drawn on `requestAnimationFrame`, and
//! only when something on screen changed.

use std::cell::RefCell;
use std::rc::Rc;

use faro_app::app::event::AppEvent;
use faro_app::app::image_cache::ImageKey;
use faro_app::app::state::{Geometry, State};
use faro_app::app::term_event::{Event, MouseEvent, MouseEventKind};
use faro_app::theme::{Theme, Themes};
use faro_app::ui::{self, View};
use faro_core::rt::{self, Instant};
use ratatui::Terminal;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;

use crate::backend::CanvasBackend;
use crate::images::{Halfblocks, Images};
use crate::input::{self, Held, Wheel};
use crate::media;
use crate::metrics;
use crate::paint::Painter;
use crate::session::{self, Inbox, Session};

/// What the page sends the loop.
#[derive(Debug)]
pub enum Input {
    /// Keys, paste and focus, ready for the reducer.
    Terminal(Event),
    /// A mouse button or move at a point, in CSS pixels from the canvas's
    /// top left.
    Pointer {
        action: Pointer,
        x: f64,
        y: f64,
        held: Held,
    },
    /// A `wheel` event's `deltaY` and `deltaMode`.
    Wheel {
        delta: f64,
        mode: u32,
        x: f64,
        y: f64,
        held: Held,
    },
    /// The canvas's parent changed size.
    Resized,
    /// An image was decoded for the cache (`None`: it couldn't be).
    Decoded(ImageKey, Option<Halfblocks>),
    /// A `Demo` API changed something while the loop was parked idle:
    /// wake it so it notices the dirty picture. Setting `dirty` alone
    /// can't — `run` blocks in `select!` and only asks for an animation
    /// frame at the top of each turn. This arm must never queue another
    /// `Redraw`, or the loop would spin.
    Redraw,
}

#[derive(Debug, Clone, Copy)]
pub enum Pointer {
    /// `MouseEvent.button`.
    Down(i16),
    Up(i16),
    /// `MouseEvent.buttons`.
    Move(u16),
}

/// What woke the loop.
enum Wake {
    Page(Input),
    App(AppEvent),
    Frame,
    Logo,
}

/// The session and what draws it.
pub struct Shell {
    pub session: Session,
    terminal: Terminal<CanvasBackend>,
    painter: Painter,
    family: String,
    size: f64,
    /// A grid fixed by the page (`?cols=&rows=`); otherwise it fills the
    /// canvas's parent.
    fixed: Option<(u16, u16)>,
    themes: Themes,
    tz: jiff::tz::TimeZone,
    dirty: bool,
    last_draw: Option<Instant>,
    logo_since: Option<Instant>,
    /// The page's own title, for when faro has none to show.
    page_title: String,
    title: Option<String>,
    wheel: Wheel,
    images: Images,
    /// How long recent frames took.
    pub timings: Timings,
    /// Where decoded images go (the loop's page input).
    page: Option<UnboundedSender<Input>>,
    /// The cell under the pointer, so moves within a cell aren't sent.
    pointer: Option<(u16, u16)>,
    /// The last settings snapshot told to the page (theme, configured
    /// theme, motion); a change sends a `faro-settings` event (§4.5, §4.6).
    settings_sent: Option<(String, String, bool)>,
}

impl Shell {
    pub fn new(
        session: Session,
        painter: Painter,
        family: String,
        size: f64,
        fixed: Option<(u16, u16)>,
    ) -> Self {
        let (columns, rows) = fixed.unwrap_or_else(|| fit(&painter));
        let mut backend = CanvasBackend::new(columns, rows);
        backend.set_cell_size(painter.cell_size());
        let Ok(terminal) = Terminal::new(backend);
        let mut shell = Self {
            session,
            terminal,
            painter,
            family,
            size,
            fixed,
            themes: Themes::default(),
            tz: local_time_zone(),
            dirty: true,
            last_draw: None,
            logo_since: None,
            page_title: document().map(|d| d.title()).unwrap_or_default(),
            title: None,
            wheel: Wheel::default(),
            images: Images::default(),
            timings: Timings::default(),
            page: None,
            pointer: None,
            settings_sent: None,
        };
        shell.painter.resize(columns, rows);
        shell.handle(AppEvent::Terminal(Event::Resize(columns, rows)));
        shell
    }

    pub fn set_page(&mut self, page: UnboundedSender<Input>) {
        self.page = Some(page);
    }

    /// The page hands faro the keys (`true`) or takes them back (`false`), as
    /// the browser's focus predicate says at start (D125). The session starts
    /// unfocused; the page reports its actual focus once the canvas is attached.
    pub fn focus(&mut self, focused: bool) {
        let event = if focused {
            Event::FocusGained
        } else {
            Event::FocusLost
        };
        self.handle(AppEvent::Terminal(event));
    }

    /// Draw in the theme `name` from now on, without a notice: the page's
    /// light/dark scheme, which the demo follows (D124). `false` if there's
    /// no such theme.
    pub fn set_theme(&mut self, name: &str) -> bool {
        let state = &mut self.session.state;
        if Theme::find(name, &state.user_themes).is_none() {
            return false;
        }
        name.clone_into(&mut state.config.theme);
        self.draw();
        true
    }

    /// The page's theme chip (§4.5): faro's own palette path, notice and all
    /// (the silent `set_theme` above stays for the page's scheme follower).
    /// `false` if there's no such theme.
    pub fn pick_theme(&mut self, name: &str) -> bool {
        let picked = self.session.pick_theme(name);
        if picked {
            self.request_frame();
        }
        picked
    }

    /// The page's motion toggle (§4.6), through faro's own setting.
    pub fn set_reduce_motion(&mut self, reduced: bool) {
        self.session.set_reduce_motion(reduced);
        self.request_frame();
    }

    /// The page's motion preference at start (§4.6): quiet — no notice over
    /// the poster's picture, and not a visitor takeover.
    pub fn configure_reduce_motion(&mut self, reduced: bool) {
        self.session.configure_reduce_motion(reduced);
        self.request_frame();
    }

    /// faro's `reduce_motion`, for the page's toggle.
    pub const fn reduce_motion(&self) -> bool {
        self.session.state.config.reduce_motion
    }

    /// faro's configured theme, ignoring any palette preview: what the
    /// page's scheme follower compares against (D124) and what its chips
    /// bootstrap from (§4.5).
    pub fn configured_theme(&self) -> String {
        self.session.state.config.theme.clone()
    }

    /// A cell's CSS size, for the page's own fit checks.
    pub fn cell_css(&self) -> (f64, f64) {
        self.painter.cell_css()
    }

    /// The "all…" chip: the palette at Themes, no synthetic keys (D125).
    pub fn open_themes(&mut self) {
        self.session.open_themes();
        self.request_frame();
    }

    /// Ask for a frame from outside the loop — a `Demo` API call the page
    /// made while faro was idle. Setting `dirty` alone isn't enough: `run`
    /// sits in `select!` with nothing scheduled and only asks for an
    /// animation frame at the top of each turn, so the `Redraw` input is
    /// what wakes it (and the loop recomputes the notice deadlines then).
    /// Callers the loop is already handling (terminal input, `Resized`)
    /// just set `dirty`.
    fn request_frame(&mut self) {
        self.dirty = true;
        if let Some(page) = &self.page {
            let _ = page.send(Input::Redraw);
        }
    }

    /// Whether faro takes the mouse (`[mouse] enabled`); if not, the wheel
    /// scrolls the page.
    pub const fn mouse(&self) -> bool {
        self.session.state.config.mouse.enabled
    }

    pub const fn state(&self) -> &State {
        &self.session.state
    }

    pub fn buffer(&self) -> &ratatui::buffer::Buffer {
        self.terminal.backend().buffer()
    }

    fn grid(&self) -> (u16, u16) {
        let area = self.buffer().area;
        (area.width, area.height)
    }

    fn handle(&mut self, event: AppEvent) {
        // The §6.9 `Demo message sent` signal: tell the page when the mock
        // accepted a message the visitor sent (the predicate ignores
        // everything else). The event carries no detail; the page counts
        // one per visit and must never call back (D125).
        let sent = session::accepted_send(&event);
        self.dirty |= self.session.handle(event);
        if sent {
            self.tell_sent();
        }
    }

    /// Tell the page a send landed (§6.9), as [`Self::exited`] does for a
    /// quit; a lost dispatch is harmless — the analytics are best-effort.
    fn tell_sent(&mut self) {
        let event = web_sys::CustomEvent::new(crate::dom::SENT_EVENT);
        if let Ok(event) = event
            && self.painter.canvas().dispatch_event(&event).is_err()
        {
            tracing::warn!("telling the page a send landed failed");
        }
    }

    fn take(&mut self, wake: Wake) {
        match wake {
            Wake::Page(input) => self.page(input),
            Wake::App(event) => self.handle(event),
            Wake::Frame => self.draw(),
            Wake::Logo => self.dirty = true,
        }
    }

    fn page(&mut self, input: Input) {
        match input {
            Input::Terminal(event) => self.handle(AppEvent::Terminal(event)),
            Input::Pointer { action, x, y, held } => self.pointer(action, x, y, held),
            Input::Wheel {
                delta,
                mode,
                x,
                y,
                held,
            } => {
                if !self.mouse() {
                    return;
                }
                let (column, row) = input::cell_at(x, y, self.painter.cell_css(), self.grid());
                let steps = self.wheel.feed(delta, mode, self.painter.cell_css().1);
                for _ in 0..steps.unsigned_abs() {
                    self.mouse_event(input::scroll(steps > 0), column, row, held);
                }
            }
            Input::Resized => self.resized(),
            Input::Decoded(key, image) => {
                self.images.insert(key, image);
                self.dirty = true;
            }
            // A `Demo` API woke us from outside the loop: the picture is
            // already dirty, this turn only asks for the frame. Never
            // re-queue — the top of the loop schedules the rAF.
            Input::Redraw => self.dirty = true,
        }
    }

    fn pointer(&mut self, action: Pointer, x: f64, y: f64, held: Held) {
        if !self.mouse() {
            return;
        }
        let (column, row) = input::cell_at(x, y, self.painter.cell_css(), self.grid());
        let moved = self.pointer != Some((column, row));
        self.pointer = Some((column, row));
        let kind = match action {
            Pointer::Down(button) => input::button(button).map(MouseEventKind::Down),
            Pointer::Up(button) => input::button(button).map(MouseEventKind::Up),
            Pointer::Move(buttons) => moved.then(|| {
                input::dragging(buttons).map_or(MouseEventKind::Moved, MouseEventKind::Drag)
            }),
        };
        if let Some(kind) = kind {
            self.mouse_event(kind, column, row, held);
        }
    }

    fn mouse_event(&mut self, kind: MouseEventKind, column: u16, row: u16, held: Held) {
        let mouse = MouseEvent {
            kind,
            column,
            row,
            modifiers: held.modifiers(),
        };
        self.handle(AppEvent::Terminal(Event::Mouse(mouse)));
    }

    /// The parent changed size (or the page was zoomed): measure the font
    /// again if the pixel ratio changed, and fit the grid.
    fn resized(&mut self) {
        let ratio = web_sys::window().map_or(1.0, |w| w.device_pixel_ratio());
        if (ratio - self.painter.ratio()).abs() > f64::EPSILON {
            self.remeasure();
        }
        self.refit();
    }

    /// Re-measure the font on the same canvas at `self.size`; the theme
    /// cache goes with it, its colours were picked against the old cell.
    fn remeasure(&mut self) -> bool {
        match Painter::new(self.painter.canvas().clone(), &self.family, self.size) {
            Ok(painter) => {
                self.painter = painter;
                self.themes = Themes::default();
                let cell = self.painter.cell_size();
                self.terminal.backend_mut().set_cell_size(cell);
                true
            }
            Err(error) => {
                tracing::warn!(?error, "measuring the font again failed");
                false
            }
        }
    }

    /// Fit the grid to the canvas's parent (or keep the page's fixed one)
    /// and make faro redraw every row. A resize is not input: the keys
    /// stay as they are (D125).
    fn refit(&mut self) {
        let (columns, rows) = self.fixed.unwrap_or_else(|| fit(&self.painter));
        self.painter.resize(columns, rows);
        self.terminal.backend_mut().resize(columns, rows);
        // The backend's cells are gone; ratatui must forget its last frame
        // too, or it only sends what changed and leaves holes.
        let Ok(()) = self.terminal.clear();
        self.handle(AppEvent::Terminal(Event::Resize(columns, rows)));
        self.dirty = true;
    }

    /// The page chose a new type size (fullscreen or a resize, §4.4):
    /// re-measure the font at `size` CSS px and redraw on the same grid —
    /// the demo's fixed 120×34 stays; faro never auto-fits. `false` if
    /// `size` isn't finite or is outside 13..=32 px
    /// ([`metrics::valid_font_size`]), or the font couldn't be measured
    /// again. The same size at the same pixels is a no-op.
    pub fn set_font_size(&mut self, size: f64) -> bool {
        if !metrics::valid_font_size(size) {
            return false;
        }
        let ratio = web_sys::window().map_or(1.0, |w| w.device_pixel_ratio());
        if (size - self.size).abs() <= f64::EPSILON
            && (ratio - self.painter.ratio()).abs() <= f64::EPSILON
        {
            return true;
        }
        // Only adopt the new size once the font has actually been measured
        // at it: on failure the size and the painter must stay a matching
        // pair, or a later same-size call would no-op onto the wrong cells.
        let previous = self.size;
        self.size = size;
        if !self.remeasure() {
            self.size = previous;
            return false;
        }
        self.refit();
        // Called from the page, not from the loop: wake it for the new
        // picture. (The same-size no-op above returns without waking.)
        self.request_frame();
        true
    }

    /// Draw a frame now and paint the rows that changed.
    pub fn draw(&mut self) {
        self.dirty = false;
        let state = &mut self.session.state;
        let logo_time = logo_time(&mut self.logo_since, state);
        if std::mem::take(&mut state.env.repaint) {
            let Ok(()) = self.terminal.clear();
        }
        self.images.prepare_frame(state);
        let theme = self
            .themes
            .get(state.theme_name(), state.colors(), &state.user_themes);
        if self.painter.set_defaults(theme.text, theme.background) {
            self.terminal.backend_mut().mark_all();
        }
        let view = View {
            theme,
            tz: &self.tz,
            now: jiff::Timestamp::now(),
            logo_time,
            images: &self.images,
        };
        let mut geometry = Geometry::default();
        let state: &State = state;
        let started = now_ms();
        let Ok(_) = self.terminal.draw(|frame| {
            geometry = ui::render(frame, state, &view);
        });
        let title = state.window_title(view.now_timestamp());
        let backend = self.terminal.backend_mut();
        let rows = backend.take_dirty();
        let cursor = backend.cursor();
        let rendered = now_ms();
        self.painter.paint(backend.buffer(), &rows, cursor);
        self.timings.record(Timing {
            render: rendered - started,
            paint: now_ms() - rendered,
            rows: rows.len(),
        });
        if title != self.title {
            if let Some(document) = document() {
                document.set_title(title.as_deref().unwrap_or(&self.page_title));
            }
            self.title = title;
        }
        self.last_draw = Some(Instant::now());
        self.decode(&geometry.images_wanted);
        // Same picture; only bookkeeping (and maybe loading older messages).
        self.handle(AppEvent::Drawn(geometry));
        self.sync_settings();
    }

    /// Tell the page its controls' picture changed (§4.5, §4.6): the theme
    /// faro draws with (palette preview included), the configured theme,
    /// and motion. The event's detail carries the values, so the page's
    /// listener never reaches back into the borrowed shell (D125).
    fn sync_settings(&mut self) {
        let state = &self.session.state;
        let snapshot = (
            state.theme_name().to_owned(),
            state.config.theme.clone(),
            state.config.reduce_motion,
        );
        if self.settings_sent.as_ref() == Some(&snapshot) {
            return;
        }
        let detail = js_sys::Object::new();
        let tell = |key: &str, value: JsValue| {
            let _ = js_sys::Reflect::set(&detail, &JsValue::from_str(key), &value);
        };
        tell("theme", snapshot.0.as_str().into());
        tell("configuredTheme", snapshot.1.as_str().into());
        tell("reduceMotion", snapshot.2.into());
        let init = web_sys::CustomEventInit::new();
        init.set_detail(&detail);
        match web_sys::CustomEvent::new_with_event_init_dict(crate::dom::SETTINGS_EVENT, &init) {
            Ok(event) => {
                if self.painter.canvas().dispatch_event(&event).is_err() {
                    tracing::warn!("telling the page the settings changed failed");
                    // Keep the old snapshot: the next draw retries.
                    return;
                }
            }
            Err(error) => {
                tracing::warn!(?error, "making the settings event failed");
                return;
            }
        }
        self.settings_sent = Some(snapshot);
    }

    /// Decode the images the frame wanted, in the background.
    fn decode(&mut self, wanted: &[ImageKey]) {
        let Some(page) = &self.page else {
            return;
        };
        for key in self.images.to_decode(wanted) {
            let page = page.clone();
            let blob = self.session.blobs.get(&key.path);
            wasm_bindgen_futures::spawn_local(async move {
                let image = match blob {
                    Some(blob) => media::decode(blob.bytes, key.width, key.height).await,
                    None => None,
                };
                let _ = page.send(Input::Decoded(key, image));
            });
        }
    }

    /// faro quit: draw the last frame and tell the page.
    fn exited(&mut self) {
        self.draw();
        let event = web_sys::CustomEvent::new(crate::dom::EXIT_EVENT);
        if let Ok(event) = event
            && self.painter.canvas().dispatch_event(&event).is_err()
        {
            tracing::warn!("telling the page faro quit failed");
        }
    }

    /// Forget the last frame and draw everything.
    pub fn full_frame(&mut self) {
        let Ok(()) = self.terminal.clear();
        self.draw();
    }

    /// Paint every row again without drawing (to measure the renderer).
    pub fn repaint(&mut self) {
        let backend = self.terminal.backend_mut();
        backend.mark_all();
        let rows = backend.take_dirty();
        let cursor = backend.cursor();
        self.painter.paint(backend.buffer(), &rows, cursor);
    }

    /// When the logo needs its next frame (D61): only while the last frame
    /// drew it moving, it should still move, and no frame is waiting anyway.
    fn logo_at(&self) -> Option<Instant> {
        let state = &self.session.state;
        (!self.dirty && state.geometry.logo_animated && state.logo_moves()).then(|| {
            self.last_draw
                .map_or_else(Instant::now, |t| t + ui::logo::FRAME)
        })
    }
}

/// One frame: milliseconds spent rendering (faro's UI and ratatui's diff)
/// and painting, and how many rows were painted.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub render: f64,
    pub paint: f64,
    pub rows: usize,
}

/// The last [`Timings::KEEP`] frames.
#[derive(Debug, Default)]
pub struct Timings {
    frames: std::collections::VecDeque<Timing>,
}

impl Timings {
    const KEEP: usize = 256;

    fn record(&mut self, timing: Timing) {
        if self.frames.len() == Self::KEEP {
            self.frames.pop_front();
        }
        self.frames.push_back(timing);
    }

    /// How many frames, the rows painted per frame on average, and the
    /// median, 95th percentile and worst frame time, render and paint, as
    /// JSON.
    pub fn summary(&self) -> String {
        let pick = |f: fn(&Timing) -> f64| {
            let mut values: Vec<f64> = self.frames.iter().map(f).collect();
            values.sort_by(f64::total_cmp);
            let at = |q: usize| {
                values
                    .get((values.len().saturating_sub(1)) * q / 100)
                    .copied()
                    .unwrap_or(0.0)
            };
            format!(
                r#"{{"p50":{:.2},"p95":{:.2},"max":{:.2}}}"#,
                at(50),
                at(95),
                at(100)
            )
        };
        let rows: usize = self.frames.iter().map(|t| t.rows).sum();
        format!(
            r#"{{"frames":{},"rows":{},"total":{},"render":{},"paint":{}}}"#,
            self.frames.len(),
            rows / self.frames.len().max(1),
            pick(|t| t.render + t.paint),
            pick(|t| t.render),
            pick(|t| t.paint),
        )
    }
}

/// Run the demo until the page goes away.
pub async fn run(shell: Rc<RefCell<Shell>>, mut page: UnboundedReceiver<Input>, mut inbox: Inbox) {
    let mut frame: Option<JsFuture> = None;
    loop {
        let (dirty, deadline, logo_at) = {
            let shell = shell.borrow();
            (
                shell.dirty,
                shell.session.state.next_deadline(),
                shell.logo_at(),
            )
        };
        if dirty && frame.is_none() {
            frame = animation_frame();
        }
        let wake = tokio::select! {
            Some(input) = page.recv() => Wake::Page(input),
            Some(event) = inbox.backend.recv() => Wake::App(AppEvent::Backend(event)),
            Some(event) = inbox.results.recv() => Wake::App(event),
            () = next_frame(&mut frame), if frame.is_some() => {
                frame = None;
                Wake::Frame
            }
            () = rt::sleep_until(logo_at.unwrap_or_else(Instant::now)), if logo_at.is_some() => {
                Wake::Logo
            }
            () = rt::sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => {
                Wake::App(AppEvent::Tick)
            }
            else => break,
        };
        let mut shell = shell.borrow_mut();
        shell.take(wake);
        if shell.session.state.quit {
            // Quitting the demo (D116): the page offers to start again.
            shell.exited();
            break;
        }
    }
}

/// The next `requestAnimationFrame`, as a future.
fn animation_frame() -> Option<JsFuture> {
    let window = web_sys::window()?;
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        if window.request_animation_frame(&resolve).is_err() {
            tracing::warn!("requestAnimationFrame failed");
        }
    });
    Some(JsFuture::from(promise))
}

async fn next_frame(frame: &mut Option<JsFuture>) {
    match frame {
        Some(future) => {
            if future.await.is_err() {
                tracing::debug!("an animation frame was rejected");
            }
        }
        None => std::future::pending().await,
    }
}

/// Seconds into the logo's animation, counted from when it started moving;
/// `None` while it stands still (as `app::logo_time` in the terminal app).
fn logo_time(since: &mut Option<Instant>, state: &State) -> Option<f64> {
    if !state.logo_moves() {
        *since = None;
        return None;
    }
    let start = *since.get_or_insert_with(Instant::now);
    Some(ui::logo::STILL + start.elapsed().as_secs_f64())
}

/// As many cells as fit the canvas's parent.
fn fit(painter: &Painter) -> (u16, u16) {
    let (width, height) = painter.parent_size();
    let (columns, rows) = painter.grid_for(width, height);
    (columns.max(1), rows.max(1))
}

/// The visitor's UTC offset now. Fixed for the visit: jiff's time zone
/// database would add hundreds of kilobytes for a demo with made-up dates.
fn local_time_zone() -> jiff::tz::TimeZone {
    let minutes = js_sys::Date::new_0().get_timezone_offset();
    // `getTimezoneOffset` is UTC minus local time, in minutes.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "UTC offsets are within ±26 hours"
    )]
    let seconds = (-minutes * 60.0).round() as i32;
    jiff::tz::Offset::from_seconds(seconds)
        .map_or(jiff::tz::TimeZone::UTC, jiff::tz::TimeZone::fixed)
}

fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or(0.0, |p| p.now())
}

fn document() -> Option<web_sys::Document> {
    web_sys::window()?.document()
}
