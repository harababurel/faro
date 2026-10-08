//! The real Signal backend from the command line (M7): `faro link`, `faro ids`
//! and opening the linked account for the TUI. Runs before the TUI takes the
//! terminal, so it prints to stdout.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;

use anyhow::Context as _;
use faro_core::backend::BackendEvents;
use faro_presage::PresageBackend;

/// Link this computer as a new device, showing the QR code to scan.
pub async fn link(data_dir: &Path, device_name: Option<String>) -> anyhow::Result<()> {
    let file = faro_presage::store_file(data_dir);
    let store = faro_presage::open_store(&file)
        .await
        .with_context(|| format!("opening {}", file.display()))?;
    if faro_presage::is_linked(&store).await {
        anyhow::bail!(
            "{} is already linked to a Signal account. To link again, unlink \"faro\" in \
             Signal on your phone (Settings › Linked devices), then delete {} and run \
             `faro link` again",
            data_dir.display(),
            file.display()
        );
    }
    let name = device_name.unwrap_or_else(|| {
        let host = gethostname::gethostname();
        faro_presage::default_device_name(host.to_str())
    });
    let linked = faro_presage::link(store, name.clone(), |url| {
        let qr = qr_code(url.as_str(), qr_colors()).unwrap_or_default();
        say(&format!(
            "\nOn your phone, open Signal › Settings › Linked devices › Link new device, \
             and scan this code:\n\n{qr}\nThe device will be called \"{name}\". Waiting…\n"
        ));
    })
    .await
    .context("linking failed")?;
    let aci = linked.registration_data().service_ids.aci;
    say(&format!(
        "\nLinked. This account's Signal ID is {aci}.\n\
         Next: `faro ids --username <your username>` prints the IDs for an allowlist.\n"
    ));
    Ok(())
}

/// Print the IDs an allowlist needs: this account's, those of the given
/// usernames, and the groups presage knows (titles and IDs, never members or
/// messages). Groups are learned from messages, so send one in the group
/// from the phone first; this catches up with queued messages before listing.
pub async fn ids(data_dir: &Path, usernames: &[String]) -> anyhow::Result<()> {
    let linked = load(data_dir).await?;
    let report = Box::pin(faro_presage::ids(linked, usernames))
        .await
        .context("looking up IDs")?;
    let mut out = format!("This account: {}\n", report.me);
    for (name, id) in &report.usernames {
        match id {
            Some(id) => writeln!(out, "Username {name}: {id}"),
            None => writeln!(out, "Username {name}: not found"),
        }?;
    }
    if report.groups.is_empty() {
        out.push_str(
            "No groups known yet. Send a message in the group from your phone, then run this again.\n",
        );
    }
    for (title, id) in &report.groups {
        writeln!(out, "Group \"{title}\": {id}")?;
    }
    out.push_str("\nAn allowlist file looks like this (keep only who faro may talk to):\n\n");
    let people: Vec<String> = report
        .usernames
        .iter()
        .filter_map(|(name, id)| id.map(|id| format!("  \"{id}\", # {name}")))
        .collect();
    let groups: Vec<String> = report
        .groups
        .iter()
        .map(|(title, id)| format!("  \"{id}\", # {title}"))
        .collect();
    writeln!(
        out,
        "people = [\n{}\n]\ngroups = [\n{}\n]",
        people.join("\n"),
        groups.join("\n")
    )?;
    say(&out);
    Ok(())
}

/// The linked account, or an error saying to run `faro link`.
pub async fn load(data_dir: &Path) -> anyhow::Result<faro_presage::Linked> {
    let file = faro_presage::store_file(data_dir);
    let store = faro_presage::open_store(&file)
        .await
        .with_context(|| format!("opening {}", file.display()))?;
    Ok(faro_presage::load(store).await?)
}

/// Start the backend on the linked account.
pub async fn start(data_dir: &Path) -> anyhow::Result<(PresageBackend, BackendEvents)> {
    Ok(faro_presage::start(load(data_dir).await?))
}

#[expect(
    clippy::print_stdout,
    reason = "`faro link` and `faro ids` talk on stdout before any TUI exists"
)]
fn say(text: &str) {
    print!("{text}");
    let _ = std::io::stdout().flush();
}

/// Exact black and white where the terminal shows RGB: themes may redefine
/// every palette entry (base16 themes make colour 16 orange), which phones
/// can't read. Elsewhere the basic ANSI black and bright white.
fn qr_colors() -> &'static str {
    use crate::theme::ColorDepth;
    match ColorDepth::detect(|name| std::env::var(name).ok()) {
        ColorDepth::Truecolor => "\x1b[38;2;255;255;255;48;2;0;0;0m",
        ColorDepth::Indexed256 | ColorDepth::Ansi16 => "\x1b[97;40m",
    }
}

/// A QR code drawn with half blocks, two modules per character cell, dark
/// on light (phones don't read inverted codes reliably), with the standard
/// 4-module quiet zone. `colors` sets light foreground and dark background.
fn qr_code(data: &str, colors: &str) -> Option<String> {
    const QUIET: usize = 4;
    let code = qrcode::QrCode::new(data.as_bytes()).ok()?;
    let width = code.width();
    let modules = code.to_colors();
    let dark = |x: usize, y: usize| -> bool {
        let (Some(x), Some(y)) = (x.checked_sub(QUIET), y.checked_sub(QUIET)) else {
            return false;
        };
        x < width && y < width && modules.get(y * width + x) == Some(&qrcode::Color::Dark)
    };
    let side = width + 2 * QUIET;
    let mut out = String::new();
    for y in (0..side).step_by(2) {
        out.push_str(colors);
        for x in 0..side {
            out.push(match (dark(x, y), dark(x, y + 1)) {
                (false, false) => '█',
                (true, false) => '▄',
                (false, true) => '▀',
                (true, true) => ' ',
            });
        }
        out.push_str("\x1b[0m\n");
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_code_is_square_with_a_quiet_zone() {
        let colors = "\x1b[38;2;255;255;255;48;2;0;0;0m";
        let qr = qr_code("sgnl://linkdevice?uuid=abc&pub_key=def", colors).unwrap();
        let rows: Vec<&str> = qr.lines().collect();
        let cells = |row: &str| {
            row.trim_start_matches(colors)
                .trim_end_matches("\x1b[0m")
                .chars()
                .count()
        };
        let side = cells(rows[0]);
        assert_eq!(rows.len(), side.div_ceil(2));
        assert!(rows.iter().all(|r| cells(r) == side));
        // The quiet zone is light: full blocks in white.
        assert!(rows[0].contains(&"█".repeat(side)));
    }
}
