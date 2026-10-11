//! Where the demo's effects go (D114, D115): store work to the store
//! worker, backend commands to the command task, and the rest to the page
//! or, for what a browser can't do, a notice (WEBSITE.md §4.8).

use std::rc::Rc;

use faro_app::app::event::{AppEvent, Command, Effect};
use faro_app::app::state::{BlockGlyphs, GraphicsSupport, State, Terminal};
use tokio::sync::mpsc::UnboundedSender;

/// What faro says about files, processes and the like in the browser.
pub const NOT_HERE: &str = "Not in the web demo: try it in your terminal";

/// What the browser is, as far as faro's terminal checks go: no mosh, the
/// renderer draws block glyphs itself, and images are half-blocks.
pub const BROWSER: Terminal = Terminal {
    mosh: false,
    glyphs: BlockGlyphs::Native,
    graphics: GraphicsSupport::HalfblocksOnly,
    kitty: false,
};

/// The page's side of the effects: the clipboard, new tabs, the bell.
pub trait Page {
    /// Put `text` on the clipboard; a failure is answered on `results`.
    fn copy(&self, text: String, results: UnboundedSender<AppEvent>);
    /// Open an http(s) link in a new tab, without access to this page.
    fn open(&self, url: &str);
    /// Ring: the page shows it (the panel's border flashes).
    fn bell(&self);
}

pub struct Router {
    pub commands: UnboundedSender<Command>,
    pub effects: UnboundedSender<Effect>,
    pub results: UnboundedSender<AppEvent>,
    pub page: Rc<dyn Page>,
}

impl Router {
    pub fn dispatch(&self, list: Vec<Effect>, _state: &State) {
        for effect in list {
            let delivered = match effect {
                Effect::Backend(command) => self.commands.send(command).is_ok(),
                Effect::CheckTerminal => self.answer(AppEvent::TerminalChecked(BROWSER)),
                // Settings last for the visit (D-c): the state already has them.
                Effect::SaveSetting(_) | Effect::DesktopNotify { .. } | Effect::StopAudio => true,
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
                // A pasted path is just text here.
                Effect::CheckPaste { raw, .. } => {
                    self.answer(AppEvent::PasteChecked { raw, staged: None })
                }
                Effect::PlayAudio { message, index, .. } => {
                    self.answer(AppEvent::PlaybackFinished {
                        message,
                        index,
                        error: Some(NOT_HERE.to_owned()),
                    })
                }
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
                | Effect::RememberOpen(_)) => self.effects.send(store).is_ok(),
            };
            if !delivered {
                tracing::error!("a worker task stopped; dropping an effect");
            }
        }
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
