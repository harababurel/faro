use std::path::PathBuf;

use clap::Parser;

/// A terminal client for Signal, built for people who live in tmux.
///
/// faro is unofficial and not affiliated with Signal.
#[derive(Debug, Parser)]
#[command(name = "faro", version, about, long_about)]
pub struct Cli {
    #[arg(
        long,
        env = "FARO_CONFIG",
        value_name = "FILE",
        help = "Config file, written with every setting commented out if missing \
                [default: $XDG_CONFIG_HOME/faro/config.toml, usually ~/.config/faro/config.toml]"
    )]
    pub config: Option<PathBuf>,

    #[arg(
        long,
        env = "FARO_DATA_DIR",
        value_name = "DIR",
        help = "Directory for faro's database, logs and Signal keys \
                [default: $XDG_DATA_HOME/faro, usually ~/.local/share/faro]"
    )]
    pub data_dir: Option<PathBuf>,

    /// Use the mock backend: fake contacts and messages, no Signal account needed.
    /// Currently always on: the Signal backend doesn't exist yet.
    #[arg(long)]
    pub mock: bool,

    /// Seed for the mock backend's scripted activity; the same seed replays the
    /// same conversation.
    #[arg(long, env = "FARO_MOCK_SEED", value_name = "SEED", default_value_t = 0)]
    pub mock_seed: u64,

    /// Mean milliseconds between the mock's scripted events. Small values flood
    /// the UI, for checking that typing never stalls (e.g. 5).
    #[arg(
        long,
        env = "FARO_MOCK_ACTIVITY",
        value_name = "MS",
        default_value_t = 4000
    )]
    pub mock_activity: u64,

    /// Turn off the mock's scripted outages and errors. Use with a small
    /// `--mock-activity` for a real flood: otherwise an outage starts almost at
    /// once and the mock spends most of its time offline.
    #[arg(long, env = "FARO_MOCK_NO_FAULTS")]
    pub mock_no_faults: bool,

    /// How to draw emoji. `compatible` draws emoji whose width depends on the
    /// terminal (❤️, 👍🏽, 🧗‍♀️…) in a simpler form every terminal agrees on, for
    /// mosh and other terminals that misalign them; `auto` does so when faro
    /// finds mosh. Overrides `[ui] emoji` in the config file (default: auto).
    #[arg(long, env = "FARO_EMOJI", value_enum)]
    pub emoji: Option<EmojiChoice>,

    /// Talk only to the people and groups in this file (D62): sending to
    /// anyone else is refused, and their messages, contacts and groups are
    /// dropped before they're shown or stored. For testing with a real
    /// account. faro won't start if the file can't be read.
    #[arg(long, env = "FARO_ALLOWLIST", value_name = "FILE")]
    pub allowlist: Option<PathBuf>,
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
    fn parses_mock_flags() {
        let cli = Cli::try_parse_from(["faro"]).unwrap();
        assert!(!cli.mock);
        assert_eq!(cli.mock_seed, 0);

        let cli = Cli::try_parse_from(["faro", "--mock", "--mock-seed", "42"]).unwrap();
        assert!(cli.mock);
        assert_eq!(cli.mock_seed, 42);

        assert!(Cli::try_parse_from(["faro", "--mock-seed", "-1"]).is_err());
        assert_eq!(cli.mock_activity, 4000);
        let cli = Cli::try_parse_from(["faro", "--mock-activity", "5"]).unwrap();
        assert_eq!(cli.mock_activity, 5);
        assert!(!cli.mock_no_faults);
        let cli = Cli::try_parse_from(["faro", "--mock-no-faults"]).unwrap();
        assert!(cli.mock_no_faults);
        assert_eq!(cli.emoji, None, "the config decides");
        let cli = Cli::try_parse_from(["faro", "--emoji", "compatible"]).unwrap();
        assert_eq!(cli.emoji, Some(EmojiChoice::Compatible));
        assert!(Cli::try_parse_from(["faro", "--emoji", "fancy"]).is_err());
    }
}
