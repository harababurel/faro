//! Panics while the TUI is up (D151).
//!
//! tokio catches a task's panic and the rest of faro carries on, so a panic
//! in, say, the Signal receive loop used to leave faro running with the
//! terminal already handed back by the hook and nothing receiving. Now any
//! panic stops faro: the hook logs it, restores the terminal, prints it, and
//! wakes the app loop (`stopped`), which leaves through the normal shutdown
//! (database closed, log flushed); `main` then exits with an error.
//!
//! The exception is work faro chose to survive: decoders of untrusted input
//! (a picture someone sent), run through `contained`. Their panics are only
//! logged.

use std::backtrace::Backtrace;
use std::cell::Cell;
use std::panic::{self, AssertUnwindSafe, PanicHookInfo};
use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::Notify;

/// A panic outside `contained` happened this run.
static CRASHED: AtomicBool = AtomicBool::new(false);
/// Wakes the app loop after such a panic (keeps the wake-up if nobody waits yet).
static STOP: Notify = Notify::const_new();

thread_local! {
    /// This thread is inside `contained`.
    static CONTAINED: Cell<bool> = const { Cell::new(false) };
}

/// Install the hook once. `restore` gives the terminal back.
pub fn install(restore: fn()) {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let message = message(info);
            let location = info
                .location()
                .map_or_else(|| "unknown".to_owned(), ToString::to_string);
            if CONTAINED.get() {
                // Nothing on the terminal: the TUI is still up.
                tracing::warn!(%location, message, "contained a panic");
                return;
            }
            tracing::error!(
                %location,
                message,
                backtrace = %Backtrace::capture(),
                "faro panicked; stopping"
            );
            CRASHED.store(true, Ordering::Relaxed);
            restore();
            previous(info);
            STOP.notify_one();
        }));
    });
}

/// Resolves once a panic asked faro to stop.
pub async fn stopped() {
    STOP.notified().await;
}

/// Whether a panic stopped this run.
pub fn crashed() -> bool {
    CRASHED.load(Ordering::Relaxed)
}

/// Run `f`, turning a panic into `None` instead of stopping faro. Only for
/// synchronous work on untrusted input (decoders), whose half-done state is
/// dropped with the panic.
pub fn contained<T>(f: impl FnOnce() -> T) -> Option<T> {
    let outer = CONTAINED.replace(true);
    let result = panic::catch_unwind(AssertUnwindSafe(f));
    CONTAINED.set(outer);
    result.ok()
}

/// The panic's message, if it's a string (it almost always is).
fn message<'a>(info: &'a PanicHookInfo<'_>) -> &'a str {
    let payload = info.payload();
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("(not a string)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contained_turns_a_panic_into_none() {
        assert_eq!(contained(|| 7), Some(7));
        assert_eq!(contained(|| -> u8 { panic!("bad input") }), None);
        assert!(!CONTAINED.get(), "the flag is cleared after a panic");
    }

    #[test]
    fn contained_nests() {
        let inner = contained(|| {
            let caught = contained(|| -> u8 { panic!("inner") });
            (caught, CONTAINED.get())
        });
        assert_eq!(
            inner,
            Some((None, true)),
            "still contained after the inner one"
        );
        assert!(!CONTAINED.get());
    }
}
