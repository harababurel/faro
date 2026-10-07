use std::time::Duration;

use super::Capability;

/// What went wrong, typed so the UI can show a useful state (offline banner,
/// "re-link this device", retry later) instead of a generic failure.
///
/// Detail strings are for logs: they must never contain message bodies or
/// phone numbers (DESIGN.md §12).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BackendError {
    /// Signal's servers can't be reached; retrying later may work.
    #[error("network error: {0}")]
    Network(String),

    /// This device is no longer linked (or never was). Needs re-linking.
    #[error("this device is not linked to a Signal account")]
    Unlinked,

    /// The server asked us to slow down.
    #[error("rate limited by the Signal server")]
    RateLimited { retry_after: Option<Duration> },

    /// The server or a peer sent something we couldn't process
    /// (e.g. a message that failed to decrypt).
    #[error("protocol error: {0}")]
    Protocol(String),

    /// The backend's own store (keys, sessions) failed.
    #[error("storage error: {0}")]
    Storage(String),

    /// The target (contact, group, message, attachment) doesn't exist.
    #[error("not found")]
    NotFound,

    /// The backend doesn't support this action; the UI should have hidden it.
    #[error("not supported by this backend: {0:?}")]
    Unsupported(Capability),
}

impl BackendError {
    /// Whether retrying the same operation later might succeed.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Network(_) | Self::RateLimited { .. } => true,
            Self::Unlinked
            | Self::Protocol(_)
            | Self::Storage(_)
            | Self::NotFound
            | Self::Unsupported(_) => false,
        }
    }
}
