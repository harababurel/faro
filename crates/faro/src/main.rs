mod app;
mod cli;
mod config;
mod frames;
mod logging;
mod signal;
mod tui;
// M13 Phase 0 spike (throwaway): frames for the web renderer prototypes.
#[cfg(test)]
mod web_spike;

#[cfg(test)]
use faro_app::fixtures;
use faro_app::{theme, ui, version};

use anyhow::Context as _;
use faro_core::backend::{Backend, BackendEvents};
use faro_core::guard::{self, Allowlist};
use faro_core::mock::{MockBackend, MockConfig};
use faro_core::paths::{self, DemoDir, Paths};
use faro_core::store::{Profile, Store};

#[expect(
    clippy::disallowed_methods,
    reason = "the program's entry point: main blocks on the runtime until faro exits"
)]
fn main() -> anyhow::Result<()> {
    let invocation = cli::Invocation::parse();
    let demo = match &invocation.cli.command {
        Some(cli::Command::Demo(args)) => Some(args.clone()),
        Some(cli::Command::Link { .. } | cli::Command::Ids { .. }) | None => None,
    };
    let profile = if demo.is_some() {
        Profile::Demo
    } else {
        Profile::Signal
    };
    // Before any check: `--fresh` asks the store whether a database is a
    // demo one.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting the async runtime")?;
    let (paths, demo_dir) = resolve_paths(&invocation, demo.as_ref(), &runtime)?;
    let allowlist_file = invocation.allowlist().map(std::path::Path::to_owned);
    let ignored_allowlist = invocation.ignores_env_allowlist();
    let cli = invocation.cli;
    paths
        .ensure_data_dirs()
        .with_context(|| format!("creating data directory {}", paths.data_dir().display()))?;
    let (_log_guard, _log_file) = logging::init(&paths.log_dir())?;
    // Reads TZ or /etc/localtime once, before the UI; falls back to UTC.
    let tz = jiff::tz::TimeZone::system();
    let config_file = paths.config_file().to_owned();
    let (mut settings, mut problems) = config::load(&config_file);
    let themes = config::load_themes(&config::themes_dir(&config_file));
    let allowlist = read_allowlist(allowlist_file.as_deref())?;

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "faro starting");
    if let Some(outcome) = demo_dir {
        tracing::info!(dir = %paths.data_dir().display(), ?outcome, "demo directory");
        if cli.data_dir.is_some() {
            // Only the environment can have set it: a typed one was refused.
            tracing::info!("FARO_DATA_DIR ignored: the demo never uses it (D87)");
        }
        if ignored_allowlist {
            tracing::info!("FARO_ALLOWLIST ignored: the demo only takes a typed --allowlist (D87)");
        }
    }
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
        Some(cli::Command::Demo(_)) | None => {}
    }
    // The allowlist guard (D62) is for testing: on only with --allowlist
    // (D95); `--no-allowlist` is still accepted and changes nothing.
    let db_file = paths.db_file();
    let data_dir = paths.data_dir().to_owned();
    local.block_on(&runtime, async {
        // Refused before anything touches it if it's the other profile's (D87).
        let store = Store::open(&db_file, profile)
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
            attachments_dir: paths.attachments_dir(),
            config: settings,
            problems,
            themes,
            profile,
        };
        let result = if let Some(args) = &demo {
            serve_demo(args, allowlist, &store, tz, detected, startup).await
        } else {
            tracing::info!("starting the Signal backend");
            let (backend, events) = signal::start(&data_dir).await?;
            if let Some(list) = allowlist {
                let (backend, events) = guard::guard(backend, events, list);
                Box::pin(serve(backend, events, &store, tz, detected, startup, true)).await
            } else {
                Box::pin(serve(backend, events, &store, tz, detected, startup, false)).await
            }
        };
        store.close().await.context("closing the database")?;
        tracing::info!("faro exiting");
        result
    })
}

/// Where faro's files are for this run, checked before anything is created
/// or logged (D87): the demo's own directory, prepared; or the real paths,
/// refused if they lead to a demo directory.
fn resolve_paths(
    invocation: &cli::Invocation,
    demo: Option<&cli::DemoArgs>,
    runtime: &tokio::runtime::Runtime,
) -> anyhow::Result<(Paths, Option<DemoDir>)> {
    if let Some(args) = demo {
        let (paths, outcome) = demo_paths(invocation, args, runtime)?;
        return Ok((paths, Some(outcome)));
    }
    let cli = &invocation.cli;
    let paths = Paths::resolve(cli.config.clone(), cli.data_dir.clone())
        .context("resolving faro's directories")?;
    // Inside a demo directory, `faro demo --fresh` could wipe it.
    let around = paths::demo_dir_around(paths.data_dir())
        .with_context(|| format!("checking {}", paths.data_dir().display()))?;
    if let Some(demo_dir) = around {
        anyhow::bail!(
            "{} is inside a demo directory ({} has {}): run `faro demo` for the demo, \
             or keep your data elsewhere",
            paths.data_dir().display(),
            demo_dir.display(),
            paths::DEMO_MARKER
        );
    }
    Ok((paths, None))
}

/// The allowlist at `path`, if one was given. Never falls back to talking
/// to everyone: a bad file stops faro.
#[expect(
    clippy::disallowed_methods,
    reason = "startup, before the UI: reads the allowlist file"
)]
fn read_allowlist(path: Option<&std::path::Path>) -> anyhow::Result<Option<Allowlist>> {
    let Some(path) = path else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading the allowlist {}", path.display()))?;
    let list = Allowlist::parse(&text)
        .with_context(|| format!("reading the allowlist {}", path.display()))?;
    Ok(Some(list))
}

/// Where `faro demo` keeps its data (D87), checked and ready: `--dir` or
/// `<temp>/faro-demo-<uid>/`, never `--data-dir` or `FARO_DATA_DIR`, and
/// never overlapping either real data directory (the default one or
/// `FARO_DATA_DIR`'s). The config file is the real one.
#[expect(
    clippy::disallowed_methods,
    reason = "startup, before the UI: `--fresh` waits for the store's read-only peek"
)]
fn demo_paths(
    invocation: &cli::Invocation,
    args: &cli::DemoArgs,
    runtime: &tokio::runtime::Runtime,
) -> anyhow::Result<(Paths, DemoDir)> {
    if let Some(conflict) = invocation.demo_conflict() {
        anyhow::bail!(conflict);
    }
    let cli = &invocation.cli;
    let real = Paths::resolve(cli.config.clone(), None).context("resolving faro's directories")?;
    let mut real_data_dirs = vec![real.data_dir().to_owned()];
    real_data_dirs.extend(cli.data_dir.clone());
    let uid = paths::current_uid();
    let dir = args
        .dir
        .clone()
        .unwrap_or_else(|| paths::default_demo_dir(&std::env::temp_dir(), uid));
    // `--fresh` deletes a `faro.db` only if it's marked as the demo's.
    let is_demo_db = |db: &std::path::Path| {
        matches!(
            runtime.block_on(Store::profile_of(db)),
            Ok(Some(Profile::Demo))
        )
    };
    let outcome = paths::prepare_demo_dir(&dir, args.fresh, &real_data_dirs, uid, is_demo_db)?;
    Ok((real.demo(dir), outcome))
}

/// Run the app on the mock backend, built from `faro demo`'s flags, behind
/// the allowlist guard if one was given.
async fn serve_demo(
    args: &cli::DemoArgs,
    allowlist: Option<Allowlist>,
    store: &Store,
    tz: jiff::tz::TimeZone,
    detected: app::state::Detected,
    startup: config::Startup,
) -> anyhow::Result<()> {
    tracing::info!(
        seed = args.seed,
        activity_ms = args.activity,
        faults = !args.no_faults,
        "starting the demo (mock backend)"
    );
    let mock = MockConfig {
        seed: args.seed,
        activity: Some(std::time::Duration::from_millis(args.activity.max(1))),
        disconnects: !args.no_faults,
        errors: !args.no_faults,
        ..MockConfig::default()
    };
    let (backend, events) = MockBackend::new(&mock);
    match allowlist {
        Some(list) => {
            let (backend, events) = guard::guard(backend, events, list);
            serve(backend, events, store, tz, detected, startup, true).await
        }
        None => serve(backend, events, store, tz, detected, startup, false).await,
    }
}

/// Load the directory and run the app on `backend`.
async fn serve<B: Backend + 'static>(
    backend: B,
    events: BackendEvents,
    store: &Store,
    tz: jiff::tz::TimeZone,
    detected: app::state::Detected,
    startup: config::Startup,
    guarded: bool,
) -> anyhow::Result<()> {
    let mut directory = app::load_directory(&backend, store).await?;
    directory.guarded = guarded;
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
    use faro_core::config::{BubbleStyle, EmojiChoice, InlineImages};
    let from_cli = cli.is_some();
    let choice = cli.unwrap_or(config.ui.emoji);
    let auto = choice == EmojiChoice::Auto;
    // Forced graphics protocols too: a new tmux client means measuring
    // and sending images again (D137).
    let terminal = if auto
        || config.messages.bubble_style == BubbleStyle::Auto
        || !matches!(
            config.media.inline_images,
            InlineImages::Halfblocks | InlineImages::Off
        ) {
        app::system::inspect_terminal().await
    } else {
        app::state::Terminal::default()
    };
    let mode = match choice {
        EmojiChoice::Compatible => EmojiMode::Compatible,
        EmojiChoice::Auto if terminal.mosh => EmojiMode::Compatible,
        EmojiChoice::Auto | EmojiChoice::Full => EmojiMode::Full,
    };
    tracing::info!(
        ?mode,
        auto,
        from_cli,
        glyphs = ?terminal.glyphs,
        graphics = ?terminal.graphics,
        "terminal"
    );
    Detected {
        emoji: EmojiSetting {
            mode,
            auto,
            from_cli,
            mosh: terminal.mosh,
            checked_at: Some(tokio::time::Instant::now()),
            exact: false,
        },
        glyphs: terminal.glyphs,
        graphics: terminal.graphics,
        graphics_blocker: terminal.graphics_blocker,
        tmux_clients: terminal.tmux_clients,
    }
}
