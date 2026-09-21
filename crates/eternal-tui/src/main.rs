mod audio_source;
mod connections;
mod mpris;
mod playback;
mod tui;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use eternal_core::{AnalysisConfig, BranchConfig, BranchGraph, analyse, decode};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "eternal",
    version,
    about = "Make a local audio file play forever",
    subcommand_precedence_over_arg = true,
    subcommand_negates_reqs = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<ControlCommand>,
    /// Audio file to play indefinitely.
    #[arg(required = true)]
    input: Option<PathBuf>,
    /// Override the automatically selected branch threshold.
    #[arg(long)]
    threshold: Option<f32>,
    /// Make the sequence reproducible.
    #[arg(long)]
    seed: Option<u64>,
}

impl Cli {
    fn validate(&self) -> Result<()> {
        if self.command.is_some()
            && (self.input.is_some() || self.threshold.is_some() || self.seed.is_some())
        {
            bail!("control commands cannot be combined with a file, --threshold, or --seed");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Subcommand)]
enum ControlCommand {
    /// Resume the running Eternal player.
    Play,
    /// Pause the running Eternal player.
    Pause,
    /// Toggle playback in the running Eternal player.
    Toggle,
    /// Print the running player's playback status.
    Status,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    cli.validate()?;
    if let Some(command) = cli.command {
        return mpris::control(command);
    }
    let input = cli.input.expect("clap requires a file or a subcommand");
    let remote = mpris::Service::start(&input)?;
    let (analysis, graph, audio) = prepare(&input, cli.threshold)?;
    if graph.branch_count() == 0 {
        bail!("no usable transitions found; try a longer or more repetitive track");
    }
    tui::play(&audio, &analysis, &graph, cli.seed, &input, remote)
}

fn prepare(
    input: &PathBuf,
    threshold: Option<f32>,
) -> Result<(
    eternal_core::Analysis,
    eternal_core::BranchGraph,
    eternal_core::Audio,
)> {
    eprintln!("Decoding {}…", input.display());
    let audio = decode(input)?;
    eprintln!("Analysing {:.1} seconds…", audio.duration_seconds());
    let analysis = analyse(&audio, &AnalysisConfig::default())?;
    let graph = BranchGraph::build(
        &analysis,
        &BranchConfig {
            threshold,
            ..BranchConfig::default()
        },
    );
    Ok((analysis, graph, audio))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_commands_do_not_require_a_file() {
        for command in ["play", "pause", "toggle", "status"] {
            let cli = Cli::try_parse_from(["eternal", command]).unwrap();
            assert!(cli.command.is_some());
            assert!(cli.input.is_none());
        }
    }

    #[test]
    fn existing_file_syntax_and_flags_are_preserved() {
        let cli =
            Cli::try_parse_from(["eternal", "song.mp3", "--threshold", "0.5", "--seed", "42"])
                .unwrap();
        assert_eq!(cli.input, Some(PathBuf::from("song.mp3")));
        assert_eq!(cli.seed, Some(42));
        assert!(cli.command.is_none());
        for path in ["./play", "./pause"] {
            let cli = Cli::try_parse_from(["eternal", path]).unwrap();
            assert_eq!(cli.input, Some(PathBuf::from(path)));
        }
    }

    #[test]
    fn missing_file_and_playback_flags_on_control_commands_are_rejected() {
        assert!(Cli::try_parse_from(["eternal"]).is_err());
        assert!(Cli::try_parse_from(["eternal", "pause", "--seed", "42"]).is_err());
        let cli = Cli::try_parse_from(["eternal", "--seed", "42", "pause"]).unwrap();
        assert!(cli.validate().is_err());
    }
}
