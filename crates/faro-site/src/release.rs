//! The version and date the page shows, from faro-app's own build (D97),
//! so the facts line can't drift from the app.

/// The plain version from faro-app's version line: `0.1.0-dev`, without
/// the commit hash and date that `version::LINE` adds.
pub fn version() -> String {
    let line = faro_app::version::LINE;
    if let Some((version, _)) = line.split_once('(') {
        version.trim_end().to_owned()
    } else {
        line.to_owned()
    }
}

/// The commit date as `YYYY-MM-DD`: `version::BUILD_DATE` when the build
/// knew it (a git checkout, or `FARO_BUILD_DATE` as `build.rs` accepts),
/// today's date otherwise.
pub fn date() -> Result<String, String> {
    let date = faro_app::version::BUILD_DATE;
    if !date.is_empty() {
        return Ok(date.to_owned());
    }
    let since = faro_core::rt::since_unix_epoch().ok_or_else(|| {
        "no commit date: build from a git checkout or set FARO_BUILD_DATE".to_owned()
    })?;
    let days = i64::try_from(since.as_secs() / 86_400)
        .map_err(|_| "the clock is too far from 1970 to name a date".to_owned())?;
    Ok(civil_date(days))
}

/// The commit as `build.rs` knows it: `8fe5755`, with a trailing `+` when
/// the tree was dirty. The site's source link needs it (§3.9), so a build
/// without one is a problem.
pub fn commit() -> Result<String, String> {
    named_commit(faro_app::version::BUILD_COMMIT)
}

/// The GitHub tree URL for the exact build: GitHub doesn't know a dirty
/// marker, so the URL carries the commit without its `+` (§3.9).
pub fn source_url() -> Result<String, String> {
    Ok(tree_url(&commit()?))
}

fn named_commit(commit: &str) -> Result<String, String> {
    if commit.is_empty() {
        return Err("no commit: build from a git checkout or set FARO_BUILD_COMMIT".to_owned());
    }
    Ok(commit.to_owned())
}

fn tree_url(commit: &str) -> String {
    format!(
        "https://github.com/harababurel/faro/tree/{}",
        commit.trim_end_matches('+')
    )
}

/// `YYYY-MM-DD` for days since the Unix epoch: the days-from-civil
/// conversion (Howard Hinnant's algorithm), so one date doesn't need a
/// calendar dependency. The epoch shift makes every division land on a
/// non-negative number.
fn civil_date(days: i64) -> String {
    let shifted = days + 719_468;
    let era = (if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    }) / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_probe = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_probe + 2) / 5 + 1;
    let month = if month_probe < 10 {
        month_probe + 3
    } else {
        month_probe - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_the_plain_crate_version() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn a_missing_commit_is_named() {
        assert_eq!(
            named_commit(""),
            Err("no commit: build from a git checkout or set FARO_BUILD_COMMIT".to_owned())
        );
        assert_eq!(named_commit("c05a3bd+").as_deref(), Ok("c05a3bd+"));
    }

    #[test]
    fn the_source_url_drops_the_dirty_marker() {
        assert_eq!(
            tree_url("c05a3bd"),
            "https://github.com/harababurel/faro/tree/c05a3bd"
        );
        assert_eq!(
            tree_url("c05a3bd+"),
            "https://github.com/harababurel/faro/tree/c05a3bd"
        );
    }

    #[test]
    fn civil_dates_match_the_known_ones() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(1_767_225_600 / 86_400), "2026-01-01");
        assert_eq!(civil_date(951_782_400 / 86_400), "2000-02-29");
        assert_eq!(civil_date(1_735_689_599 / 86_400), "2024-12-31");
        assert_eq!(civil_date(-1), "1969-12-31");
    }
}
