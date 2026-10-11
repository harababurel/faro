//! faro's app, shared by the terminal client (`crates/faro`) and the web demo
//! (`crates/faro-web`, M13): the state, the reducer, the key map, the
//! palette, themes and rendering into a ratatui buffer.
//!
//! Nothing here touches the terminal, the file system, processes or the
//! network: input arrives as [`app::term_event`] and [`app::event::AppEvent`],
//! work leaves as [`app::event::Effect`], and frames are drawn into a
//! `ratatui::Frame` with images from an [`app::image_cache::ImageCache`].
//! The binary owns the loop and the outside world.

pub mod app;
#[cfg(any(test, feature = "test-support"))]
pub mod fixtures;
pub mod theme;
pub mod ui;
pub mod version;
