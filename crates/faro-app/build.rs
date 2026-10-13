//! Build script: the version line faro shows (D97), as `FARO_VERSION`.
//!
//! `0.1.0-dev (8fe5755 2026-10-11)` from a git checkout (the commit's
//! short hash, `+` if tracked files had changes, and the commit's date), or
//! just `0.1.0` where there's no git (a release tarball, crates.io).
//! Packagers can set `FARO_BUILD_COMMIT` and `FARO_BUILD_DATE` instead.
#![expect(
    clippy::disallowed_methods,
    reason = "a build script runs before anything else and may wait for git"
)]

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=FARO_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=FARO_BUILD_DATE");
    let version = env_or("CARGO_PKG_VERSION", "");
    let (commit, date) = match (var("FARO_BUILD_COMMIT"), var("FARO_BUILD_DATE")) {
        (Some(commit), date) => (Some(commit), date),
        (None, _) => from_git(),
    };
    // The date on its own too, for code that wants `YYYY-MM-DD` alone
    // (the site generator's `{{date}}`, M13).
    println!(
        "cargo:rustc-env=FARO_BUILD_DATE={}",
        date.clone().unwrap_or_default()
    );
    // And the commit, for the site's source link (§3.9). `rustc-env` only
    // reaches the crate being compiled, so it can share the name of the
    // input variable `rerun-if-env-changed` watches.
    println!(
        "cargo:rustc-env=FARO_BUILD_COMMIT={}",
        commit.clone().unwrap_or_default()
    );
    let line = match (commit, date) {
        (Some(commit), Some(date)) => format!("{version} ({commit} {date})"),
        (Some(commit), None) => format!("{version} ({commit})"),
        (None, _) => version,
    };
    println!("cargo:rustc-env=FARO_VERSION={line}");
}

/// The commit and its date, if this crate is tracked in a git checkout.
fn from_git() -> (Option<String>, Option<String>) {
    // Not ours if this crate isn't tracked there (e.g. unpacked from
    // crates.io inside some other repository).
    if git(&["ls-files", "--error-unmatch", "Cargo.toml"]).is_none() {
        return (None, None);
    }
    // Rebuild the line when the commit moves or files change (for `+`).
    for path in ["HEAD", "index"] {
        if let Some(path) = git(&["rev-parse", "--git-path", path]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(top) = git(&["rev-parse", "--show-toplevel"]) {
        println!("cargo:rerun-if-changed={top}/crates");
        println!("cargo:rerun-if-changed={top}/Cargo.lock");
    }
    let Some(hash) = git(&["rev-parse", "--short=7", "HEAD"]) else {
        return (None, None);
    };
    let dirty = git(&["status", "--porcelain", "--untracked-files=no"])
        .is_some_and(|changes| !changes.is_empty());
    let commit = if dirty { format!("{hash}+") } else { hash };
    let date = git(&["log", "-1", "--date=short", "--format=%cd"]);
    (Some(commit), date)
}

/// A git command's trimmed output, or `None` if it failed.
fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn env_or(name: &str, default: &str) -> String {
    var(name).unwrap_or_else(|| default.to_owned())
}
