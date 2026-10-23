//! Where the demo's effects go (D114, D115): store work to the store
//! worker, backend commands to the command task, and the rest to the page
//! or, for what a browser can't do, a notice (WEBSITE.md §4.8).

use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use faro_app::app::event::{AppEvent, Command, Effect};
use faro_app::app::state::{BlockGlyphs, GraphicsSupport, State, Terminal, TerminalName};
use faro_core::domain::MessageRef;
use tokio::sync::mpsc::UnboundedSender;

use crate::blobs::Blobs;

/// What faro says about files, processes and the like in the browser.
pub const NOT_HERE: &str = "Not in the web demo: try it in your terminal";

/// What the browser is, as far as faro's terminal checks go: no mosh, the
/// renderer draws block glyphs itself, and images are pictures as in kitty
/// (D135; `kitty` stays off: it means kitty's clipboard, which the page
/// doesn't speak).
pub const BROWSER: Terminal = Terminal {
    mosh: false,
    glyphs: BlockGlyphs::Native,
    graphics: GraphicsSupport::Direct,
    kitty: false,
    graphics_blocker: None,
    tmux_clients: Vec::new(),
    name: TerminalName::Other,
    remote: false,
};

/// The page's side of the effects: the clipboard, new tabs, the bell.
pub trait Page {
    /// Put `text` on the clipboard; a failure is answered on `results`.
    fn copy(&self, text: String, results: UnboundedSender<AppEvent>);
    /// Open an http(s) link in a new tab, without access to this page.
    fn open(&self, url: &str);
    /// Ring: the page shows it (the panel's border flashes).
    fn bell(&self);
    /// Play a voice note (WAV bytes); `done` hears when it ends, with an
    /// error if it couldn't play. One at a time: a new one stops the last.
    fn play(&self, bytes: Arc<[u8]>, done: Box<dyn FnOnce(Option<String>)>);
    /// Stop what's playing, without calling its `done`.
    fn stop(&self);
    /// `Escape` had nothing left to dismiss (§4.3): release the keyboard focus
    /// so the page takes keys again. Only touches the DOM, never the state.
    fn release_focus(&self);
}

pub struct Router {
    pub commands: UnboundedSender<Command>,
    pub effects: UnboundedSender<Effect>,
    pub results: UnboundedSender<AppEvent>,
    pub page: Rc<dyn Page>,
    pub blobs: Blobs,
}

impl Router {
    pub fn dispatch(&self, list: Vec<Effect>, _state: &State) {
        for effect in list {
            let delivered = match effect {
                Effect::Backend(command) => self.commands.send(command).is_ok(),
                Effect::CheckTerminal => self.answer(AppEvent::TerminalChecked(BROWSER)),
                // Settings last for the visit (D-c): the state already has them.
                // The demo never checks for updates (D148).
                Effect::SaveSetting(_) | Effect::DesktopNotify { .. } | Effect::CheckForUpdate => {
                    true
                }
                Effect::StopAudio => {
                    self.page.stop();
                    true
                }
                // An attachment's path means nothing outside the demo.
                Effect::Copy(text) if self.blobs.holds(&text) => {
                    self.answer(AppEvent::Unavailable(NOT_HERE.to_owned()))
                }
                Effect::Copy(text) => {
                    self.page.copy(text, self.results.clone());
                    true
                }
                Effect::OpenUrl(url) => {
                    if is_web_link(&url) {
                        self.page.open(&url);
                        true
                    } else {
                        self.answer(AppEvent::OpenFailed("not a web link".to_owned()))
                    }
                }
                Effect::Bell => {
                    self.page.bell();
                    true
                }
                // Esc had nothing to close: the page blurs the canvas (§4.3).
                Effect::ReleaseFocus => {
                    self.page.release_focus();
                    true
                }
                // The demo's transcription is canned, so its command is there.
                Effect::CheckCommand(program) => self.answer(AppEvent::CommandChecked {
                    program,
                    found: true,
                }),
                // A pasted path is just text here.
                Effect::CheckPaste { raw, .. } => {
                    self.answer(AppEvent::PasteChecked { raw, staged: None })
                }
                Effect::PlayAudio {
                    message,
                    index,
                    path,
                    ..
                } => self.play(message, index, &path),
                Effect::OpenFile(_)
                | Effect::SaveToDownloads { .. }
                | Effect::ListFiles(_)
                | Effect::StageFile(_)
                | Effect::ReadClipboardImage { .. } => {
                    self.answer(AppEvent::Unavailable(NOT_HERE.to_owned()))
                }
                store @ (Effect::LoadConversations
                | Effect::LoadMessages(_)
                | Effect::LoadOlder { .. }
                | Effect::LoadMessagesAround { .. }
                | Effect::SearchMessages { .. }
                | Effect::LoadMessageReceipts { .. }
                | Effect::Persist(_)
                | Effect::SaveAttachment { .. }
                | Effect::MarkRead(_)
                | Effect::ReadElsewhere(_)
                | Effect::SaveConversationState(..)
                | Effect::SaveNickname(..)
                | Effect::SaveRecentReactions(_)
                | Effect::LoadUpdateState
                | Effect::SaveUpdateState { .. }
                | Effect::RememberOpen(_)) => self.effects.send(store).is_ok(),
            };
            if !delivered {
                tracing::error!("a worker task stopped; dropping an effect");
            }
        }
    }

    fn play(&self, message: MessageRef, index: usize, path: &Path) -> bool {
        let Some(blob) = self.blobs.get(path) else {
            return self.answer(AppEvent::PlaybackFinished {
                message,
                index,
                error: Some("the voice note isn't downloaded".to_owned()),
            });
        };
        let results = self.results.clone();
        self.page.play(
            blob.bytes,
            Box::new(move |error| {
                let finished = AppEvent::PlaybackFinished {
                    message,
                    index,
                    error,
                };
                if results.send(finished).is_err() {
                    tracing::debug!("the demo stopped before a voice note ended");
                }
            }),
        );
        true
    }

    /// Answer an effect right away (through the loop, like a worker would).
    fn answer(&self, event: AppEvent) -> bool {
        self.results.send(event).is_ok()
    }
}

fn is_web_link(url: &str) -> bool {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

#[cfg(test)]
pub mod tests {
    use std::cell::RefCell;

    use super::*;

    /// A page that writes down what it was asked to do.
    #[derive(Default)]
    pub struct FakePage {
        pub log: RefCell<Vec<String>>,
    }

    impl Page for FakePage {
        fn copy(&self, text: String, _results: UnboundedSender<AppEvent>) {
            self.log.borrow_mut().push(format!("copy {text}"));
        }
        fn open(&self, url: &str) {
            self.log.borrow_mut().push(format!("open {url}"));
        }
        fn bell(&self) {
            self.log.borrow_mut().push("bell".to_owned());
        }
        fn play(&self, bytes: Arc<[u8]>, done: Box<dyn FnOnce(Option<String>)>) {
            self.log.borrow_mut().push(format!("play {}", bytes.len()));
            done(None);
        }
        fn stop(&self) {
            self.log.borrow_mut().push("stop".to_owned());
        }
        fn release_focus(&self) {
            self.log.borrow_mut().push("release_focus".to_owned());
        }
    }

    #[test]
    fn only_web_links_open() {
        assert!(is_web_link("https://example.com/x"));
        assert!(is_web_link("HTTP://example.com"));
        assert!(!is_web_link("javascript:alert(1)"));
        assert!(!is_web_link("file:///etc/passwd"));
        assert!(!is_web_link("http"));
    }
}
