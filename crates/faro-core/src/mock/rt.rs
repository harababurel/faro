//! The mock's clock, timers and task (M13 Phase 0 spike).
//!
//! Natively this is tokio's clock, so tests can pause and advance it. On wasm32
//! (`wasm-spike`) there is no tokio time driver and `std::time` panics, so the
//! browser's `setTimeout`, `performance.now()` and `Date.now()` stand in, and the
//! driver runs on the page's event loop. Phase 2 replaces this with the shared
//! runtime shim (WEBSITE.md §6.5).

#[cfg(not(target_arch = "wasm32"))]
pub(super) use native::*;
#[cfg(target_arch = "wasm32")]
pub(super) use web::*;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::future::Future;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    pub(in crate::mock) use tokio::time::{Instant, sleep, sleep_until};

    /// The driver task; aborted when the backend is dropped.
    pub(in crate::mock) struct Task(tokio::task::JoinHandle<()>);

    impl Task {
        pub(in crate::mock) fn abort(&self) {
            self.0.abort();
        }
    }

    /// Must be called within a tokio runtime, or `tokio::spawn` panics.
    pub(in crate::mock) fn spawn(task: impl Future<Output = ()> + Send + 'static) -> Task {
        Task(tokio::spawn(task))
    }

    pub(in crate::mock) fn since_unix_epoch() -> Option<Duration> {
        SystemTime::now().duration_since(UNIX_EPOCH).ok()
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use std::future::Future;
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::sync::Notify;
    pub(in crate::mock) use web_time::Instant;
    use web_time::{SystemTime, UNIX_EPOCH};

    pub(in crate::mock) async fn sleep(duration: Duration) {
        gloo_timers::future::sleep(duration).await;
    }

    pub(in crate::mock) async fn sleep_until(deadline: Instant) {
        sleep(deadline.saturating_duration_since(Instant::now())).await;
    }

    /// The driver task; `abort` wakes it to stop at its next await point.
    pub(in crate::mock) struct Task(Arc<Notify>);

    impl Task {
        pub(in crate::mock) fn abort(&self) {
            // `notify_one` keeps a permit, so this works even before the task runs.
            self.0.notify_one();
        }
    }

    pub(in crate::mock) fn spawn(task: impl Future<Output = ()> + 'static) -> Task {
        let stop = Arc::new(Notify::new());
        let stopped = Arc::clone(&stop);
        wasm_bindgen_futures::spawn_local(async move {
            tokio::select! {
                () = task => {}
                () = stopped.notified() => {}
            }
        });
        Task(stop)
    }

    pub(in crate::mock) fn since_unix_epoch() -> Option<Duration> {
        SystemTime::now().duration_since(UNIX_EPOCH).ok()
    }
}
