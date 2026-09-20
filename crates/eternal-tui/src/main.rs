mod audio_source;
mod connections;
mod tui;

use anyhow::{Result, bail};
use clap::Parser;
use eternal_core::{AnalysisConfig, BranchConfig, BranchGraph, analyse, decode};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "eternal",
    version,
    about = "Make a local audio file play forever"
)]
struct Cli {
    /// Audio file to play indefinitely.
    input: PathBuf,
    /// Override the automatically selected branch threshold.
    #[arg(long)]
    threshold: Option<f32>,
    /// Make the sequence reproducible.
    #[arg(long)]
    seed: Option<u64>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let (analysis, graph, audio) = prepare(&cli.input, cli.threshold)?;
    if graph.branch_count() == 0 {
        bail!("no usable transitions found; try a longer or more repetitive track");
    }
    tui::play(&audio, &analysis, &graph, cli.seed, &cli.input)
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
