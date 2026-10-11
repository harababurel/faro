//! The async runtime faro's shared code runs on: clock, timers and tasks (M13).
//!
//! Natively this is tokio, so tests can pause and advance the clock
//! (`#[tokio::test(start_paused = true)]`). On wasm32 there is no tokio time
//! driver and `std::time` panics, so the browser's `setTimeout`,
//! `performance.now()` and `Date.now()` stand in, and tasks run on the page's
//! event loop. Code shared by the native app and the web demo (the mock, the
//! store, the guard, the reducer's deadlines) uses this module, never tokio's
//! clock or `tokio::spawn` directly.

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;
#[cfg(target_arch = "wasm32")]
pub use web::*;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::future::Future;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    pub use tokio::time::{Instant, sleep, sleep_until};

    /// A spawned task. Dropping it lets the task run on; [`Task::abort`] stops it.
    #[derive(Debug)]
    pub struct Task(tokio::task::JoinHandle<()>);

    impl Task {
        pub fn abort(&self) {
            self.0.abort();
        }
    }

    /// Run `task` in the background. Must be called within a tokio runtime, or
    /// `tokio::spawn` panics (faro's `main` and `#[tokio::test]` both are).
    pub fn spawn(task: impl Future<Output = ()> + Send + 'static) -> Task {
        Task(tokio::spawn(task))
    }

    /// Wall-clock time since the Unix epoch; `None` if the clock is before it.
    pub fn since_unix_epoch() -> Option<Duration> {
        SystemTime::now().duration_since(UNIX_EPOCH).ok()
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use std::future::Future;
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::sync::Notify;
    pub use web_time::Instant;
    use web_time::{SystemTime, UNIX_EPOCH};

    pub async fn sleep(duration: Duration) {
        gloo_timers::future::sleep(duration).await;
    }

    pub async fn sleep_until(deadline: Instant) {
        sleep(deadline.saturating_duration_since(Instant::now())).await;
    }

    /// A spawned task. Dropping it lets the task run on; [`Task::abort`] wakes
    /// it to stop at its next await point.
    #[derive(Debug)]
    pub struct Task(Arc<Notify>);

    impl Task {
        pub fn abort(&self) {
            // `notify_one` keeps a permit, so this works even before the task runs.
            self.0.notify_one();
        }
    }

    /// Run `task` on the page's event loop.
    pub fn spawn(task: impl Future<Output = ()> + 'static) -> Task {
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

    /// Wall-clock time since the Unix epoch; `None` if the clock is before it.
    pub fn since_unix_epoch() -> Option<Duration> {
        SystemTime::now().duration_since(UNIX_EPOCH).ok()
    }
}
