mod cli;
mod logging;

use anyhow::Context as _;
use clap::Parser as _;
use faro_core::paths::Paths;

fn main() -> anyhow::Result<()> {
    let cli = cli::Cli::parse();

    let paths = Paths::resolve(None, cli.data_dir).context("resolving faro's directories")?;
    paths
        .ensure_data_dirs()
        .with_context(|| format!("creating data directory {}", paths.data_dir().display()))?;
    let (_log_guard, log_file) = logging::init(&paths.log_dir())?;

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "faro starting");

    // The TUI arrives in M3 (docs/PLAN.md).
    println!(
        "faro {}: nothing to show yet; the TUI is still being built.\nLog: {}",
        env!("CARGO_PKG_VERSION"),
        log_file.display()
    );

    tracing::info!("faro exiting");
    Ok(())
}
