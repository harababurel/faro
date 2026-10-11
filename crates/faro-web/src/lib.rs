//! faro in the browser (M13, WEBSITE.md §6): the shared app on the mock,
//! drawn on a `<canvas>` by a Canvas 2D renderer (D106).
//!
//! - [`backend`]: a ratatui backend that keeps the cells and the rows that
//!   changed;
//! - [`glyphs`]: block, octant, box-drawing and other glyphs as shapes;
//! - `paint`: paints rows onto the canvas;
//! - [`demo`]: the state the demo starts from;
//! - `web`: what the page calls (wasm-bindgen).

pub mod backend;
pub mod demo;
pub mod glyphs;
mod paint;
mod web;

pub use demo::{Error, SEED, first_frame, first_state};
