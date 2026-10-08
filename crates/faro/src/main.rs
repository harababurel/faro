mod app;
mod cli;
mod config;
#[cfg(test)]
mod fixtures;
mod frames;
mod logging;
mod signal;
mod theme;
mod tui;
mod ui;

use anyhow::Context as _;
use clap::Parser as _;
use faro_core::backend::{Backend, BackendEvents};
use faro_core::guard::{self, Allowlist};
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
    // Never fall back to talking to everyone: a bad file stops faro.
    let allowlist = match &cli.allowlist {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading the allowlist {}", path.display()))?;
            let list = Allowlist::parse(&text)
                .with_context(|| format!("reading the allowlist {}", path.display()))?;
            Some(list)
        }
        None => None,
    };

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "faro starting");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting the async runtime")?;
    // presage's futures aren't `Send` (D30) and it spawns local tasks, so
    // everything runs inside a `LocalSet`.
    let local = tokio::task::LocalSet::new();
    match cli.command {
        Some(cli::Command::Link { device_name }) => {
            return local.block_on(&runtime, signal::link(paths.data_dir(), device_name));
        }
        Some(cli::Command::Ids { usernames }) => {
            return local.block_on(&runtime, signal::ids(paths.data_dir(), &usernames));
        }
        None => {}
    }
    // The Signal backend runs behind the allowlist guard (D62) unless the
    // user says otherwise, every time (D67).
    if !cli.mock && allowlist.is_none() && !cli.no_allowlist {
        anyhow::bail!(
            "the Signal backend needs --allowlist FILE (`faro ids` prints the IDs for it), \
             or --no-allowlist to talk to everyone; use --mock to try faro without an account"
        );
    }
    let mock = MockConfig {
        seed: cli.mock_seed,
        activity: Some(std::time::Duration::from_millis(cli.mock_activity.max(1))),
        disconnects: !cli.mock_no_faults,
        errors: !cli.mock_no_faults,
        ..MockConfig::default()
    };
    let db_file = paths.db_file();
    let data_dir = paths.data_dir().to_owned();
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
        let detected = terminal_setting(cli.emoji.map(Into::into), &settings).await;
        let startup = config::Startup {
            path: config_file,
            config: settings,
            problems,
            themes,
        };
        let result = if cli.mock {
            tracing::info!(
                seed = mock.seed,
                activity_ms = cli.mock_activity,
                faults = !cli.mock_no_faults,
                "starting mock backend"
            );
            let (backend, events) = MockBackend::new(&mock);
            match allowlist {
                Some(list) => {
                    let (backend, events) = guard::guard(backend, events, list);
                    serve(backend, events, &store, tz, detected, startup, false).await
                }
                None => serve(backend, events, &store, tz, detected, startup, false).await,
            }
        } else {
            tracing::info!("starting the Signal backend");
            let (backend, events) = signal::start(&data_dir).await?;
            if let Some(list) = allowlist {
                let (backend, events) = guard::guard(backend, events, list);
                Box::pin(serve(backend, events, &store, tz, detected, startup, false)).await
            } else {
                tracing::warn!("no allowlist: faro talks to everyone (D67)");
                Box::pin(serve(backend, events, &store, tz, detected, startup, true)).await
            }
        };
        store.close().await.context("closing the database")?;
        tracing::info!("faro exiting");
        result
    })
}

/// Load the directory and run the app on `backend`.
async fn serve<B: Backend + 'static>(
    backend: B,
    events: BackendEvents,
    store: &Store,
    tz: jiff::tz::TimeZone,
    detected: app::state::Detected,
    startup: config::Startup,
    unguarded: bool,
) -> anyhow::Result<()> {
    let mut directory = app::load_directory(&backend, store).await?;
    directory.unguarded = unguarded;
    app::run(
        backend,
        events,
        store.clone(),
        directory,
        tz,
        detected,
        startup,
    )
    .await
}

/// What the terminal checks decide at startup: the emoji mode, as chosen
/// (`--emoji`/`FARO_EMOJI`, else the config file) or (`auto`) compatible if
/// mosh is in the way (D47), and who draws block glyphs, for
/// `bubble_style = "auto"` (D58). `auto` keeps checking when the terminal
/// changes. The terminal is only looked at if something is `auto`.
async fn terminal_setting(
    cli: Option<faro_core::config::EmojiChoice>,
    config: &faro_core::config::Config,
) -> app::state::Detected {
    use app::state::{Detected, EmojiMode, EmojiSetting};
    use faro_core::config::{BubbleStyle, EmojiChoice};
    let from_cli = cli.is_some();
    let choice = cli.unwrap_or(config.ui.emoji);
    let auto = choice == EmojiChoice::Auto;
    let terminal = if auto || config.messages.bubble_style == BubbleStyle::Auto {
        app::system::inspect_terminal().await
    } else {
        app::system::Terminal::default()
    };
    let mode = match choice {
        EmojiChoice::Compatible => EmojiMode::Compatible,
        EmojiChoice::Auto if terminal.mosh => EmojiMode::Compatible,
        EmojiChoice::Auto | EmojiChoice::Full => EmojiMode::Full,
    };
    tracing::info!(?mode, auto, from_cli, glyphs = ?terminal.glyphs, "terminal");
    Detected {
        emoji: EmojiSetting {
            mode,
            auto,
            from_cli,
            checked_at: Some(tokio::time::Instant::now()),
        },
        glyphs: terminal.glyphs,
    }
}
