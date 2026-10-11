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
    AddEventListenerOptions, ClipboardEvent, EventTarget, FocusEvent, HtmlCanvasElement,
    KeyboardEvent, MouseEvent, ResizeObserver, WheelEvent,
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

impl Page for CanvasPage {
    fn copy(&self, text: String, results: UnboundedSender<AppEvent>) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let promise = window.navigator().clipboard().write_text(&text);
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
        listeners.on(target, "keydown", page, move |e| keydown(e, &shell))?;
        listeners.on(target, "mousedown", page, |e| pointer(e, Pointer::Down))?;
        listeners.on(target, "mouseup", page, |e| pointer(e, Pointer::Up))?;
        listeners.on(target, "mousemove", page, mouse_move)?;
        listeners.on(target, "wheel", page, wheel)?;
        listeners.on(target, "contextmenu", page, |e| {
            e.prevent_default();
            None
        })?;
        listeners.on(target, "focus", page, |e| {
            e.dyn_ref::<FocusEvent>()
                .map(|_| Input::Terminal(Event::FocusGained))
        })?;
        listeners.on(target, "blur", page, |e| {
            e.dyn_ref::<FocusEvent>()
                .map(|_| Input::Terminal(Event::FocusLost))
        })?;
        // Paste goes to the focused element; the canvas isn't editable, so
        // listen on the document and check.
        if let Some(document) = canvas.owner_document() {
            let canvas = canvas.clone();
            listeners.on(&document, "paste", page, move |e| paste(e, &canvas))?;
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

fn keydown(event: &web_sys::Event, shell: &Weak<RefCell<Shell>>) -> Option<Input> {
    let event = event.dyn_ref::<KeyboardEvent>()?;
    if event.is_composing() {
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

fn pointer(event: &web_sys::Event, action: fn(i16) -> Pointer) -> Option<Input> {
    let event = event.dyn_ref::<MouseEvent>()?;
    Some(Input::Pointer {
        action: action(event.button()),
        x: f64::from(event.offset_x()),
        y: f64::from(event.offset_y()),
        held: held_mouse(event),
    })
}

fn mouse_move(event: &web_sys::Event) -> Option<Input> {
    let event = event.dyn_ref::<MouseEvent>()?;
    Some(Input::Pointer {
        action: Pointer::Move(event.buttons()),
        x: f64::from(event.offset_x()),
        y: f64::from(event.offset_y()),
        held: held_mouse(event),
    })
}

fn wheel(event: &web_sys::Event) -> Option<Input> {
    let event = event.dyn_ref::<WheelEvent>()?;
    // Only while it has the keys: an unfocused demo lets the page scroll.
    let focused = event
        .current_target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        .zip(web_sys::window().and_then(|w| w.document()))
        .is_some_and(|(el, doc)| doc.active_element().is_some_and(|a| a == el));
    if !focused {
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
    let document = canvas.owner_document()?;
    let element: &web_sys::Element = canvas;
    if document.active_element().as_ref() != Some(element) {
        return None;
    }
    let text = event.clipboard_data()?.get_data("text/plain").ok()?;
    event.prevent_default();
    (!text.is_empty()).then_some(Input::Terminal(Event::Paste(text)))
}
