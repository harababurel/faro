//! The page's listeners (D114): DOM events read into `live::Input` and sent
//! to the loop. They never touch the state themselves, so an event the
//! browser dispatches while Rust is running (a `focus()` call, say) can't
//! find the state borrowed.

use std::cell::RefCell;
use std::rc::Weak;
use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;
use web_sys::{
    AddEventListenerOptions, ClipboardEvent, EventTarget, HtmlCanvasElement, KeyboardEvent,
    MouseEvent, ResizeObserver, WheelEvent,
};

use faro_app::app::event::AppEvent;
use faro_app::app::term_event::Event;
use wasm_bindgen_futures::JsFuture;

use crate::input::{self, Held, Route};
use crate::keys;
use crate::live::{Input, Pointer, Shell};
use crate::media::Audio;
use crate::router::Page;

type Handler = Closure<dyn FnMut(web_sys::Event)>;

/// The page around the canvas.
pub struct CanvasPage {
    pub canvas: HtmlCanvasElement,
    pub audio: Audio,
}

/// The event the canvas sends for the bell; the page decides how it looks.
pub const BELL_EVENT: &str = "faro-bell";
/// The event the canvas sends when faro quits; the page offers a restart.
pub const EXIT_EVENT: &str = "faro-exit";
/// The event the canvas sends when the settings its controls show change
/// (theme, configured theme, motion): the detail carries the new values,
/// so the page's listener never reaches back into the borrowed shell.
pub const SETTINGS_EVENT: &str = "faro-settings";
/// The event the canvas sends when the mock accepts a message the visitor
/// sent (§6.9's `Demo message sent`): no detail, and the page counts one
/// per visit. Dispatched while the shell is borrowed: the listener must
/// never call back into Rust (D125).
pub const SENT_EVENT: &str = "faro-sent";

impl Page for CanvasPage {
    fn copy(&self, text: String, results: UnboundedSender<AppEvent>) {
        let Some(window) = web_sys::window() else {
            return;
        };
        // Pages that aren't secure (plain http other than localhost) have no
        // `navigator.clipboard`; calling it anyway throws and stops the demo.
        let navigator = window.navigator();
        let available = js_sys::Reflect::get(&navigator, &JsValue::from_str("clipboard"))
            .is_ok_and(|clipboard| !clipboard.is_undefined());
        if !available {
            let text = "Couldn't copy: the browser only allows it on https pages".to_owned();
            let _ = results.send(AppEvent::Unavailable(text));
            return;
        }
        let promise = navigator.clipboard().write_text(&text);
        wasm_bindgen_futures::spawn_local(async move {
            if JsFuture::from(promise).await.is_err() {
                let text = "Couldn't copy: the browser kept the clipboard closed".to_owned();
                let _ = results.send(AppEvent::Unavailable(text));
            }
        });
    }

    fn open(&self, url: &str) {
        let opened = web_sys::window()
            .map(|w| w.open_with_url_and_target_and_features(url, "_blank", "noopener,noreferrer"));
        if !matches!(opened, Some(Ok(_))) {
            tracing::warn!("opening a link failed");
        }
    }

    fn bell(&self) {
        if let Ok(event) = web_sys::CustomEvent::new(BELL_EVENT) {
            let _ = self.canvas.dispatch_event(&event);
        }
    }

    fn play(&self, bytes: Arc<[u8]>, done: Box<dyn FnOnce(Option<String>)>) {
        self.audio.play(&bytes, done);
    }

    fn stop(&self) {
        self.audio.stop();
    }

    fn release_focus(&self) {
        // Only the DOM: the canvas's own blur listener posts `FocusLost`, and
        // the loop unfocuses faro on its next turn. Never borrow the state
        // here — we're called from inside the loop's `borrow_mut`.
        let _ = self.canvas.blur();
    }
}

/// The listeners, removed when dropped.
pub struct Listeners {
    attached: Vec<(EventTarget, &'static str, Handler)>,
    resize: Option<(ResizeObserver, Closure<dyn FnMut()>)>,
}

impl Listeners {
    pub fn attach(
        canvas: &HtmlCanvasElement,
        page: &UnboundedSender<Input>,
        shell: Weak<RefCell<Shell>>,
    ) -> Result<Self, JsValue> {
        let mut listeners = Self {
            attached: Vec::new(),
            resize: None,
        };
        // Focusable, so it gets keys after a click.
        if !canvas.has_attribute("tabindex") {
            canvas.set_attribute("tabindex", "0")?;
        }
        let target: &EventTarget = canvas;
        listeners.on(target, "keydown", page, {
            let canvas = canvas.clone();
            move |e| keydown(e, &canvas, &shell)
        })?;
        listeners.on(target, "mousedown", page, {
            let canvas = canvas.clone();
            move |e| mousedown(e, &canvas)
        })?;
        listeners.on(target, "mouseup", page, {
            let canvas = canvas.clone();
            move |e| mouse_up(e, &canvas)
        })?;
        listeners.on(target, "mousemove", page, {
            let canvas = canvas.clone();
            move |e| mouse_move(e, &canvas)
        })?;
        listeners.on(target, "wheel", page, {
            let canvas = canvas.clone();
            move |e| wheel(e, &canvas)
        })?;
        // faro's own menu only while it has the keys; otherwise the browser's
        // native context menu stays (D125).
        listeners.on(target, "contextmenu", page, {
            let canvas = canvas.clone();
            move |e| {
                if focused(&canvas) {
                    e.prevent_default();
                }
                None
            }
        })?;
        // The canvas gaining or losing the active element moves faro's focus,
        // but only in step with the predicate (a focus event while the window
        // is unfocused must not claim the keys).
        listeners.on(target, "focus", page, {
            let canvas = canvas.clone();
            move |_| focused(&canvas).then_some(Input::Terminal(Event::FocusGained))
        })?;
        listeners.on(target, "blur", page, {
            let canvas = canvas.clone();
            move |_| (!focused(&canvas)).then_some(Input::Terminal(Event::FocusLost))
        })?;
        if let Some(document) = canvas.owner_document() {
            // Paste goes to the focused element; the canvas isn't editable, so
            // listen on the document and check.
            let paste_canvas = canvas.clone();
            listeners.on(&document, "paste", page, move |e| paste(e, &paste_canvas))?;
            // A click anywhere off the canvas gives up the keys, even on text
            // that isn't focusable (D125).
            let down_canvas = canvas.clone();
            listeners.on(&document, "mousedown", page, move |e| {
                outside_mousedown(e, &down_canvas)
            })?;
            // A hidden tab suspends faro's focus; showing it again hands the
            // keys back only if the canvas is still active (D125).
            let vis_canvas = canvas.clone();
            listeners.on(&document, "visibilitychange", page, move |_| {
                Some(Input::Terminal(if focused(&vis_canvas) {
                    Event::FocusGained
                } else {
                    Event::FocusLost
                }))
            })?;
        }
        // The window losing focus always suspends faro; regaining it hands the
        // keys back only if the canvas is still the active element — never
        // stealing them from another element the visitor moved to (D125).
        if let Some(window) = web_sys::window() {
            listeners.on(&window, "blur", page, |_| {
                Some(Input::Terminal(Event::FocusLost))
            })?;
            listeners.on(&window, "focus", page, {
                let canvas = canvas.clone();
                move |_| focused(&canvas).then_some(Input::Terminal(Event::FocusGained))
            })?;
        }
        if let Some(parent) = canvas.parent_element() {
            let sender = page.clone();
            let callback: Closure<dyn FnMut()> = Closure::new(move || {
                let _ = sender.send(Input::Resized);
            });
            let observer = ResizeObserver::new(callback.as_ref().unchecked_ref())?;
            observer.observe(&parent);
            listeners.resize = Some((observer, callback));
        }
        Ok(listeners)
    }

    /// Listen for `kind` on `target`; `read` turns the event into input (or
    /// nothing). Never passive: a passive listener's `preventDefault` is
    /// ignored, so Tab would move the page's focus and the wheel scroll it.
    fn on(
        &mut self,
        target: &EventTarget,
        kind: &'static str,
        page: &UnboundedSender<Input>,
        mut read: impl FnMut(&web_sys::Event) -> Option<Input> + 'static,
    ) -> Result<(), JsValue> {
        let sender = page.clone();
        let handler: Handler = Closure::new(move |event: web_sys::Event| {
            if let Some(input) = read(&event) {
                // The loop is gone only when the demo is being dropped.
                let _ = sender.send(input);
            }
        });
        let options = AddEventListenerOptions::new();
        options.set_passive(false);
        target.add_event_listener_with_callback_and_add_event_listener_options(
            kind,
            handler.as_ref().unchecked_ref(),
            &options,
        )?;
        self.attached.push((target.clone(), kind, handler));
        Ok(())
    }
}

impl Drop for Listeners {
    fn drop(&mut self) {
        for (target, kind, handler) in &self.attached {
            let _ =
                target.remove_event_listener_with_callback(kind, handler.as_ref().unchecked_ref());
        }
        if let Some((observer, _)) = &self.resize {
            observer.disconnect();
        }
    }
}

fn held_keys(event: &KeyboardEvent) -> Held {
    Held {
        shift: event.shift_key(),
        ctrl: event.ctrl_key(),
        alt: event.alt_key(),
        meta: event.meta_key(),
    }
}

fn held_mouse(event: &MouseEvent) -> Held {
    Held {
        shift: event.shift_key(),
        ctrl: event.ctrl_key(),
        alt: event.alt_key(),
        meta: event.meta_key(),
    }
}

/// Whether faro should have the keys: the canvas is the page's active element,
/// the document is showing (not a hidden tab), and the window is focused
/// (D125). Every focus-related listener and gate reads this, so the app's
/// focus and the browser's agree.
pub(crate) fn focused(canvas: &HtmlCanvasElement) -> bool {
    let Some(document) = canvas.owner_document() else {
        return false;
    };
    let element: &web_sys::Element = canvas;
    document.active_element().as_ref() == Some(element)
        && !document.hidden()
        && document.has_focus().unwrap_or(false)
}

fn keydown(
    event: &web_sys::Event,
    canvas: &HtmlCanvasElement,
    shell: &Weak<RefCell<Shell>>,
) -> Option<Input> {
    let event = event.dyn_ref::<KeyboardEvent>()?;
    if event.is_composing() || !focused(canvas) {
        return None;
    }
    match input::route(&event.key(), &event.code(), held_keys(event)) {
        Route::Faro(key) => {
            // Busy only if the browser dispatched this while Rust runs
            // (rare); faro then gets the key.
            let theirs = shell
                .upgrade()
                .and_then(|shell| {
                    shell
                        .try_borrow()
                        .ok()
                        .map(|s| !keys::takes(s.state(), key))
                })
                .unwrap_or(false);
            if theirs {
                return None;
            }
            event.prevent_default();
            Some(Input::Terminal(Event::Key(key)))
        }
        Route::Browser => None,
    }
}

fn mousedown(event: &web_sys::Event, canvas: &HtmlCanvasElement) -> Option<Input> {
    let event = event.dyn_ref::<MouseEvent>()?;
    let left = event.button() == 0;
    if focused(canvas) {
        // faro has the keys: it gets every button. Non-primary buttons cancel
        // their default (which would move focus, though it's already here).
        if !left {
            event.prevent_default();
        }
    } else if left {
        // A left click takes the keys for faro, then acts (D125).
        let _ = canvas.focus();
    } else if event.button() == 2 {
        // A right click would focus the canvas and hand faro the keys by
        // accident, so cancel that default — but not the `contextmenu` event,
        // so the browser's own menu still opens (D125).
        event.prevent_default();
        return None;
    } else {
        // Middle and other buttons while unfocused: leave the browser's own
        // behaviour (autoscroll, middle-paste) alone.
        return None;
    }
    Some(Input::Pointer {
        action: Pointer::Down(event.button()),
        x: f64::from(event.offset_x()),
        y: f64::from(event.offset_y()),
        held: held_mouse(event),
    })
}

/// A `mousedown` on the document: off the canvas, give up the keys even when
/// the click landed on text that isn't focusable (D125).
fn outside_mousedown(event: &web_sys::Event, canvas: &HtmlCanvasElement) -> Option<Input> {
    let element: &web_sys::Element = canvas;
    let on_canvas = event.target().is_some_and(|t| {
        t.dyn_ref::<web_sys::Element>()
            .is_some_and(|el| el == element)
    });
    if !on_canvas && focused(canvas) {
        let _ = canvas.blur();
    }
    None
}

fn mouse_up(event: &web_sys::Event, canvas: &HtmlCanvasElement) -> Option<Input> {
    let event = event.dyn_ref::<MouseEvent>()?;
    if !focused(canvas) {
        return None;
    }
    Some(Input::Pointer {
        action: Pointer::Up(event.button()),
        x: f64::from(event.offset_x()),
        y: f64::from(event.offset_y()),
        held: held_mouse(event),
    })
}

fn mouse_move(event: &web_sys::Event, canvas: &HtmlCanvasElement) -> Option<Input> {
    let event = event.dyn_ref::<MouseEvent>()?;
    // Hover only counts while faro has the keys, so moving the mouse over an
    // unfocused demo can't reclaim them (D125).
    if !focused(canvas) {
        return None;
    }
    Some(Input::Pointer {
        action: Pointer::Move(event.buttons()),
        x: f64::from(event.offset_x()),
        y: f64::from(event.offset_y()),
        held: held_mouse(event),
    })
}

fn wheel(event: &web_sys::Event, canvas: &HtmlCanvasElement) -> Option<Input> {
    let event = event.dyn_ref::<WheelEvent>()?;
    // Only while it has the keys: an unfocused demo lets the page scroll.
    if !focused(canvas) {
        return None;
    }
    event.prevent_default();
    let mouse: &MouseEvent = event;
    Some(Input::Wheel {
        delta: event.delta_y(),
        mode: event.delta_mode(),
        x: f64::from(mouse.offset_x()),
        y: f64::from(mouse.offset_y()),
        held: held_mouse(mouse),
    })
}

fn paste(event: &web_sys::Event, canvas: &HtmlCanvasElement) -> Option<Input> {
    let event = event.dyn_ref::<ClipboardEvent>()?;
    if !focused(canvas) {
        return None;
    }
    let text = event.clipboard_data()?.get_data("text/plain").ok()?;
    event.prevent_default();
    (!text.is_empty()).then_some(Input::Terminal(Event::Paste(text)))
}
