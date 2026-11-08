//! Update checks (D148): when to ask GitHub, and what to tell the user.
//! Opt-in (`[updates] check`); notify only. One footer notice per release
//! per run (at launch, or when a check in a long session finds it), then a
//! quiet marker; never the bell, a desktop notification or a popup.

use std::time::Duration;

use faro_core::rt::Instant;

use crate::app::event::{Effect, UpdateEvent};
use crate::app::state::{Notice, NoticeLevel, State};
use crate::app::updates::{CHECK_INTERVAL, GRACE_SECS, NOTICE_TIME, RETRY_AFTER, Release, Version};

/// Seconds since the Unix epoch, now.
pub fn now_secs() -> i64 {
    faro_core::rt::since_unix_epoch().map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// What update checks heard back.
pub fn on_event(state: &mut State, event: UpdateEvent) -> Vec<Effect> {
    let clocks = (now_secs(), Instant::now());
    match event {
        UpdateEvent::StateLoaded { checked_at, latest } => {
            on_state_loaded(state, checked_at, latest, clocks)
        }
        UpdateEvent::Checked(result) => on_checked(state, result, clocks),
    }
}

/// At startup: read what the last run knew, if checks are on.
pub fn start(state: &mut State) -> Vec<Effect> {
    if !state.update_checks_active() {
        return vec![];
    }
    state.updates.busy = true;
    vec![Effect::LoadUpdateState]
}

/// `[updates] check` changed (the palette, the file, `Ctrl+R`): on asks
/// now; off stops asking and hides what was found.
pub fn on_setting_changed(state: &mut State, was_on: bool) -> Vec<Effect> {
    let on = state.config.updates.check;
    if on == was_on {
        return vec![];
    }
    if !on {
        state.updates.next_check = None;
        state.updates.available = None;
        return vec![];
    }
    if !state.update_checks_active() || state.updates.busy {
        return vec![];
    }
    ask(state)
}

/// The last run's answer: announce what it found, then ask GitHub unless
/// the last check was less than `CHECK_INTERVAL` ago.
fn on_state_loaded(
    state: &mut State,
    checked_at: Option<i64>,
    latest: Option<Release>,
    (now, instant): (i64, Instant),
) -> Vec<Effect> {
    state.updates.busy = false;
    if !state.update_checks_active() {
        return vec![];
    }
    if state.updates.latest.is_none() {
        if let (Some(prev), Some(current)) = (latest.as_ref(), Version::current())
            && prev.version < current
        {
            state.updates.since_version = Some(prev.version);
        }
        state.updates.latest = latest;
    }
    evaluate(state, now);
    let interval = i64::try_from(CHECK_INTERVAL.as_secs()).unwrap_or(i64::MAX);
    match checked_at.map(|t| now.saturating_sub(t)) {
        // A clock that went backwards (negative) asks again.
        Some(since) if (0..interval).contains(&since) => {
            let left = Duration::from_secs(u64::try_from(interval - since).unwrap_or(0));
            schedule(state, left, (now, instant));
            vec![]
        }
        Some(_) | None => ask(state),
    }
}

/// GitHub answered: remember a release and announce it if it's new; a
/// failure is only logged and tried again later.
fn on_checked(
    state: &mut State,
    result: Result<Release, String>,
    (now, instant): (i64, Instant),
) -> Vec<Effect> {
    state.updates.busy = false;
    if !state.update_checks_active() {
        return vec![];
    }
    match result {
        Ok(release) => {
            tracing::info!(latest = %release.version, "update check done");
            state.updates.latest = Some(release.clone());
            evaluate(state, now);
            schedule(state, CHECK_INTERVAL, (now, instant));
            vec![Effect::SaveUpdateState {
                checked_at: now,
                latest: release,
            }]
        }
        Err(error) => {
            tracing::debug!(%error, "update check failed");
            schedule(state, RETRY_AFTER, (now, instant));
            vec![]
        }
    }
}

/// A deadline passed: ask if it's time.
pub fn on_tick(state: &mut State, instant: Instant) -> Vec<Effect> {
    let due = state.updates.next_check.is_some_and(|t| instant >= t);
    if !due || state.updates.busy || !state.update_checks_active() {
        return vec![];
    }
    ask(state)
}

fn ask(state: &mut State) -> Vec<Effect> {
    state.updates.next_check = None;
    state.updates.busy = true;
    vec![Effect::CheckForUpdate]
}

/// Ask again after `after`, or sooner when a known release's grace period
/// ends (so it's announced then, not up to `CHECK_INTERVAL` later).
fn schedule(state: &mut State, after: Duration, (now, instant): (i64, Instant)) {
    let grace_left = state
        .updates
        .latest
        .as_ref()
        .filter(|r| Version::current().is_some_and(|current| r.version > current))
        .and_then(|r| u64::try_from(r.published_at.saturating_add(GRACE_SECS) - now).ok())
        .filter(|&secs| secs > 0)
        .map(Duration::from_secs);
    let wait = grace_left.map_or(after, |grace| grace.min(after));
    state.updates.next_check = Some(instant + wait);
}

/// Decide what to show for the latest release; the first time a release is
/// shown in this run, say so in the footer (unless a warning or an error is
/// there: the marker stays either way).
fn evaluate(state: &mut State, now: i64) {
    let Some(current) = Version::current() else {
        return;
    };
    state.updates.available = state
        .updates
        .latest
        .clone()
        .filter(|r| r.announceable(current, now));
    let Some(release) = state.updates.available.as_ref() else {
        return;
    };
    let version = release.version;
    if state.updates.announced == Some(version) {
        return;
    }
    state.updates.announced = Some(version);
    if state
        .notice
        .as_ref()
        .is_some_and(|n| n.level != NoticeLevel::Info)
    {
        return;
    }
    state.notice = Some(Notice {
        text: format!(
            "faro {version} is available (you have {current}); release notes in Ctrl+P › Commands",
        ),
        level: NoticeLevel::Info,
        until: Some(Instant::now() + NOTICE_TIME),
    });
}

/// `PaletteCommand::ReleaseNotes`: render release notes and the "what's new
/// since you" digest in the palette (`Level::ReleaseNotes`), or fall back to
/// opening/copying the GitHub tag URL when the terminal is too small for
/// legible notes (< 80×24) or when a remote release has no cached notes.
pub fn release_notes(state: &mut State) -> Vec<Effect> {
    let cramped = state
        .geometry
        .palette
        .is_some_and(|g| g.area.width < 60 || g.list.height < 5);
    if cramped {
        return open_release_url(state);
    }
    if let Some(release) = state.update_available()
        && release.notes.trim().is_empty()
        && !crate::app::updates::has_bundled_notes(release.version)
    {
        return open_release_url(state);
    }
    super::palette::open(state, crate::app::palette::Level::ReleaseNotes);
    vec![]
}

/// `PaletteCommand::OpenReleaseUrl`: open (or copy, over SSH) the GitHub tag
/// page for the available update, or for this build's version.
pub fn open_release_url(state: &mut State) -> Vec<Effect> {
    let url = state
        .update_available()
        .map(Release::url)
        .or_else(|| Version::current().map(Release::url_for));
    let Some(url) = url else {
        return vec![];
    };
    super::selection::open_shared_url(state, url)
}
