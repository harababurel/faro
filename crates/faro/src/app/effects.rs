//! Runs the reducer's store effects one at a time and in order (so a
//! `Persist` is done before a later `MarkRead` or `LoadMessages`), and sends
//! results back to the app loop as `AppEvent`s. What each effect does is
//! shared with the web demo (`faro_app::app::storage`, D113).

use faro_core::store::Store;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::event::{AppEvent, Effect};
use faro_app::app::storage;

/// Start the worker. Effects are queued without limit: the reducer must never
/// wait, and the queue only grows as fast as the backend produces events.
/// Dropping the sender lets the worker finish the queue and stop; await the
/// handle before closing the store.
pub fn spawn(
    store: Store,
    results: mpsc::UnboundedSender<AppEvent>,
) -> (mpsc::UnboundedSender<Effect>, JoinHandle<()>) {
    let (effects, queue) = mpsc::unbounded_channel();
    let worker = tokio::spawn(run(store, queue, results));
    (effects, worker)
}

async fn run(
    store: Store,
    mut queue: mpsc::UnboundedReceiver<Effect>,
    results: mpsc::UnboundedSender<AppEvent>,
) {
    while let Some(effect) = queue.recv().await {
        if let Some(event) = storage::execute(&store, effect).await
            && results.send(event).is_err()
        {
            // The app loop is gone; keep draining so queued writes still happen.
            tracing::debug!("app loop stopped before a store result arrived");
        }
    }
}
