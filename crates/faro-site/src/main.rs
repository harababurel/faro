//! `faro-site`: the getfaro.dev site generator (M13 Phase 1, WEBSITE.md §6.1).
//!
//! `faro-site <template dir> <out dir>` copies `<template dir>` to the
//! output recursively, fills the `{{name}}` placeholders in the `.html`
//! and `.css` files from faro's own code (version, date, key table, theme
//! chips, poster, OG image), and rewrites asset references to
//! content-hashed names. `site/build.sh` runs it; an unknown or unfilled
//! placeholder, or any I/O error, exits non-zero with a message.

mod assets;
mod keys;
mod placeholders;
mod release;
mod scene;
mod site;
mod themes;

use std::process::ExitCode;

#[expect(
    clippy::print_stderr,
    reason = "a one-shot CLI reports usage and errors on stderr"
)]
fn main() -> ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let [template, out] = args.as_slice() else {
        eprintln!("usage: faro-site <template dir> <out dir>");
        return ExitCode::FAILURE;
    };
    let (template, out) = (std::path::Path::new(template), std::path::Path::new(out));
    match site::generate(template, out) {
        Ok(()) => ExitCode::SUCCESS,
        Err(problem) => {
            eprintln!("faro-site: {problem}");
            ExitCode::FAILURE
        }
    }
}
