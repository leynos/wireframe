//! Command line interface for the wireframe example binary.
//!
//! Provides a tiny CLI to demonstrate argument parsing and man page
//! generation.

use clap::Parser;

/// Command line arguments for the `wireframe` binary.
#[derive(Debug, Parser)]
#[command(name = "wireframe", version, about = "Example Wireframe binary")]
pub struct Cli {
    /// Name to greet.
    #[arg(short, long)]
    pub name: Option<String>,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::Cli;

    #[test]
    fn parses_name_option() {
        let cli = Cli::parse_from(["wireframe", "--name", "Sam"]);
        assert_eq!(cli.name.as_deref(), Some("Sam"));
    }
}
