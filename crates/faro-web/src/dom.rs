//! The page's listeners (D114): DOM events read into `live::Input` and sent
//! to the loop. They never touch the state themselves, so an event the
//! browser dispatches while Rust is running (a `focus()` call, say) can't
//! find the state borrowed.

use tokio::sync::mpsc::UnboundedSender;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;
use web_sys::{
    AddEventListenerOptions, ClipboardEvent, EventTarget, FocusEvent, HtmlCanvasElement,
    KeyboardEvent, MouseEvent, ResizeObserver, WheelEvent,
};

use crate::input::{self, Held, Route};
use crate::live::{Input, Pointer};
use faro_app::app::term_event::Event;

type Handler = Closure<dyn FnMut(web_sys::Event)>;

/// The listeners, removed when dropped.
pub struct Listeners {
    attached: Vec<(EventTarget, &'static str, Handler)>,
    resize: Option<(ResizeObserver, Closure<dyn FnMut()>)>,
}

impl Listeners {
    pub fn attach(
        canvas: &HtmlCanvasElement,
        page: &UnboundedSender<Input>,
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
        listeners.on(target, "keydown", page, keydown, false)?;
        listeners.on(
            target,
            "mousedown",
            page,
            |e| pointer(e, Pointer::Down),
            false,
        )?;
        listeners.on(target, "mouseup", page, |e| pointer(e, Pointer::Up), false)?;
        listeners.on(target, "mousemove", page, mouse_move, false)?;
        listeners.on(target, "wheel", page, wheel, true)?;
        listeners.on(
            target,
            "contextmenu",
            page,
            |e| {
                e.prevent_default();
                None
            },
            false,
        )?;
        listeners.on(
            target,
            "focus",
            page,
            |e| {
                e.dyn_ref::<FocusEvent>()
                    .map(|_| Input::Terminal(Event::FocusGained))
            },
            false,
        )?;
        listeners.on(
            target,
            "blur",
            page,
            |e| {
                e.dyn_ref::<FocusEvent>()
                    .map(|_| Input::Terminal(Event::FocusLost))
            },
            false,
        )?;
        // Paste goes to the focused element; the canvas isn't editable, so
        // listen on the document and check.
        if let Some(document) = canvas.owner_document() {
            let canvas = canvas.clone();
            listeners.on(&document, "paste", page, move |e| paste(e, &canvas), false)?;
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
    /// nothing). `active`: the listener may call `preventDefault` on events
    /// browsers make passive by default (`wheel`).
    fn on(
        &mut self,
        target: &EventTarget,
        kind: &'static str,
        page: &UnboundedSender<Input>,
        mut read: impl FnMut(&web_sys::Event) -> Option<Input> + 'static,
        active: bool,
    ) -> Result<(), JsValue> {
        let sender = page.clone();
        let handler: Handler = Closure::new(move |event: web_sys::Event| {
            if let Some(input) = read(&event) {
                // The loop is gone only when the demo is being dropped.
                let _ = sender.send(input);
            }
        });
        let options = AddEventListenerOptions::new();
        options.set_passive(!active);
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

fn keydown(event: &web_sys::Event) -> Option<Input> {
    let event = event.dyn_ref::<KeyboardEvent>()?;
    if event.is_composing() {
        return None;
    }
    match input::route(&event.key(), &event.code(), held_keys(event)) {
        Route::Faro(key) => {
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
