mod playback;
mod tui;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use eternal_core::{AnalysisConfig, BranchConfig, BranchGraph, analyse, decode};
use serde_json::json;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "eternal",
    version,
    about = "Make a local audio file play forever"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Analyse a file and print its beat graph as JSON.
    Analyse {
        input: PathBuf,
        /// Write JSON to this file instead of stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Override the automatically selected branch threshold.
        #[arg(long)]
        threshold: Option<f32>,
    },
    /// Analyse and play a file indefinitely.
    Play {
        input: PathBuf,
        /// Override the automatically selected branch threshold.
        #[arg(long)]
        threshold: Option<f32>,
        /// Make the sequence reproducible.
        #[arg(long)]
        seed: Option<u64>,
        /// Show an interactive live playback dashboard.
        #[arg(long)]
        tui: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Analyse {
            input,
            output,
            threshold,
        } => {
            let (analysis, graph, _) = prepare(&input, threshold)?;
            let document = serde_json::to_string_pretty(&json!({
                "analysis": analysis,
                "graph": graph,
            }))?;
            if let Some(output) = output {
                std::fs::write(&output, document)
                    .with_context(|| format!("could not write {}", output.display()))?;
            } else {
                println!("{document}");
            }
        }
        Command::Play {
            input,
            threshold,
            seed,
            tui,
        } => {
            let (analysis, graph, audio) = prepare(&input, threshold)?;
            if graph.branch_count() == 0 {
                bail!("no usable transitions found; try a longer or more repetitive track");
            }
            eprintln!(
                "{:.1} BPM, {} beats, {} transitions (threshold {:.3})",
                analysis.tempo,
                analysis.beats.len(),
                graph.branch_count(),
                graph.threshold
            );
            if tui {
                tui::play(&audio, &analysis, &graph, seed, &input)?;
            } else {
                eprintln!("Playing forever; press Ctrl-C to stop.");
                playback::play(&audio, &analysis, graph, seed)?;
            }
        }
    }
    Ok(())
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
