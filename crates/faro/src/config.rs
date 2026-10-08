//! The config file on disk (M5, D48): written on first run, read at startup,
//! watched for changes, and edited when a setting changes in the app. What
//! the settings mean is in `faro_core::config`; applying them is the
//! reducer's job (`app/update/settings.rs`).

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use faro_core::config::{self as core, Config, ConfigValue, Problem, SettingEdit};
use faro_core::paths;
use faro_core::store::Store;
use notify::{EventKind, RecursiveMode, Watcher as _};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::app::event::AppEvent;
use crate::app::keymap;
use crate::theme::ThemeText;

/// Changes in a burst (an editor saving in steps) are read once, this long
/// after the last one.
pub const DEBOUNCE: Duration = Duration::from_millis(300);

/// `app_state` keys of the preferences kept in the database before M5
/// (D41, D45), and the settings they became.
const MOVED: [(&str, &str); 2] = [
    ("ui.allowlist_mode", core::key::ALLOWLIST),
    ("ui.message_layout", core::key::LAYOUT),
];

/// What `main` hands the app: the config read at startup and where it lives.
pub struct Startup {
    pub path: PathBuf,
    pub config: Config,
    pub problems: Vec<Problem>,
    /// The user's theme files (D54).
    pub themes: Vec<ThemeText>,
}

/// The file written on first run: every setting and default key binding,
/// commented out.
pub fn template() -> String {
    core::template(&keymap::config_template())
}

/// Read the config file at startup, first writing the template if there's
/// none. Problems don't stop faro: they're shown once the UI is up.
#[expect(
    clippy::disallowed_methods,
    reason = "startup, before the UI: reads the config file"
)]
pub fn load(path: &Path) -> (Config, Vec<Problem>) {
    let defaults = Config::default();
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let parsed = core::parse(&text, &defaults);
            (parsed.config, parsed.problems)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => match create(path) {
            Ok(()) => {
                tracing::info!(path = %path.display(), "wrote the default config file");
                (defaults, Vec::new())
            }
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "couldn't write the default config file");
                let text = format!("couldn't create {}: {error}", path.display());
                (defaults, vec![Problem::warning(text)])
            }
        },
        Err(error) => {
            let text = format!("couldn't read {}: {error}", path.display());
            (defaults, vec![Problem::error(text)])
        }
    }
}

/// Write the template to a new file (`0600`, in a `0700` directory if it
/// has to be created).
#[expect(
    clippy::disallowed_methods,
    reason = "startup, before the UI: writes the default config file"
)]
fn create(path: &Path) -> io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    if let Some(dir) = parent(path) {
        paths::create_private_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(paths::FILE_MODE)
        .open(path)?;
    file.write_all(template().as_bytes())
}

fn parent(path: &Path) -> Option<&Path> {
    path.parent().filter(|p| !p.as_os_str().is_empty())
}

/// Where the user's theme files are: `themes/` beside the config file.
pub fn themes_dir(config: &Path) -> PathBuf {
    parent(config).unwrap_or(Path::new(".")).join("themes")
}

/// Whether `path` is a theme file's (by name; it may be gone already).
fn is_theme_file(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "json")
}

/// Read the theme files at startup, first creating the (empty, `0700`)
/// directory so it's there to put them in. A directory that can't be read
/// means no user themes, not a failure.
#[expect(
    clippy::disallowed_methods,
    reason = "startup, before the UI: reads the theme files"
)]
pub fn load_themes(dir: &Path) -> Vec<ThemeText> {
    if let Err(error) = paths::create_private_dir_all(dir) {
        tracing::warn!(%error, dir = %dir.display(), "couldn't create the themes directory");
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!(%error, dir = %dir.display(), "couldn't read the themes directory");
            return Vec::new();
        }
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_theme_file(path))
        .map(|path| theme_text(&path, std::fs::read_to_string(&path)))
        .collect()
}

/// Read the theme files again, after one changed.
async fn read_themes(dir: &Path) -> Vec<ThemeText> {
    let mut themes = Vec::new();
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return themes;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if is_theme_file(&path) {
            themes.push(theme_text(&path, tokio::fs::read_to_string(&path).await));
        }
    }
    themes
}

fn theme_text(path: &Path, read: io::Result<String>) -> ThemeText {
    ThemeText {
        file: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        text: read.map_err(|e| format!("couldn't read it: {e}")),
    }
}

/// Move preferences the database held before M5 into the config file, once:
/// a stored value that differs from the default is written to the file,
/// unless the file already sets that setting (the file wins); then it's
/// removed from the database. If the file can't be written, the value is used for this run
/// and the move is tried again next time.
pub async fn migrate(store: &Store, path: &Path, config: &mut Config) -> Vec<Problem> {
    let mut problems = Vec::new();
    for (old, setting) in MOVED {
        if let Err(error) = migrate_one(store, path, config, old, setting).await {
            tracing::warn!(%error, setting, "moving a preference to the config file failed");
            problems.push(Problem::warning(format!(
                "couldn't move {setting} to the config file: {error}"
            )));
        }
    }
    problems
}

async fn migrate_one(
    store: &Store,
    path: &Path,
    config: &mut Config,
    old: &str,
    setting: &'static str,
) -> Result<(), String> {
    let Some(stored) = store.app_state(old).await.map_err(|e| e.to_string())? else {
        return Ok(());
    };
    let value = match stored.as_str() {
        "on" => ConfigValue::Bool(true),
        "off" => ConfigValue::Bool(false),
        name => ConfigValue::Text(name.to_owned()),
    };
    let default = Config::default().value(setting);
    let set_in_file = match tokio::fs::read_to_string(path).await {
        Ok(text) => core::mentions(&text, setting),
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(format!("couldn't read {}: {error}", path.display())),
    };
    if !set_in_file && Some(&value) != default.as_ref() {
        // A value faro wrote itself; if it's somehow invalid, just drop it.
        if config.set(setting, &value).is_ok() {
            tracing::info!(
                setting,
                "moving a preference from the database to the config file"
            );
            save(
                path,
                &SettingEdit {
                    path: setting,
                    value,
                },
            )
            .await?;
        }
    }
    store.remove_app_state(old).await.map_err(|e| e.to_string())
}

/// Write one setting to the file, keeping everything else in it. A missing
/// file is created from the template first.
pub async fn save(path: &Path, edit: &SettingEdit) -> Result<(), String> {
    let shown = path.display();
    let (text, exists) = match tokio::fs::read_to_string(path).await {
        Ok(text) => (text, true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => (template(), false),
        Err(error) => return Err(format!("couldn't read {shown}: {error}")),
    };
    let edited = core::set(&text, edit.path, &edit.value).map_err(|e| e.to_string())?;
    if exists && edited == text {
        return Ok(());
    }
    if !exists {
        if let Some(dir) = parent(path) {
            tokio::fs::DirBuilder::new()
                .recursive(true)
                .mode(paths::DIR_MODE)
                .create(dir)
                .await
                .map_err(|e| format!("couldn't create {}: {e}", dir.display()))?;
        }
        tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(paths::FILE_MODE)
            .open(path)
            .await
            .map_err(|e| format!("couldn't create {shown}: {e}"))?;
    }
    // In place, not via a temporary file and a rename: the file may be a
    // symlink into a dotfiles repository, which a rename would replace.
    tokio::fs::write(path, edited)
        .await
        .map_err(|e| format!("couldn't write {shown}: {e}"))
}

/// Start the task that writes settings changed in the app, one at a time;
/// failures come back as `AppEvent::SettingNotSaved`. Dropping the sender
/// lets it finish what's queued; await the handle before exiting.
pub fn spawn_writer(
    path: PathBuf,
    results: mpsc::UnboundedSender<AppEvent>,
) -> (mpsc::UnboundedSender<SettingEdit>, JoinHandle<()>) {
    let (edits, mut queue) = mpsc::unbounded_channel::<SettingEdit>();
    let task = tokio::spawn(async move {
        while let Some(edit) = queue.recv().await {
            if let Err(error) = save(&path, &edit).await {
                tracing::warn!(%error, setting = edit.path, "saving a setting failed");
                // The app may be gone (quitting); nothing to tell then.
                let _ = results.send(AppEvent::SettingNotSaved(error));
            }
        }
    });
    (edits, task)
}

/// Start watching the config file and the theme files, or explain in
/// `problems` why changes won't apply.
pub fn try_watch(
    path: &Path,
    results: &mpsc::UnboundedSender<AppEvent>,
    problems: &mut Vec<Problem>,
) -> Vec<Watch> {
    let themes = themes_dir(path);
    [
        (watch(path, results.clone()), path),
        (watch_themes(&themes, results.clone()), themes.as_path()),
    ]
    .into_iter()
    .filter_map(|(watch, watched)| {
        watch
            .inspect_err(|error| {
                tracing::warn!(%error, watched = %watched.display(), "not watching");
                problems.push(Problem::warning(format!(
                    "changes to {} won't apply until restart: {error}",
                    watched.display()
                )));
            })
            .ok()
    })
    .collect()
}

/// Watching the config file or the themes; stops when dropped.
pub struct Watch {
    _watcher: notify::RecommendedWatcher,
    task: JoinHandle<()>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Watch `dir` (not recursively); `changed` picks the paths that matter.
/// Each burst of changes sends one ping.
fn watch_dir(
    dir: &Path,
    changed: impl Fn(&Path) -> bool + Send + 'static,
) -> notify::Result<(notify::RecommendedWatcher, mpsc::UnboundedReceiver<()>)> {
    let (ping, pings) = mpsc::unbounded_channel();
    let mut watcher =
        notify::recommended_watcher(move |event: notify::Result<notify::Event>| match event {
            // Not `Access`: reading the file (as the reload does) would
            // trigger another reload.
            Ok(event)
                if matches!(
                    event.kind,
                    EventKind::Any
                        | EventKind::Create(_)
                        | EventKind::Modify(_)
                        | EventKind::Remove(_)
                ) && event.paths.iter().any(|p| changed(p)) =>
            {
                // Runs on notify's thread; never blocks.
                let _ = ping.send(());
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "file watcher"),
        })?;
    watcher.watch(dir, RecursiveMode::NonRecursive)?;
    Ok((watcher, debounce(pings)))
}

/// One ping per burst: sent once nothing has changed for `DEBOUNCE`.
fn debounce(mut changes: mpsc::UnboundedReceiver<()>) -> mpsc::UnboundedReceiver<()> {
    let (quiet, settled) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while changes.recv().await.is_some() {
            loop {
                match tokio::time::timeout(DEBOUNCE, changes.recv()).await {
                    Ok(Some(())) => {}
                    Ok(None) => return,
                    Err(_quiet) => break,
                }
            }
            if quiet.send(()).is_err() {
                return;
            }
        }
    });
    settled
}

/// Watch the config file and send its text as `AppEvent::ConfigRead` after
/// each burst of changes. Watches the directory, so editors that save by
/// replacing the file are seen too.
pub fn watch(path: &Path, results: mpsc::UnboundedSender<AppEvent>) -> notify::Result<Watch> {
    let name = path.file_name().map(ToOwned::to_owned);
    let (watcher, pings) = watch_dir(parent(path).unwrap_or(Path::new(".")), move |p| {
        p.file_name() == name.as_deref()
    })?;
    let task = tokio::spawn(reload(path.to_owned(), pings, results));
    Ok(Watch {
        _watcher: watcher,
        task,
    })
}

/// Watch the theme files and send them all as `AppEvent::ThemesRead` after
/// each burst of changes to any of them.
pub fn watch_themes(dir: &Path, results: mpsc::UnboundedSender<AppEvent>) -> notify::Result<Watch> {
    let (watcher, mut pings) = watch_dir(dir, is_theme_file)?;
    let dir = dir.to_owned();
    let task = tokio::spawn(async move {
        while pings.recv().await.is_some() {
            let themes = read_themes(&dir).await;
            if results.send(AppEvent::ThemesRead(themes)).is_err() {
                return;
            }
        }
    });
    Ok(Watch {
        _watcher: watcher,
        task,
    })
}

async fn reload(
    path: PathBuf,
    mut changes: mpsc::UnboundedReceiver<()>,
    results: mpsc::UnboundedSender<AppEvent>,
) {
    while changes.recv().await.is_some() {
        let read = match tokio::fs::read_to_string(&path).await {
            Ok(text) => Ok(text),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                tracing::info!("the config file is gone; keeping the current settings");
                continue;
            }
            Err(error) => Err(format!("couldn't read {}: {error}", path.display())),
        };
        if results.send(AppEvent::ConfigRead(read)).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests;
