mod app;
mod cli;
mod config;
#[cfg(test)]
mod fixtures;
mod frames;
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

    let paths = Paths::resolve(cli.config, cli.data_dir).context("resolving faro's directories")?;
    paths
        .ensure_data_dirs()
        .with_context(|| format!("creating data directory {}", paths.data_dir().display()))?;
    let (_log_guard, _log_file) = logging::init(&paths.log_dir())?;
    // Reads TZ or /etc/localtime once, before the UI; falls back to UTC.
    let tz = jiff::tz::TimeZone::system();
    let config_file = paths.config_file().to_owned();
    let (mut settings, mut problems) = config::load(&config_file);
    let themes = config::load_themes(&config::themes_dir(&config_file));

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "faro starting");
    if !cli.mock {
        // Until the presage backend exists (M7), the mock is the only backend.
        tracing::info!("no Signal backend yet; using the mock backend");
    }
    let mock = MockConfig {
        seed: cli.mock_seed,
        activity: Some(std::time::Duration::from_millis(cli.mock_activity.max(1))),
        disconnects: !cli.mock_no_faults,
        errors: !cli.mock_no_faults,
        ..MockConfig::default()
    };
    tracing::info!(
        seed = mock.seed,
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
        problems.extend(config::migrate(&store, &config_file, &mut settings).await);
        let (backend, events) = MockBackend::new(&mock);
        let directory = app::load_directory(&backend, &store).await?;
        let emoji = emoji_setting(cli.emoji.map(Into::into), settings.ui.emoji).await;
        let startup = config::Startup {
            path: config_file,
            config: settings,
            problems,
            themes,
        };
        let result = app::run(
            backend,
            events,
            store.clone(),
            directory,
            tz,
            emoji,
            startup,
        )
        .await;
        store.close().await.context("closing the database")?;
        tracing::info!("faro exiting");
        result
    })
}

/// The emoji mode to start with: as chosen (`--emoji`/`FARO_EMOJI`, else the
/// config file), or (`auto`) compatible if mosh is in the way (D47). `auto`
/// keeps checking when the terminal changes.
async fn emoji_setting(
    cli: Option<faro_core::config::EmojiChoice>,
    config: faro_core::config::EmojiChoice,
) -> app::state::EmojiSetting {
    use app::state::{EmojiMode, EmojiSetting};
    use faro_core::config::EmojiChoice;
    let from_cli = cli.is_some();
    let (mode, auto) = match cli.unwrap_or(config) {
        EmojiChoice::Full => (EmojiMode::Full, false),
        EmojiChoice::Compatible => (EmojiMode::Compatible, false),
        EmojiChoice::Auto => {
            let mosh = app::system::mosh_attached().await;
            let mode = if mosh {
                EmojiMode::Compatible
            } else {
                EmojiMode::Full
            };
            (mode, true)
        }
    };
    tracing::info!(?mode, auto, from_cli, "emoji mode");
    EmojiSetting {
        mode,
        auto,
        from_cli,
        checked_at: auto.then(tokio::time::Instant::now),
    }
}
