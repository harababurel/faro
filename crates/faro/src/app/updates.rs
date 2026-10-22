//! Asking GitHub for the latest release (D148), off the UI thread. Only
//! runs when the user turned `[updates] check` on; the reducer decides when.
//! The request carries nothing but faro's version in the `User-Agent`.

use tokio::sync::mpsc::UnboundedSender;

use crate::app::event::{AppEvent, UpdateEvent};
#[cfg(feature = "update-check")]
use faro_app::app::updates::{Release, Version};

/// Whether this binary can check (`State::updates.built`).
pub const BUILT: bool = cfg!(feature = "update-check");

/// Ask, and send the answer back as `AppEvent::UpdateChecked`.
#[cfg(feature = "update-check")]
pub fn check(results: &UnboundedSender<AppEvent>) {
    let results = results.clone();
    tokio::spawn(async move {
        let _ = results.send(AppEvent::Updates(UpdateEvent::Checked(latest().await)));
    });
}

/// Built without update checks: the reducer never asks, but answer anyway.
#[cfg(not(feature = "update-check"))]
pub fn check(results: &UnboundedSender<AppEvent>) {
    let answer = Err("built without update checks".to_owned());
    let _ = results.send(AppEvent::Updates(UpdateEvent::Checked(answer)));
}

/// GitHub's API for a repository's latest release (drafts and pre-releases
/// are never "latest").
#[cfg(feature = "update-check")]
fn api_url() -> String {
    let repository = faro_app::app::updates::REPOSITORY;
    let path = repository
        .strip_prefix("https://github.com/")
        .unwrap_or(repository);
    format!("https://api.github.com/repos/{path}/releases/latest")
}

/// The answer is a few kilobytes; anything over this isn't GitHub's.
#[cfg(feature = "update-check")]
const MAX_BODY: usize = 1 << 20;

#[cfg(feature = "update-check")]
async fn latest() -> Result<Release, String> {
    // libsignal-service installs the same provider when it connects; install
    // it here too in case update checks run first.
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    let client = reqwest::Client::builder()
        .user_agent(concat!("faro/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| format!("setting up: {e}"))?;
    let mut response = client
        .get(api_url())
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|e| format!("asking GitHub: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("GitHub answered {status}"));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("reading GitHub's answer: {e}"))?
    {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_BODY {
            return Err("GitHub's answer is too large".to_owned());
        }
    }
    parse(&body)
}

/// The release in GitHub's answer: its tag and when it was published.
#[cfg(feature = "update-check")]
fn parse(body: &[u8]) -> Result<Release, String> {
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| format!("GitHub's answer isn't JSON: {e}"))?;
    let flag = |name: &str| value.get(name).and_then(serde_json::Value::as_bool) == Some(true);
    if flag("draft") || flag("prerelease") {
        return Err("the latest release is a draft or a pre-release".to_owned());
    }
    let tag = value
        .get("tag_name")
        .and_then(serde_json::Value::as_str)
        .ok_or("no tag_name in GitHub's answer")?;
    let version = Version::parse(tag).ok_or_else(|| format!("{tag:?} isn't a release version"))?;
    let published = value
        .get("published_at")
        .and_then(serde_json::Value::as_str)
        .ok_or("no published_at in GitHub's answer")?;
    let published_at = published
        .parse::<jiff::Timestamp>()
        .map_err(|e| format!("published_at {published:?}: {e}"))?
        .as_second();
    Ok(Release {
        version,
        published_at,
    })
}

#[cfg(test)]
#[cfg(feature = "update-check")]
mod tests {
    use super::*;

    #[test]
    fn github_s_answer_gives_the_version_and_date() {
        let body = br#"{"tag_name":"v0.2.0","name":"faro 0.2.0","draft":false,
            "prerelease":false,"published_at":"2026-10-20T09:30:00Z","assets":[]}"#;
        let release = parse(body).unwrap();
        assert_eq!(release.version.to_string(), "0.2.0");
        assert_eq!(
            release.published_at,
            "2026-10-20T09:30:00Z"
                .parse::<jiff::Timestamp>()
                .unwrap()
                .as_second()
        );
    }

    #[test]
    fn drafts_pre_releases_and_odd_answers_are_errors() {
        for body in [
            &br#"{"tag_name":"v0.2.0","draft":true,"published_at":"2026-10-20T09:30:00Z"}"#[..],
            br#"{"tag_name":"v0.2.0","prerelease":true,"published_at":"2026-10-20T09:30:00Z"}"#,
            br#"{"tag_name":"v0.2.0-rc.1","published_at":"2026-10-20T09:30:00Z"}"#,
            br#"{"tag_name":"v0.2.0"}"#,
            br#"{"tag_name":"v0.2.0","published_at":"yesterday"}"#,
            br#"{"message":"API rate limit exceeded"}"#,
            b"<html>",
        ] {
            assert!(parse(body).is_err(), "{}", String::from_utf8_lossy(body));
        }
    }

    #[test]
    fn it_asks_this_repository() {
        assert_eq!(
            api_url(),
            "https://api.github.com/repos/harababurel/faro/releases/latest"
        );
    }

    /// The real request (network): `cargo test -p faro -- --ignored asks_github`.
    #[expect(
        clippy::disallowed_methods,
        reason = "#[tokio::test] expands to block_on; tests may block"
    )]
    #[tokio::test]
    #[ignore = "talks to GitHub"]
    async fn asks_github_for_real() {
        let release = latest().await.unwrap();
        assert!(release.published_at > 1_760_000_000, "{release:?}");
    }
}
