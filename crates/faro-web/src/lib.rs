//! faro in the browser (M13, WEBSITE.md §6): the shared app on the mock,
//! drawn on a `<canvas>` by a Canvas 2D renderer (D106).
//!
//! - [`backend`]: a ratatui backend that keeps the cells and the rows that
//!   changed;
//! - [`glyphs`]: block, octant, box-drawing and other glyphs as shapes;
//! - [`color`]: cell colours as RGB and cell widths, free of canvas types;
//! - [`metrics`]: the cell shape both the painter and the poster use;
//! - [`svg`]: a buffer as an SVG document (the poster and OG image, §6.4);
//! - [`blobs`], [`images`]: attachments in memory, images as half-blocks;
//! - `paint`: paints rows onto the canvas;
//! - [`demo`]: a still first frame (tests, the poster);
//! - [`session`]: the mock, the store, the state and the workers;
//! - [`input`]: DOM input as faro's terminal events;
//! - [`keys`]: the demo's settings, and which chords faro takes;
//! - `router`: where effects go;
//! - `live`: the event loop; `dom`: the page's listeners; `media`: Web
//!   Audio and image decoding;
//! - `web`: what the page calls (wasm-bindgen).
//!
//! `dom`, `live`, `media`, `paint` and `web` only build for wasm32: they need the page, and
//! their tasks aren't `Send` (natively `rt::spawn` wants that).

pub mod backend;
pub mod blobs;
pub mod color;
pub mod demo;
#[cfg(target_arch = "wasm32")]
mod dom;
pub mod glyphs;
pub mod images;
pub mod input;
pub mod keys;
#[cfg(target_arch = "wasm32")]
mod live;
#[cfg(target_arch = "wasm32")]
mod media;
pub mod metrics;
#[cfg(target_arch = "wasm32")]
mod paint;
mod router;
pub mod session;
pub mod svg;
#[cfg(target_arch = "wasm32")]
mod web;

pub use demo::{Error, SEED, first_frame, first_state};
