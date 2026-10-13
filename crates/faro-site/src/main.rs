//! `faro-site`: the getfaro.dev site generator (M13 Phase 1, WEBSITE.md §6.1).
//!
//! `faro-site <template dir> <out dir> [--demo <pkg dir>]` copies
//! `<template dir>` to the output recursively, fills the `{{name}}`
//! placeholders in the `.html` and `.css` files from faro's own code
//! (version, date, commit, source link, key table, theme chips, poster,
//! OG image), and rewrites
//! asset references to content-hashed names. With `--demo`, the output of
//! `crates/faro-web/build.sh` is copied into `<out>/demo/` the same way
//! (Phase 4). `site/build.sh` runs it; an unknown or unfilled placeholder,
//! or any I/O error, exits non-zero with a message.

mod assets;
mod demo;
mod keys;
mod placeholders;
mod release;
mod scene;
mod site;
mod themes;

use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage: faro-site <template dir> <out dir> [--demo <wasm pkg dir>]";

#[expect(
    clippy::print_stderr,
    reason = "a one-shot CLI reports usage and errors on stderr"
)]
fn main() -> ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        eprintln!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let (positional, pkg) = match split(&args) {
        Ok(found) => found,
        Err(why) => {
            eprintln!("{why}\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    let [template, out] = positional.as_slice() else {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    };
    let (template, out) = (Path::new(template), Path::new(out));
    let pkg = pkg.map(Path::new);
    match site::generate(template, out, pkg) {
        Ok(()) => ExitCode::SUCCESS,
        Err(problem) => {
            eprintln!("faro-site: {problem}");
            ExitCode::FAILURE
        }
    }
}

/// The positional arguments, and the `<pkg dir>` after `--demo` if given.
fn split(
    args: &[std::ffi::OsString],
) -> Result<(Vec<&std::ffi::OsString>, Option<&std::ffi::OsStr>), String> {
    let mut positional = Vec::new();
    let mut pkg = None;
    let mut rest = args;
    while let Some((first, tail)) = rest.split_first() {
        rest = tail;
        if first == "--demo" {
            let Some((dir, tail)) = rest.split_first() else {
                return Err(
                    "--demo needs a directory: the output of crates/faro-web/build.sh".to_owned(),
                );
            };
            rest = tail;
            if pkg.is_some() {
                return Err("--demo given twice".to_owned());
            }
            pkg = Some(dir.as_os_str());
        } else {
            positional.push(first);
        }
    }
    Ok((positional, pkg))
}
