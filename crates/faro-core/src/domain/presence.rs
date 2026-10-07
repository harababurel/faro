//! Transient state: typing indicators and the connection to Signal.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypingState {
    Started,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Connecting,
    Connected,
    /// Lost or not yet established; the backend retries on its own.
    Disconnected,
    /// The device was removed from the account (or never linked). Needs re-linking.
    Unlinked,
}
