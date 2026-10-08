//! A hidden debugging aid for terminal glitches (D55): with
//! `FARO_DEBUG_FRAMES=<file>` set, the text of every frame faro draws is
//! appended to that file, so a recording of faro's output can be replayed
//! frame by frame and compared with what faro meant to draw.
//!
//! Each frame is a header line `@@ frame N WxH` followed by H rows, one line
//! per row, a wide character written once (as `tmux capture-pane -p` shows
//! it). Frame N is the one that ends with the Nth synchronized-update end
//! (`ESC [ ? 2026 l`) in faro's output, counting from 1: nothing is added to
//! the output itself. Off by default; when off it costs one `Option` check per
//! frame. Frames are written by a task, never on the UI thread.

use std::path::PathBuf;

use ratatui::buffer::Buffer;
use tokio::io::AsyncWriteExt as _;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use unicode_width::UnicodeWidthStr as _;

/// The environment variable that turns the frame log on.
pub const ENV: &str = "FARO_DEBUG_FRAMES";

/// Sends drawn frames to the writer task.
pub struct FrameLog {
    frames: mpsc::UnboundedSender<String>,
    count: u64,
}

impl FrameLog {
    /// Start logging frames if [`ENV`] names a file (truncated first). The
    /// task ends once the log is dropped and everything is written.
    pub fn from_env() -> Option<(Self, JoinHandle<()>)> {
        let path = PathBuf::from(std::env::var_os(ENV).filter(|p| !p.is_empty())?);
        let (frames, receiver) = mpsc::unbounded_channel();
        tracing::info!(path = %path.display(), "logging drawn frames");
        let task = tokio::spawn(write(path, receiver));
        Some((Self { frames, count: 0 }, task))
    }

    /// Record a drawn frame.
    pub fn record(&mut self, buffer: &Buffer) {
        self.count += 1;
        let area = buffer.area;
        let mut text = format!("@@ frame {} {}x{}\n", self.count, area.width, area.height);
        text.push_str(&grid(buffer));
        // The writer only stops when the log is dropped or failed (logged).
        let _ = self.frames.send(text);
    }
}

async fn write(path: PathBuf, mut frames: mpsc::UnboundedReceiver<String>) {
    let file = match tokio::fs::File::create(&path).await {
        Ok(file) => file,
        Err(error) => {
            tracing::error!(%error, path = %path.display(), "can't write the frame log");
            return;
        }
    };
    let mut out = tokio::io::BufWriter::new(file);
    while let Some(frame) = frames.recv().await {
        let mut result = out.write_all(frame.as_bytes()).await;
        // Write whatever else is already queued before flushing.
        while result.is_ok()
            && let Ok(frame) = frames.try_recv()
        {
            result = out.write_all(frame.as_bytes()).await;
        }
        if let Err(error) = result.and(out.flush().await) {
            tracing::error!(%error, "writing the frame log failed; stopping it");
            return;
        }
    }
}

/// A buffer's text: one line per row, a wide character once (its hidden
/// cells aren't printed).
pub fn grid(buffer: &Buffer) -> String {
    let width = usize::from(buffer.area.width).max(1);
    let mut out = String::new();
    for row in buffer.content.chunks(width) {
        let mut skip = 0;
        for cell in row {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let symbol = cell.symbol();
            skip = symbol.width().saturating_sub(1);
            out.push_str(symbol);
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;
    use ratatui::style::Style;

    use super::*;

    #[test]
    fn grid_prints_wide_characters_once() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 6, 2));
        buffer.set_string(0, 0, "a🇩🇪b", Style::new());
        buffer.set_string(0, 1, "日本", Style::new());
        assert_eq!(grid(&buffer), "a🇩🇪b  \n日本  \n");
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "#[tokio::test] expands to block_on, and the test reads the file back; tests may block"
    )]
    fn frames_are_numbered_from_one() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("frames.txt");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let (frames, receiver) = mpsc::unbounded_channel();
            let task = tokio::spawn(write(path.clone(), receiver));
            let mut log = FrameLog { frames, count: 0 };
            let mut buffer = Buffer::empty(Rect::new(0, 0, 3, 1));
            log.record(&buffer);
            buffer.set_string(0, 0, "hi", Style::new());
            log.record(&buffer);
            drop(log);
            task.await.expect("writer");
        });
        let text = std::fs::read_to_string(&path).expect("frame log");
        assert_eq!(text, "@@ frame 1 3x1\n   \n@@ frame 2 3x1\nhi \n");
    }
}
