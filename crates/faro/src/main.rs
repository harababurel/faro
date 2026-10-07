mod cli;
mod headless;
mod logging;

use anyhow::Context as _;
use clap::Parser as _;
use faro_core::mock::{MockBackend, MockConfig};
use faro_core::paths::Paths;

#[expect(
    clippy::print_stdout,
    reason = "placeholder output before the TUI exists (M3)"
)]
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
    let (_log_guard, log_file) = logging::init(&paths.log_dir())?;

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "faro starting");
    if !cli.mock {
        // Until the presage backend exists (M7), the mock is the only backend.
        tracing::info!("no Signal backend yet; using the mock backend");
    }
    let config = MockConfig {
        seed: cli.mock_seed,
        ..MockConfig::default()
    };
    tracing::info!(seed = config.seed, "starting mock backend");

    println!(
        "faro {}: no UI yet (it arrives in M3).\n\
         Running the mock backend (seed {}); events are logged to {}\n\
         Press Ctrl+C to stop.",
        env!("CARGO_PKG_VERSION"),
        config.seed,
        log_file.display()
    );

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting the async runtime")?;
    let summary = runtime.block_on(async {
        let (backend, events) = MockBackend::new(&config);
        headless::run(&backend, events, ctrl_c()).await
    });

    println!("\nStopped: {summary}.");
    tracing::info!(%summary, "faro exiting");
    Ok(())
}

async fn ctrl_c() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "can't listen for Ctrl+C; stopping");
    }
}
