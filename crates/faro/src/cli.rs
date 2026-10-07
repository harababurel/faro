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
        env = "FARO_DATA_DIR",
        value_name = "DIR",
        help = "Directory for faro's database, logs and Signal keys \
                [default: $XDG_DATA_HOME/faro, usually ~/.local/share/faro]"
    )]
    pub data_dir: Option<PathBuf>,
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
    }
}
