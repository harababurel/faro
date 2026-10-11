use std::path::{Path, PathBuf};

use clap::parser::ValueSource;
use clap::{CommandFactory as _, FromArgMatches as _, Parser};

/// A terminal client for Signal, built for people who live in tmux.
///
/// faro is unofficial and not affiliated with Signal.
#[derive(Debug, Parser)]
#[command(name = "faro", version, about, long_about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    #[arg(
        long,
        env = "FARO_CONFIG",
        global = true,
        value_name = "FILE",
        help = "Config file, written with every setting commented out if missing \
                [default: $XDG_CONFIG_HOME/faro/config.toml, usually ~/.config/faro/config.toml]"
    )]
    pub config: Option<PathBuf>,

    #[arg(
        long,
        env = "FARO_DATA_DIR",
        global = true,
        value_name = "DIR",
        help = "Directory for faro's database, logs and Signal keys \
                [default: $XDG_DATA_HOME/faro, usually ~/.local/share/faro]. \
                Never used by `faro demo`"
    )]
    pub data_dir: Option<PathBuf>,

    /// How to draw emoji. `compatible` draws emoji whose width depends on the
    /// terminal (❤️, 👍🏽, 🧗‍♀️…) in a simpler form every terminal agrees on, for
    /// mosh and other terminals that misalign them; `auto` does so when faro
    /// finds mosh. Overrides `[ui] emoji` in the config file (default: auto).
    #[arg(long, env = "FARO_EMOJI", global = true, value_enum)]
    pub emoji: Option<EmojiChoice>,

    /// Talk only to the people and groups in this file (D62): sending to
    /// anyone else is refused, and their messages, contacts and groups are
    /// dropped before they're shown or stored. For testing with a real
    /// account. faro won't start if the file can't be read.
    #[arg(long, env = "FARO_ALLOWLIST", global = true, value_name = "FILE")]
    pub allowlist: Option<PathBuf>,

    /// Does nothing now: without --allowlist faro talks to everyone, like
    /// any Signal client (D95). Still accepted so older commands work.
    #[arg(long, global = true, conflicts_with = "allowlist")]
    pub no_allowlist: bool,
}

/// The parsed command line, plus what [`Cli`] alone can't say: where a
/// value came from.
#[derive(Debug)]
pub struct Invocation {
    pub cli: Cli,
    /// `--data-dir` was typed (rather than read from `FARO_DATA_DIR`).
    /// `faro demo` refuses the first and ignores the second (D87).
    pub data_dir_typed: bool,
    /// `--allowlist` was typed (rather than read from `FARO_ALLOWLIST`).
    /// `faro demo` takes only the first (D87).
    pub allowlist_typed: bool,
}

impl Invocation {
    /// Parse the process's arguments, exiting with clap's message on error.
    pub fn parse() -> Self {
        Self::from_matches(&Cli::command().get_matches()).unwrap_or_else(|error| error.exit())
    }

    #[cfg(test)]
    pub fn try_parse_from<I, T>(args: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        Self::from_matches(&Cli::command().try_get_matches_from(args)?)
    }

    fn from_matches(matches: &clap::ArgMatches) -> Result<Self, clap::Error> {
        Ok(Self {
            cli: Cli::from_arg_matches(matches)?,
            data_dir_typed: matches.value_source("data_dir") == Some(ValueSource::CommandLine),
            allowlist_typed: matches.value_source("allowlist") == Some(ValueSource::CommandLine),
        })
    }

    fn is_demo(&self) -> bool {
        matches!(self.cli.command, Some(Command::Demo(_)))
    }

    /// The allowlist this run uses. The demo takes only a typed one:
    /// `FARO_ALLOWLIST` is for a real account, and would leave the demo's
    /// fake world empty (D87).
    pub fn allowlist(&self) -> Option<&Path> {
        self.cli
            .allowlist
            .as_deref()
            .filter(|_| !self.is_demo() || self.allowlist_typed)
    }

    /// `FARO_ALLOWLIST` is set but this demo run ignores it (worth a log line).
    pub fn ignores_env_allowlist(&self) -> bool {
        self.cli.allowlist.is_some() && self.allowlist().is_none()
    }

    /// Why the flags can't go with `faro demo`, if they can't.
    pub fn demo_conflict(&self) -> Option<&'static str> {
        if self.data_dir_typed {
            Some(
                "`faro demo` never uses --data-dir: its data stays apart from yours. \
                 Use `faro demo --dir DIR` for another demo directory",
            )
        } else {
            None
        }
    }
}

/// Things to do instead of running the TUI.
#[derive(Debug, PartialEq, Eq, clap::Subcommand)]
pub enum Command {
    /// Try faro without a Signal account: fake contacts and scripted
    /// conversations, nothing is sent. Kept in its own directory, never with
    /// your real data; your config is shared
    Demo(DemoArgs),
    /// Link this computer to your Signal account as a new device (shows a QR
    /// code to scan with Signal on your phone)
    Link {
        /// Name shown in the phone's list of linked devices [default: faro (<hostname>)]
        #[arg(long, value_name = "NAME")]
        device_name: Option<String>,
    },
    /// Print the Signal IDs an allowlist needs: this account's, the given
    /// usernames' and the known groups' (titles and IDs only)
    Ids {
        /// A Signal username to look up (repeatable), e.g. --username alice.42
        #[arg(long = "username", value_name = "NAME")]
        usernames: Vec<String>,
    },
}

/// `faro demo`'s flags (D87). Command line only: no environment variables,
/// so nothing left in a shell profile changes a demo run.
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct DemoArgs {
    /// Seed for the scripted activity; the same seed replays the same
    /// conversation.
    #[arg(long, value_name = "SEED", default_value_t = 0)]
    pub seed: u64,

    /// Mean milliseconds between scripted events. Small values flood the UI,
    /// for checking that typing never stalls (e.g. 5).
    #[arg(long, value_name = "MS", default_value_t = 4000)]
    pub activity: u64,

    /// Turn off the scripted outages and errors. Use with a small
    /// `--activity` for a real flood: otherwise an outage starts almost at
    /// once and the demo spends most of its time offline.
    #[arg(long)]
    pub no_faults: bool,

    /// Delete the demo's data and start over (only ever a directory faro
    /// marked as a demo directory).
    #[arg(long)]
    pub fresh: bool,

    /// Keep the demo's data here instead. It must be new, empty, or an
    /// earlier demo directory [default: <temp>/faro-demo-<uid>, kept until
    /// the system clears it]
    #[arg(long, value_name = "DIR")]
    pub dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum EmojiChoice {
    Auto,
    Full,
    Compatible,
}

impl From<EmojiChoice> for faro_core::config::EmojiChoice {
    fn from(choice: EmojiChoice) -> Self {
        match choice {
            EmojiChoice::Auto => Self::Auto,
            EmojiChoice::Full => Self::Full,
            EmojiChoice::Compatible => Self::Compatible,
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_data_dir() {
        let cli = Cli::try_parse_from(["faro", "--data-dir", "/tmp/x"]).unwrap();
        assert_eq!(cli.data_dir, Some(PathBuf::from("/tmp/x")));
        assert_eq!(cli.config, None);
        let cli = Cli::try_parse_from(["faro", "--config", "/tmp/c.toml"]).unwrap();
        assert_eq!(cli.config, Some(PathBuf::from("/tmp/c.toml")));
    }

    #[test]
    fn parses_subcommands() {
        assert_eq!(Cli::try_parse_from(["faro"]).unwrap().command, None);
        let cli = Cli::try_parse_from(["faro", "link", "--data-dir", "/tmp/x"]).unwrap();
        assert_eq!(cli.command, Some(Command::Link { device_name: None }));
        assert_eq!(cli.data_dir, Some(PathBuf::from("/tmp/x")));
        let cli =
            Cli::try_parse_from(["faro", "ids", "--username", "a.1", "--username", "b.2"]).unwrap();
        assert_eq!(
            cli.command,
            Some(Command::Ids {
                usernames: vec!["a.1".to_owned(), "b.2".to_owned()]
            })
        );
    }

    #[test]
    fn parses_demo_flags() {
        let demo = |args: &[&str]| match Cli::try_parse_from(args).unwrap().command {
            Some(Command::Demo(demo)) => demo,
            other => panic!("expected demo, got {other:?}"),
        };
        assert_eq!(
            demo(&["faro", "demo"]),
            DemoArgs {
                seed: 0,
                activity: 4000,
                no_faults: false,
                fresh: false,
                dir: None,
            }
        );
        assert_eq!(
            demo(&[
                "faro",
                "demo",
                "--seed",
                "42",
                "--activity",
                "5",
                "--no-faults",
                "--fresh",
                "--dir",
                "/tmp/rec-1",
            ]),
            DemoArgs {
                seed: 42,
                activity: 5,
                no_faults: true,
                fresh: true,
                dir: Some(PathBuf::from("/tmp/rec-1")),
            }
        );
        assert!(Cli::try_parse_from(["faro", "demo", "--seed", "-1"]).is_err());
        // Demo flags belong to the subcommand; the old ones are gone.
        for args in [
            &["faro", "--seed", "1"][..],
            &["faro", "--mock"],
            &["faro", "--mock-seed", "1"],
            &["faro", "--mock-activity", "5"],
            &["faro", "--mock-no-faults"],
        ] {
            assert!(Cli::try_parse_from(args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn global_flags_work_after_demo() {
        let cli = Cli::try_parse_from([
            "faro",
            "demo",
            "--emoji",
            "compatible",
            "--allowlist",
            "/tmp/a.toml",
            "--config",
            "/tmp/c.toml",
        ])
        .unwrap();
        assert_eq!(cli.emoji, Some(EmojiChoice::Compatible));
        assert_eq!(cli.allowlist, Some(PathBuf::from("/tmp/a.toml")));
        assert_eq!(cli.config, Some(PathBuf::from("/tmp/c.toml")));
        assert!(Cli::try_parse_from(["faro", "--emoji", "fancy"]).is_err());
    }

    #[test]
    fn demo_refuses_a_typed_data_dir() {
        let ok = Invocation::try_parse_from(["faro", "demo", "--fresh"]).unwrap();
        assert!(!ok.data_dir_typed, "FARO_DATA_DIR, if set, isn't typed");
        assert_eq!(ok.demo_conflict(), None);
        for args in [
            &["faro", "demo", "--data-dir", "/tmp/x"][..],
            &["faro", "--data-dir", "/tmp/x", "demo"],
        ] {
            let typed = Invocation::try_parse_from(args).unwrap();
            assert!(typed.data_dir_typed, "{args:?}");
            assert!(typed.demo_conflict().unwrap().contains("--dir"), "{args:?}");
        }
        // `--no-allowlist` does nothing anywhere now (D95), the demo included.
        for args in [
            &["faro", "demo", "--no-allowlist"][..],
            &["faro", "--no-allowlist", "demo"],
        ] {
            let old = Invocation::try_parse_from(args).unwrap();
            assert_eq!(old.demo_conflict(), None, "{args:?}");
        }
    }

    #[test]
    fn the_demo_takes_a_typed_allowlist() {
        for args in [
            &["faro", "demo", "--allowlist", "/tmp/a.toml"][..],
            &["faro", "--allowlist", "/tmp/a.toml", "demo"],
        ] {
            let typed = Invocation::try_parse_from(args).unwrap();
            assert!(typed.allowlist_typed, "{args:?}");
            assert_eq!(typed.allowlist(), Some(Path::new("/tmp/a.toml")));
            assert!(!typed.ignores_env_allowlist());
        }
    }

    /// Values from the environment don't count as typed, so the demo ignores
    /// `FARO_DATA_DIR` and `FARO_ALLOWLIST` (D87). Checked in a child
    /// process: tests can't set environment variables safely.
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "a test waiting for its own child process"
    )]
    fn environment_values_are_not_typed() {
        const CHILD: &str = "FARO_CLI_TEST_CHILD";
        const NAME: &str = "cli::tests::environment_values_are_not_typed";
        if std::env::var_os(CHILD).is_some() {
            let demo = Invocation::try_parse_from(["faro", "demo"]).unwrap();
            assert_eq!(demo.cli.data_dir, Some(PathBuf::from("/env/data")));
            assert!(!demo.data_dir_typed);
            assert_eq!(demo.demo_conflict(), None);
            assert_eq!(demo.cli.allowlist, Some(PathBuf::from("/env/allow.toml")));
            assert!(!demo.allowlist_typed);
            assert_eq!(demo.allowlist(), None);
            assert!(demo.ignores_env_allowlist());
            // A real run still uses it.
            let real = Invocation::try_parse_from(["faro"]).unwrap();
            assert_eq!(real.allowlist(), Some(Path::new("/env/allow.toml")));
            assert!(!real.ignores_env_allowlist());
            return;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", NAME, "--test-threads", "1"])
            .env(CHILD, "1")
            .env("FARO_DATA_DIR", "/env/data")
            .env("FARO_ALLOWLIST", "/env/allow.toml")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "{stdout}");
        assert!(
            stdout.contains("1 passed"),
            "the child ran the test: {stdout}"
        );
    }

    #[test]
    fn no_allowlist_is_still_accepted_but_excludes_an_allowlist() {
        assert!(!Cli::try_parse_from(["faro"]).unwrap().no_allowlist);
        assert!(
            Cli::try_parse_from(["faro", "--no-allowlist"])
                .unwrap()
                .no_allowlist
        );
        assert!(
            Cli::try_parse_from(["faro", "--no-allowlist", "--allowlist", "/tmp/a.toml"]).is_err()
        );
    }
}
