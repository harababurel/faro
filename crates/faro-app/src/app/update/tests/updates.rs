//! Update checks (D148): opt-in, at most one notice per release per run,
//! quiet on failure, and nothing at all while off.

use super::*;
use crate::app::event::UpdateEvent;
use crate::app::palette::{Setting, SettingChoice};
use crate::app::state::Notice;
use crate::app::update::updates as reducer;
use crate::app::updates::{CHECK_INTERVAL, GRACE_SECS, RETRY_AFTER, Release, Version};
use faro_core::config::Config;
use faro_core::store::Profile;

/// Far newer than any real version.
fn newer(published_at: i64) -> Release {
    Release::new(
        Version {
            major: 99,
            minor: 0,
            patch: 0,
        },
        published_at,
    )
}

fn newer_with_notes(published_at: i64, notes: &str) -> Release {
    Release::with_notes(
        Version {
            major: 99,
            minor: 0,
            patch: 0,
        },
        published_at,
        notes,
    )
}

fn now() -> i64 {
    reducer::now_secs()
}

/// A loaded state in a build that can check, with checks `on` or off.
fn state_with(on: bool) -> State {
    let mut state = loaded_state();
    state.updates.built = true;
    let mut config = Config::default();
    config.updates.check = on;
    configure(&mut state, config, vec![]);
    state
}

fn asks(effects: &[Effect]) -> bool {
    effects.contains(&Effect::CheckForUpdate)
}

fn notice_text(state: &State) -> String {
    state
        .notice
        .as_ref()
        .map(|n| n.text.clone())
        .unwrap_or_default()
}

#[test]
fn off_by_default_nothing_is_read_or_asked() {
    let mut state = loaded_state();
    state.updates.built = true;
    let effects = configure(&mut state, Config::default(), vec![]);
    assert!(!effects.contains(&Effect::LoadUpdateState));
    assert!(!asks(&effects));
    assert_eq!(state.updates.next_check, None);
}

#[test]
fn turned_on_at_start_reads_what_the_last_run_knew() {
    let mut state = loaded_state();
    state.updates.built = true;
    let mut config = Config::default();
    config.updates.check = true;
    let effects = configure(&mut state, config, vec![]);
    assert!(effects.contains(&Effect::LoadUpdateState));
    assert!(!asks(&effects), "the stored state decides whether to ask");
}

#[test]
fn a_recent_check_is_not_repeated_but_its_release_is_announced_at_launch() {
    let mut state = state_with(true);
    let effects = update(
        &mut state,
        AppEvent::Updates(UpdateEvent::StateLoaded {
            checked_at: Some(now() - 60),
            latest: Some(newer(0)),
        }),
    );
    assert!(!asks(&effects));
    assert!(
        notice_text(&state).starts_with("faro 99.0.0 is available"),
        "{}",
        notice_text(&state)
    );
    assert_eq!(state.update_available(), Some(&newer(0)));
    let next = state
        .updates
        .next_check
        .expect("the next check is scheduled");
    let wait = next.saturating_duration_since(Instant::now());
    assert!(
        wait <= CHECK_INTERVAL && wait > CHECK_INTERVAL / 2,
        "{wait:?}"
    );
}

#[test]
fn an_old_check_or_none_at_all_asks_github_now() {
    for checked_at in [None, Some(now() - 13 * 3600), Some(now() + 3600)] {
        let mut state = state_with(true);
        let effects = update(
            &mut state,
            AppEvent::Updates(UpdateEvent::StateLoaded {
                checked_at,
                latest: None,
            }),
        );
        assert!(asks(&effects), "{checked_at:?}");
        assert!(state.updates.busy);
    }
}

#[test]
fn a_new_release_is_saved_and_announced_once_per_run() {
    let mut state = state_with(true);
    let effects = update(
        &mut state,
        AppEvent::Updates(UpdateEvent::Checked(Ok(newer(0)))),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::SaveUpdateState { latest, .. }] if *latest == newer(0)
    ));
    assert!(notice_text(&state).contains("99.0.0"));
    state.notice = None;
    update(
        &mut state,
        AppEvent::Updates(UpdateEvent::Checked(Ok(newer(0)))),
    );
    assert_eq!(state.notice, None, "not again in the same run");
    assert_eq!(
        state.update_available(),
        Some(&newer(0)),
        "the marker stays"
    );
}

#[test]
fn a_release_is_announced_only_after_the_grace_period() {
    let mut state = state_with(true);
    update(
        &mut state,
        AppEvent::Updates(UpdateEvent::Checked(Ok(newer(now())))),
    );
    assert_eq!(state.update_available(), None);
    assert_eq!(state.notice, None);
    let next = state.updates.next_check.expect("scheduled");
    let wait = next.saturating_duration_since(Instant::now());
    assert!(
        wait <= CHECK_INTERVAL,
        "checked again by the next interval at the latest: {wait:?}"
    );
    // Published just over the grace period ago: announced.
    update(
        &mut state,
        AppEvent::Updates(UpdateEvent::Checked(Ok(newer(now() - GRACE_SECS - 60)))),
    );
    assert!(state.update_available().is_some());
}

#[test]
fn the_grace_period_ending_brings_the_next_check_forward() {
    let mut state = state_with(true);
    // Half an hour left of the grace period.
    let published = now() - GRACE_SECS + 30 * 60;
    update(
        &mut state,
        AppEvent::Updates(UpdateEvent::Checked(Ok(newer(published)))),
    );
    let next = state.updates.next_check.expect("scheduled");
    let wait = next.saturating_duration_since(Instant::now());
    assert!(
        wait <= Duration::from_secs(30 * 60 + 5) && wait > Duration::from_mins(15),
        "{wait:?}"
    );
}

#[test]
fn a_failed_check_is_quiet_and_tried_again_in_an_hour() {
    let mut state = state_with(true);
    let effects = update(
        &mut state,
        AppEvent::Updates(UpdateEvent::Checked(Err("offline".to_owned()))),
    );
    assert_eq!(effects, vec![]);
    assert_eq!(state.notice, None);
    let next = state.updates.next_check.expect("retried");
    let wait = next.saturating_duration_since(Instant::now());
    assert!(wait <= RETRY_AFTER && wait > RETRY_AFTER / 2, "{wait:?}");
}

#[test]
fn a_warning_is_not_covered_by_the_announcement() {
    let mut state = state_with(true);
    state.notice = Some(Notice::warning("Config: something"));
    update(
        &mut state,
        AppEvent::Updates(UpdateEvent::Checked(Ok(newer(0)))),
    );
    assert_eq!(notice_text(&state), "Config: something");
    assert!(state.update_available().is_some(), "the marker shows it");
}

#[test]
fn the_tick_asks_when_the_next_check_is_due() {
    let mut state = state_with(true);
    // As after the stored state was read.
    state.updates.busy = false;
    state.updates.next_check = Some(Instant::now() + Duration::from_secs(3600));
    assert!(!asks(&update(&mut state, AppEvent::Tick)), "not yet");
    state.updates.next_check = Some(Instant::now());
    assert!(asks(&update(&mut state, AppEvent::Tick)));
    assert_eq!(state.updates.next_check, None, "until the answer comes");
    assert!(!asks(&update(&mut state, AppEvent::Tick)), "one at a time");
}

#[test]
fn turning_it_on_in_the_palette_asks_now_and_off_hides_everything() {
    let mut state = state_with(false);
    let effects = choose(&mut state, SettingChoice::UpdateCheck(true));
    assert!(asks(&effects));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::SaveSetting(edit) if edit.path == "updates.check"))
    );
    update(
        &mut state,
        AppEvent::Updates(UpdateEvent::Checked(Ok(newer(0)))),
    );
    assert!(state.update_available().is_some());

    let effects = choose(&mut state, SettingChoice::UpdateCheck(false));
    assert!(!asks(&effects));
    assert_eq!(state.update_available(), None);
    assert_eq!(state.updates.next_check, None);
    assert_eq!(state.next_deadline().filter(|_| false), None);
}

#[test]
fn an_answer_after_turning_it_off_is_ignored() {
    let mut state = state_with(false);
    choose(&mut state, SettingChoice::UpdateCheck(true));
    choose(&mut state, SettingChoice::UpdateCheck(false));
    state.notice = None;
    let effects = update(
        &mut state,
        AppEvent::Updates(UpdateEvent::Checked(Ok(newer(0)))),
    );
    assert_eq!(effects, vec![]);
    assert_eq!(state.notice, None);
    assert_eq!(state.updates.next_check, None);
}

#[test]
fn the_demo_and_builds_without_it_never_check() {
    let mut state = loaded_state();
    let mut config = Config::default();
    config.updates.check = true;
    let effects = configure(&mut state, config.clone(), vec![]);
    assert!(!effects.contains(&Effect::LoadUpdateState), "not built");
    assert_eq!(
        Setting::UpdateCheck.unavailable(&state),
        Some("not in this build")
    );

    let mut state = loaded_state();
    state.updates.built = true;
    state.profile = Profile::Demo;
    let effects = configure(&mut state, config, vec![]);
    assert!(!effects.contains(&Effect::LoadUpdateState), "the demo");
    assert_eq!(
        Setting::UpdateCheck.unavailable(&state),
        Some("not in the demo")
    );
}

#[test]
fn release_notes_open_the_tag_s_page() {
    let mut state = state_with(true);
    update(
        &mut state,
        AppEvent::Updates(UpdateEvent::Checked(Ok(newer(0)))),
    );
    state.env.over_ssh = false;
    let effects = reducer::release_notes(&mut state);
    assert_eq!(
        effects,
        vec![Effect::OpenUrl(format!(
            "{}/releases/tag/v99.0.0",
            crate::app::updates::REPOSITORY
        ))]
    );
}

#[test]
fn cached_release_notes_and_digest_open_in_the_palette_and_respect_opt_out() {
    use crate::app::palette::{ItemKind, Level, PaletteCommand};
    use crate::app::state::PaletteGeometry;
    use faro_core::config::TranscribeMode;
    use ratatui::layout::Rect;

    let mut state = state_with(true);
    // User customized sidebar_width and local voice-note transcription, and
    // upgraded from 0.1.7 stored in `updates.latest`.
    state.config.ui.sidebar_width = 40;
    state.config.voice.transcribe = TranscribeMode::Local;
    let v017 = Version {
        major: 0,
        minor: 1,
        patch: 7,
    };
    update(
        &mut state,
        AppEvent::Updates(UpdateEvent::StateLoaded {
            checked_at: Some(now() - 60),
            latest: Some(Release::new(v017, 0)),
        }),
    );
    assert_eq!(state.updates.since_version, Some(v017));
    assert_eq!(state.notice, None, "no second notice channel on upgrade");

    // GitHub returns a newer release with markdown tag body notes.
    let release = newer_with_notes(
        0,
        "## What's Changed\n\
         - Added `sidebar_width` double-click reset on the border\n\
         - Improved group sync performance\n",
    );
    let effects = update(
        &mut state,
        AppEvent::Updates(UpdateEvent::Checked(Ok(release.clone()))),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::SaveUpdateState { latest, .. }] if latest.notes == release.notes
    ));

    // Running ReleaseNotes opens `Level::ReleaseNotes` in the palette.
    let effects = reducer::release_notes(&mut state);
    assert_eq!(effects, vec![]);
    let palette = state.palette.as_ref().expect("palette opened");
    assert_eq!(palette.level(), Level::ReleaseNotes);
    let titles: Vec<&str> = palette.items.iter().map(|i| i.title.as_str()).collect();
    assert!(
        titles.contains(&"What's new for you · since 0.1.7"),
        "{titles:?}"
    );
    assert!(titles.contains(&"faro 99.0.0 · available"), "{titles:?}");
    assert!(
        titles
            .iter()
            .any(|t| t.contains("Added sidebar_width double-click reset")),
        "{titles:?}"
    );
    assert!(
        titles.contains(&"Resizable conversation list via drag or keys"),
        "personalized digest includes sidebar_width change since 0.1.7: {titles:?}"
    );
    assert!(
        palette
            .items
            .iter()
            .any(|i| i.kind == ItemKind::Command(PaletteCommand::OpenReleaseUrl)),
        "includes GitHub release page action"
    );

    // Turning update checks off hides the cached remote release notes while
    // keeping offline release notes accessible.
    choose(&mut state, SettingChoice::UpdateCheck(false));
    state.palette = None;
    reducer::release_notes(&mut state);
    let palette = state.palette.as_ref().expect("offline notes open");
    assert!(
        !palette.items.iter().any(|i| i.title.contains("99.0.0")),
        "opt-out hides cached remote release"
    );
    assert!(
        palette
            .items
            .iter()
            .any(|i| i.title.starts_with("faro 0.1.10")),
        "offline bundled notes remain available"
    );

    // If the terminal is too small for legible notes (< 80x24 palette box),
    // `release_notes` falls back to opening/copying the tag URL.
    choose(&mut state, SettingChoice::UpdateCheck(true));
    update(
        &mut state,
        AppEvent::Updates(UpdateEvent::Checked(Ok(release))),
    );
    state.env.over_ssh = true;
    state.geometry.palette = Some(PaletteGeometry {
        area: Rect::new(0, 0, 48, 8),
        list: Rect::new(1, 3, 46, 4),
        offset: 0,
    });
    let effects = reducer::release_notes(&mut state);
    assert_eq!(
        effects,
        vec![Effect::Copy(format!(
            "{}/releases/tag/v99.0.0",
            crate::app::updates::REPOSITORY
        ))]
    );
}
