//! Where the demo's effects go (D114): store work to the store worker,
//! backend commands to the command task, and the rest to the browser.

use faro_app::app::event::{AppEvent, Command, Effect};
use faro_app::app::state::{BlockGlyphs, GraphicsSupport, State, Terminal};
use tokio::sync::mpsc::UnboundedSender;

pub struct Router {
    pub commands: UnboundedSender<Command>,
    pub effects: UnboundedSender<Effect>,
    pub results: UnboundedSender<AppEvent>,
}

/// What the browser is, as far as faro's terminal checks go: no mosh, the
/// renderer draws block glyphs itself, and images are half-blocks.
pub const BROWSER: Terminal = Terminal {
    mosh: false,
    glyphs: BlockGlyphs::Native,
    graphics: GraphicsSupport::HalfblocksOnly,
    kitty: false,
};

impl Router {
    pub fn dispatch(&self, list: Vec<Effect>, _state: &State) {
        for effect in list {
            let delivered = match effect {
                Effect::Backend(command) => self.commands.send(command).is_ok(),
                Effect::CheckTerminal => self
                    .results
                    .send(AppEvent::TerminalChecked(BROWSER))
                    .is_ok(),
                // Settings last for the visit (D-c): the state already has them.
                Effect::SaveSetting(_) => true,
                Effect::Copy(_)
                | Effect::OpenUrl(_)
                | Effect::OpenFile(_)
                | Effect::PlayAudio { .. }
                | Effect::StopAudio
                | Effect::SaveToDownloads { .. }
                | Effect::ListFiles(_)
                | Effect::StageFile(_)
                | Effect::CheckPaste { .. }
                | Effect::ReadClipboardImage { .. }
                | Effect::Bell
                | Effect::DesktopNotify { .. } => {
                    tracing::debug!("effect not handled in the browser yet");
                    true
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
}
