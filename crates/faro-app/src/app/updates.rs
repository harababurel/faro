//! Update checks (D148): opt-in (`[updates] check`), notify only. This is
//! what's known and decided; the binary asks GitHub
//! (`Effect::CheckForUpdate`) and the reducer (`update/updates.rs`) decides
//! when to ask and what to say.

use std::fmt;
use std::time::Duration;

use faro_core::rt::Instant;

/// Ask again this long after a check (and at most this often across runs).
pub const CHECK_INTERVAL: Duration = Duration::from_hours(12);
/// After a failed check (offline, rate-limited), try again this much later.
pub const RETRY_AFTER: Duration = Duration::from_hours(1);
/// A release is announced only once it's this old: time for the release
/// workflow to finish its uploads (`.deb`s, the Homebrew tap) and the AUR to
/// follow, and to pull a broken release before anyone is told about it.
pub const GRACE_SECS: i64 = 60 * 60;
/// How long the "faro x.y.z is available" notice stays in the footer.
pub const NOTICE_TIME: Duration = Duration::from_secs(10);

/// The repository releases come from (`https://github.com/<owner>/<repo>`).
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// A release version: `major.minor.patch`, nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    /// `1.2.3` or `v1.2.3`. Pre-releases (`1.2.3-dev`), build metadata and
    /// anything else are `None`.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        let mut parts = text.split('.');
        let mut number = || -> Option<u64> {
            let part = parts.next()?;
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            part.parse().ok()
        };
        let version = Self {
            major: number()?,
            minor: number()?,
            patch: number()?,
        };
        parts.next().is_none().then_some(version)
    }

    /// This build's version; `None` for a pre-release (`0.2.0-dev`), which
    /// never checks.
    pub fn current() -> Option<Self> {
        Self::parse(env!("CARGO_PKG_VERSION"))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The latest release on GitHub.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    /// When it was published, in seconds since the Unix epoch.
    pub published_at: i64,
}

impl Release {
    /// Its release notes.
    pub fn url(&self) -> String {
        format!("{REPOSITORY}/releases/tag/v{}", self.version)
    }

    /// As kept in `app_state`: `"<version> <published_at>"`.
    pub fn to_stored(self) -> String {
        format!("{} {}", self.version, self.published_at)
    }

    pub fn from_stored(text: &str) -> Option<Self> {
        let (version, published_at) = text.split_once(' ')?;
        Some(Self {
            version: Version::parse(version)?,
            published_at: published_at.trim().parse().ok()?,
        })
    }

    /// Newer than `current` and old enough to announce at `now` (seconds).
    pub fn announceable(&self, current: Version, now: i64) -> bool {
        self.version > current && now >= self.published_at.saturating_add(GRACE_SECS)
    }
}

/// Why update checks can't run here, if they can't (the palette dims the
/// setting with this reason).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    /// Built without the `update-check` feature (a distro package), or the
    /// web demo.
    NotBuilt,
    /// `faro demo` (D87).
    Demo,
    /// A pre-release build (`-dev`): there's nothing to compare with.
    Development,
}

impl Unavailable {
    pub const fn reason(self) -> &'static str {
        match self {
            Self::NotBuilt => "not in this build",
            Self::Demo => "not in the demo",
            Self::Development => "development build",
        }
    }
}

/// What the app knows about updates (`State::updates`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Updates {
    /// This binary can ask GitHub (built with `update-check`). The web demo
    /// and tests leave it off.
    pub built: bool,
    /// The newest release seen: from the last check, or remembered from the
    /// previous run.
    pub latest: Option<Release>,
    /// The release to tell the user about (newer, past the grace period),
    /// decided by the reducer so drawing needs no clock.
    pub available: Option<Release>,
    /// Already announced in this run, so it isn't repeated.
    pub announced: Option<Version>,
    /// When to ask next in this run; `None` while off, before the stored
    /// state is read, or while a check is in flight.
    pub next_check: Option<Instant>,
    /// Waiting for the stored state or for GitHub's answer.
    pub busy: bool,
}

impl crate::app::state::State {
    /// Why update checks can't run here, if they can't.
    pub fn update_checks_unavailable(&self) -> Option<Unavailable> {
        if self.is_demo() {
            Some(Unavailable::Demo)
        } else if !self.updates.built {
            Some(Unavailable::NotBuilt)
        } else if Version::current().is_none() {
            Some(Unavailable::Development)
        } else {
            None
        }
    }

    /// Update checks are on and can run.
    pub fn update_checks_active(&self) -> bool {
        self.config.updates.check && self.update_checks_unavailable().is_none()
    }

    /// The release to tell the user about (the footer marker, the logo
    /// screen, About and the palette), while update checks are on.
    pub fn update_available(&self) -> Option<&Release> {
        self.updates
            .available
            .as_ref()
            .filter(|_| self.update_checks_active())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn v(major: u64, minor: u64, patch: u64) -> Version {
        Version {
            major,
            minor,
            patch,
        }
    }

    #[test]
    fn versions_parse_with_or_without_a_v() {
        assert_eq!(Version::parse("0.2.0"), Some(v(0, 2, 0)));
        assert_eq!(Version::parse("v1.10.3"), Some(v(1, 10, 3)));
        assert_eq!(Version::parse(" v0.1.3\n"), Some(v(0, 1, 3)));
    }

    #[test]
    fn pre_releases_and_junk_are_not_versions() {
        for text in [
            "0.2.0-dev",
            "0.2.0-rc.1",
            "0.2.0+build",
            "0.2",
            "0.2.0.1",
            "v",
            "",
            "x.y.z",
            "0..1",
            "+1.2.3",
            "1.2.-3",
        ] {
            assert_eq!(Version::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn versions_compare_by_number_not_text() {
        assert!(v(0, 10, 0) > v(0, 9, 9));
        assert!(v(1, 0, 0) > v(0, 99, 99));
        assert!(v(0, 1, 4) > v(0, 1, 3));
    }

    #[test]
    fn the_current_version_is_this_crate_s() {
        let current = Version::current().map(|c| c.to_string());
        let expected = env!("CARGO_PKG_VERSION");
        if expected.contains('-') {
            assert_eq!(current, None);
        } else {
            assert_eq!(current.as_deref(), Some(expected));
        }
    }

    #[test]
    fn a_release_round_trips_through_app_state() {
        let release = Release {
            version: v(0, 2, 0),
            published_at: 1_760_000_000,
        };
        assert_eq!(release.to_stored(), "0.2.0 1760000000");
        assert_eq!(Release::from_stored(&release.to_stored()), Some(release));
        assert_eq!(Release::from_stored("0.2.0"), None);
        assert_eq!(Release::from_stored("0.2.0-dev 1"), None);
        assert_eq!(Release::from_stored("0.2.0 soon"), None);
    }

    #[test]
    fn a_release_is_announced_once_newer_and_past_the_grace_period() {
        let release = Release {
            version: v(0, 2, 0),
            published_at: 1_000_000,
        };
        let later = 1_000_000 + GRACE_SECS;
        assert!(
            !release.announceable(v(0, 1, 3), later - 1),
            "still in grace"
        );
        assert!(release.announceable(v(0, 1, 3), later));
        assert!(!release.announceable(v(0, 2, 0), later), "same version");
        assert!(!release.announceable(v(0, 3, 0), later), "older than ours");
    }

    #[test]
    fn release_notes_link_to_the_tag() {
        let release = Release {
            version: v(0, 2, 0),
            published_at: 0,
        };
        assert_eq!(release.url(), format!("{REPOSITORY}/releases/tag/v0.2.0"));
        assert!(
            REPOSITORY.starts_with("https://github.com/"),
            "{REPOSITORY}"
        );
    }
}
