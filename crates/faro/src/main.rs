mod app;
mod cli;
#[cfg(test)]
mod fixtures;
mod logging;
mod theme;
mod tui;
mod ui;

use anyhow::Context as _;
use clap::Parser as _;
use faro_core::mock::{MockBackend, MockConfig};
use faro_core::paths::Paths;
use faro_core::store::Store;

#[expect(
    clippy::disallowed_methods,
    reason = "the program's entry point: main blocks on the runtime until faro exits"
)]
fn main() -> anyhow::Result<()> {
    let cli = cli::Cli::parse();

    let paths = Paths::resolve(None, cli.data_dir).context("resolving faro's directories")?;
    paths
        .ensure_data_dirs()
        .with_context(|| format!("creating data directory {}", paths.data_dir().display()))?;
    let (_log_guard, _log_file) = logging::init(&paths.log_dir())?;
    // Reads TZ or /etc/localtime once, before the UI; falls back to UTC.
    let tz = jiff::tz::TimeZone::system();

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "faro starting");
    if !cli.mock {
        // Until the presage backend exists (M7), the mock is the only backend.
        tracing::info!("no Signal backend yet; using the mock backend");
    }
    let config = MockConfig {
        seed: cli.mock_seed,
        activity: Some(std::time::Duration::from_millis(cli.mock_activity.max(1))),
        disconnects: !cli.mock_no_faults,
        errors: !cli.mock_no_faults,
        ..MockConfig::default()
    };
    tracing::info!(
        seed = config.seed,
        activity_ms = cli.mock_activity,
        faults = !cli.mock_no_faults,
        "starting mock backend"
    );

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting the async runtime")?;
    let db_file = paths.db_file();
    // Backend futures may be `!Send` (D30): the app drives the backend from a
    // local task, so everything runs inside a `LocalSet`.
    let local = tokio::task::LocalSet::new();
    local.block_on(&runtime, async {
        let store = Store::open(&db_file)
            .await
            .with_context(|| format!("opening the database {}", db_file.display()))?;
        let unsent = store.fail_unsent().await?;
        if unsent > 0 {
            tracing::info!(
                unsent,
                "messages left sending by the last run marked failed"
            );
        }
        let (backend, events) = MockBackend::new(&config);
        let directory = app::load_directory(&backend, &store).await?;
        let result = app::run(backend, events, store.clone(), directory, tz).await;
        store.close().await.context("closing the database")?;
        tracing::info!("faro exiting");
        result
    })
}
