//! The browser's media (D117): voice notes through Web Audio, and images
//! decoded by the browser: as bitmaps for pictures (D135), or scaled for the
//! half-block cache.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AudioBuffer, AudioBufferSourceNode, AudioContext, AudioScheduledSourceNode,
    CanvasRenderingContext2d, HtmlCanvasElement, ImageBitmap, ImageBitmapOptions, ResizeQuality,
};

use crate::images::Halfblocks;

/// What's playing: the source, and its `onended` handler (kept alive here,
/// never dropped while it runs).
type Playing = (AudioBufferSourceNode, Closure<dyn FnMut()>);

/// One voice note at a time.
#[derive(Default)]
pub struct Audio {
    context: RefCell<Option<AudioContext>>,
    playing: Rc<RefCell<Option<Playing>>>,
    /// Bumped by every play and stop, so a note still decoding when another
    /// starts (or playback stops) never plays.
    generation: Rc<Cell<u64>>,
}

impl Audio {
    pub fn play(&self, bytes: &Arc<[u8]>, done: Box<dyn FnOnce(Option<String>)>) {
        self.stop();
        let generation = self.generation.get();
        let Some(context) = self.context() else {
            done(Some("this browser has no Web Audio".to_owned()));
            return;
        };
        // The browser may have suspended it until the visitor interacted.
        let _ = context.resume();
        let data = js_sys::Uint8Array::from(&bytes[..]).buffer();
        let Ok(promise) = context.decode_audio_data(&data) else {
            done(Some("couldn't read the voice note".to_owned()));
            return;
        };
        let decoding = JsFuture::from(promise);
        let playing = Rc::clone(&self.playing);
        let current = Rc::clone(&self.generation);
        wasm_bindgen_futures::spawn_local(async move {
            let buffer = decoding
                .await
                .ok()
                .and_then(|b| b.dyn_into::<AudioBuffer>().ok());
            if current.get() != generation {
                return;
            }
            let Some(buffer) = buffer else {
                done(Some("couldn't decode the voice note".to_owned()));
                return;
            };
            match start(&context, &buffer, done) {
                Ok(started) => *playing.borrow_mut() = Some(started),
                Err(error) => tracing::warn!(?error, "playing a voice note failed"),
            }
        });
    }

    pub fn stop(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        if let Some((source, _ended)) = self.playing.borrow_mut().take() {
            let scheduled: &AudioScheduledSourceNode = &source;
            scheduled.set_onended(None);
            if scheduled.stop().is_err() {
                tracing::debug!("the voice note had already ended");
            }
        }
    }

    fn context(&self) -> Option<AudioContext> {
        let mut context = self.context.borrow_mut();
        if context.is_none() {
            *context = AudioContext::new().ok();
        }
        context.clone()
    }
}

impl Drop for Audio {
    /// Stop the voice note (and leave any still-decoding one unable to
    /// play — `stop` bumps the generation), then close the context:
    /// browsers cap how many a page may open, and every demo restart
    /// makes another.
    fn drop(&mut self) {
        self.stop();
        if let Some(context) = self.context.borrow_mut().take() {
            let _ = context.close();
        }
    }
}

fn start(
    context: &AudioContext,
    buffer: &AudioBuffer,
    done: Box<dyn FnOnce(Option<String>)>,
) -> Result<Playing, JsValue> {
    let source = context.create_buffer_source()?;
    source.set_buffer(Some(buffer));
    source.connect_with_audio_node(&context.destination())?;
    let mut done = Some(done);
    let ended: Closure<dyn FnMut()> = Closure::new(move || {
        if let Some(done) = done.take() {
            done(None);
        }
    });
    let scheduled: &AudioScheduledSourceNode = &source;
    scheduled.set_onended(Some(ended.as_ref().unchecked_ref()));
    scheduled.start()?;
    Ok((source, ended))
}

/// Decode `bytes` (JPEG, PNG, WebP…) into a bitmap at its own size, for the
/// painter to draw as a picture, scaled to its box (D135). `None` if the
/// browser can't.
pub async fn bitmap(bytes: Arc<[u8]>) -> Option<ImageBitmap> {
    let window = web_sys::window()?;
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(&bytes[..]));
    let blob = web_sys::Blob::new_with_u8_array_sequence(&parts).ok()?;
    let promise = window.create_image_bitmap_with_blob(&blob).ok()?;
    JsFuture::from(promise).await.ok()?.dyn_into().ok()
}

/// Decode `bytes` (JPEG, PNG, WebP…) and scale it to `width` × `height`
/// cells of half-blocks. `None` if the browser can't.
pub async fn decode(bytes: Arc<[u8]>, width: u16, height: u16) -> Option<Halfblocks> {
    let (w, h) = (u32::from(width), u32::from(height) * 2);
    let window = web_sys::window()?;
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(&bytes[..]));
    let blob = web_sys::Blob::new_with_u8_array_sequence(&parts).ok()?;
    // Scaled by the browser with a proper filter, not by drawing.
    let options = ImageBitmapOptions::new();
    options.set_resize_width(w);
    options.set_resize_height(h);
    options.set_resize_quality(ResizeQuality::High);
    let promise = window
        .create_image_bitmap_with_blob_and_image_bitmap_options(&blob, &options)
        .or_else(|_| window.create_image_bitmap_with_blob(&blob))
        .ok()?;
    let bitmap: ImageBitmap = JsFuture::from(promise).await.ok()?.dyn_into().ok()?;
    let canvas: HtmlCanvasElement = window
        .document()?
        .create_element("canvas")
        .ok()?
        .dyn_into()
        .ok()?;
    canvas.set_width(w);
    canvas.set_height(h);
    let context: CanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
    let (fw, fh) = (f64::from(w), f64::from(h));
    context
        .draw_image_with_image_bitmap_and_dw_and_dh(&bitmap, 0.0, 0.0, fw, fh)
        .ok()?;
    bitmap.close();
    let pixels = context.get_image_data(0.0, 0.0, fw, fh).ok()?.data();
    Halfblocks::from_rgba(width, height, &pixels)
}
