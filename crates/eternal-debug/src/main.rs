use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use anyhow::{Context, Result, ensure};
use clap::Parser;
use eternal_core::{
    Analysis, AnalysisConfig, BranchConfig, BranchGraph, PlaybackPlanner, analyse, decode,
};
use serde::{Deserialize, Serialize};

const DEFAULT_SEED: u64 = 1_474_317_007;
const DEFAULT_STEPS: usize = 20_000;
const DEFAULT_RUNS: usize = 4;
// Bump when feature extraction or beat tracking changes; old JSON can still
// deserialize successfully while containing an obsolete beat grid.
const ANALYSIS_CACHE_VERSION: u32 = 2;

#[derive(Debug, Parser)]
#[command(about = "Deterministically simulate Eternal Jukebox playback")]
struct Cli {
    /// Audio files to analyse and simulate.
    #[arg(required = true)]
    inputs: Vec<PathBuf>,
    /// First deterministic seed; additional runs use consecutive seeds.
    #[arg(long, default_value_t = DEFAULT_SEED)]
    seed: u64,
    /// Planned beats per run.
    #[arg(long, default_value_t = DEFAULT_STEPS)]
    steps: usize,
    /// Number of consecutive seeds to simulate.
    #[arg(long, default_value_t = DEFAULT_RUNS)]
    runs: usize,
    /// Print outgoing branches around this beat (may be repeated).
    #[arg(long)]
    inspect: Vec<usize>,
    /// Print one candidate as SOURCE:DESTINATION (may be repeated).
    #[arg(long)]
    edge: Vec<String>,
    /// Ignore cached audio analysis.
    #[arg(long)]
    no_cache: bool,
    /// Nearest candidates retained per source while building the graph.
    #[arg(long, default_value_t = 4)]
    max_branches: usize,
    /// Fraction of beats targeted as branch sources.
    #[arg(long, default_value_t = 0.1)]
    branch_fraction: f32,
    /// Override the graph's adaptive distance threshold.
    #[arg(long)]
    threshold: Option<f32>,
    /// Fail if any run exceeds this number of physical end-of-file restarts.
    #[arg(long)]
    max_wraps: Option<usize>,
}

#[derive(Serialize, Deserialize)]
struct CachedTrack {
    analysis: Analysis,
    #[serde(default, rename = "graph")]
    _graph: Option<BranchGraph>,
}

struct Simulation {
    visits: Vec<u32>,
    jumps: usize,
    backward: usize,
    long_backward: usize,
    wraps: usize,
    longest_local_run: usize,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    for input in &cli.inputs {
        let cached = prepare(input, cli.no_cache)?;
        let graph = BranchGraph::build(
            &cached.analysis,
            &BranchConfig {
                maximum_branches: cli.max_branches,
                target_branch_fraction: cli.branch_fraction,
                threshold: cli.threshold,
                ..BranchConfig::default()
            },
        );
        print_graph(input, &cached.analysis, &graph);
        for &beat in &cli.inspect {
            print_branches(&graph, beat);
        }
        for edge in &cli.edge {
            print_edge(&graph, edge)?;
        }
        for offset in 0..cli.runs {
            let seed = cli.seed.wrapping_add(offset as u64);
            let simulation = simulate(&graph, seed, cli.steps);
            print_simulation(seed, &simulation);
            if let Some(limit) = cli.max_wraps {
                ensure!(
                    simulation.wraps <= limit,
                    "{}: seed {seed} reached the end {} times (limit {limit})",
                    input.display(),
                    simulation.wraps
                );
            }
        }
    }
    Ok(())
}

fn prepare(input: &Path, no_cache: bool) -> Result<CachedTrack> {
    let cache_path = cache_path(input)?;
    if !no_cache && cache_path.exists() {
        let bytes = fs::read(&cache_path).context("could not read analysis cache")?;
        return serde_json::from_slice(&bytes).context("could not parse analysis cache");
    }
    eprintln!("analyse {}", input.display());
    let audio = decode(input)?;
    let analysis = analyse(&audio, &AnalysisConfig::default())?;
    let cached = CachedTrack {
        analysis,
        _graph: None,
    };
    if let Some(parent) = cache_path.parent() {
        fs::create_dir_all(parent).context("could not create analysis cache")?;
    }
    fs::write(&cache_path, serde_json::to_vec(&cached)?)
        .context("could not write analysis cache")?;
    Ok(cached)
}

fn cache_path(input: &Path) -> Result<PathBuf> {
    let metadata = input.metadata().context("could not inspect input")?;
    let modified = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut hasher = DefaultHasher::new();
    ANALYSIS_CACHE_VERSION.hash(&mut hasher);
    input.canonicalize()?.hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    modified.hash(&mut hasher);
    Ok(PathBuf::from("target/eternal-debug-cache").join(format!("{:016x}.json", hasher.finish())))
}

fn print_graph(input: &Path, analysis: &Analysis, graph: &BranchGraph) {
    let sources = graph
        .branches
        .iter()
        .filter(|items| !items.is_empty())
        .count();
    let backward: Vec<_> = graph
        .branches
        .iter()
        .enumerate()
        .flat_map(|(source, items)| {
            items
                .iter()
                .filter(move |branch| branch.destination < source)
                .map(move |branch| (source - branch.destination, source, branch))
        })
        .collect();
    let long = backward
        .iter()
        .filter(|(distance, _, _)| *distance >= analysis.beats.len() / 4)
        .count();
    println!(
        "TRACK {} beats={} bpm={:.1} threshold={:.3} edges={} sources={} back={} long_back={} last={}",
        input
            .file_name()
            .unwrap_or(input.as_os_str())
            .to_string_lossy(),
        analysis.beats.len(),
        analysis.tempo,
        graph.threshold,
        graph.branch_count(),
        sources,
        backward.len(),
        long,
        graph.last_branch_point
    );
    let mut longest = backward;
    longest.sort_by_key(|item| std::cmp::Reverse(item.0));
    let labels = longest
        .iter()
        .take(6)
        .map(|(_, source, branch)| {
            format!("{source}>{}:{:.2}", branch.destination, branch.distance)
        })
        .collect::<Vec<_>>()
        .join(",");
    println!("GRAPH longest=[{labels}]");
    let mut best_long = longest
        .into_iter()
        .filter(|(span, _, _)| *span >= analysis.beats.len() / 4)
        .collect::<Vec<_>>();
    best_long.sort_by(|left, right| left.2.distance.total_cmp(&right.2.distance));
    let labels = best_long
        .into_iter()
        .take(6)
        .map(|(_, source, branch)| {
            format!("{source}>{}:{:.2}", branch.destination, branch.distance)
        })
        .collect::<Vec<_>>()
        .join(",");
    println!("GRAPH best_long=[{labels}]");
}

fn print_branches(graph: &BranchGraph, beat: usize) {
    let start = beat.saturating_sub(3);
    let end = (beat + 4).min(graph.branches.len());
    for source in start..end {
        let branches = graph.branches[source]
            .iter()
            .map(|branch| format!("{}:{:.2}", branch.destination, branch.distance))
            .collect::<Vec<_>>()
            .join(",");
        println!("BRANCH {source}=[{branches}]");
    }
}

fn print_edge(graph: &BranchGraph, value: &str) -> Result<()> {
    let (source, destination) = value
        .split_once(':')
        .context("edge must use SOURCE:DESTINATION")?;
    let source = source.parse::<usize>().context("invalid edge source")?;
    let destination = destination
        .parse::<usize>()
        .context("invalid edge destination")?;
    let distance = graph
        .branches
        .get(source)
        .and_then(|items| {
            items
                .iter()
                .find(|branch| branch.destination == destination)
        })
        .map_or_else(
            || "unavailable".to_owned(),
            |branch| format!("{:.3}", branch.distance),
        );
    println!("EDGE {source}>{destination}={distance}");
    Ok(())
}

fn simulate(graph: &BranchGraph, seed: u64, steps: usize) -> Simulation {
    let mut planner = PlaybackPlanner::with_seed(graph.clone(), seed);
    let mut visits = vec![0_u32; graph.branches.len()];
    let mut jumps = 0;
    let mut backward = 0;
    let mut long_backward = 0;
    let mut wraps = 0;
    let mut local_run = 0;
    let mut longest_local_run = 0;
    let locality = (graph.branches.len() / 12).max(8);
    let mut recent = Vec::with_capacity(64);
    let mut previous_beat = None;
    for _ in 0..steps {
        let Some(step) = planner.next_step() else {
            break;
        };
        visits[step.beat] = visits[step.beat].saturating_add(1);
        if let Some(source) = step.jumped_from {
            // A branch *replacing* the final beat with beat zero is a valid
            // jump, not a restart after actually playing the physical end.
            let is_wrap = previous_beat == graph.branches.len().checked_sub(1) && step.beat == 0;
            if is_wrap {
                wraps += 1;
            } else {
                jumps += 1;
                if step.beat < source {
                    backward += 1;
                    long_backward += usize::from(source - step.beat >= graph.branches.len() / 4);
                }
            }
        }
        previous_beat = Some(step.beat);
        recent.push(step.beat);
        if recent.len() > 64 {
            recent.remove(0);
        }
        let range = recent
            .iter()
            .max()
            .zip(recent.iter().min())
            .map_or(0, |(max, min)| max - min);
        if recent.len() == 64 && range <= locality {
            local_run += 1;
            longest_local_run = longest_local_run.max(local_run + 63);
        } else {
            local_run = 0;
        }
    }
    Simulation {
        visits,
        jumps,
        backward,
        long_backward,
        wraps,
        longest_local_run,
    }
}

fn print_simulation(seed: u64, simulation: &Simulation) {
    let total = simulation
        .visits
        .iter()
        .map(|value| u64::from(*value))
        .sum::<u64>();
    let covered = simulation.visits.iter().filter(|value| **value > 0).count();
    let mean = total as f64 / simulation.visits.len().max(1) as f64;
    let variance = simulation
        .visits
        .iter()
        .map(|value| (f64::from(*value) - mean).powi(2))
        .sum::<f64>()
        / simulation.visits.len().max(1) as f64;
    let cv = variance.sqrt() / mean.max(f64::EPSILON);
    let quartiles = (0..4)
        .map(|quarter| {
            let start = quarter * simulation.visits.len() / 4;
            let end = (quarter + 1) * simulation.visits.len() / 4;
            f64::from(simulation.visits[start..end].iter().sum::<u32>()) / total.max(1) as f64
        })
        .map(|share| format!("{:.0}", share * 100.0))
        .collect::<Vec<_>>()
        .join("/");
    let window = simulation
        .visits
        .len()
        .div_ceil(20)
        .max(8)
        .min(simulation.visits.len());
    let (hot_start, hot_visits) = simulation
        .visits
        .windows(window.max(1))
        .enumerate()
        .map(|(start, values)| (start, values.iter().sum::<u32>()))
        .max_by_key(|item| item.1)
        .unwrap_or((0, 0));
    println!(
        "SIM seed={seed} cover={covered}/{}({:.0}%) cv={cv:.2} q%={quartiles} hot={hot_start}-{}:{:.1}% jumps={}/{}b/{}long wraps={} local_max={}",
        simulation.visits.len(),
        100.0 * covered as f64 / simulation.visits.len().max(1) as f64,
        hot_start + window.saturating_sub(1),
        100.0 * f64::from(hot_visits) / total.max(1) as f64,
        simulation.jumps,
        simulation.backward,
        simulation.long_backward,
        simulation.wraps,
        simulation.longest_local_run,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use eternal_core::Branch;

    #[test]
    fn final_beat_replacement_is_not_counted_as_a_restart() {
        let graph = BranchGraph {
            branches: vec![
                vec![],
                vec![],
                vec![Branch {
                    destination: 0,
                    distance: 0.1,
                }],
            ],
            threshold: 0.2,
            last_branch_point: 2,
        };
        let simulation = simulate(&graph, DEFAULT_SEED, 100);
        assert_eq!(simulation.wraps, 0);
        assert!(simulation.jumps > 0);
        assert_eq!(simulation.visits[2], 0);
    }

    #[test]
    fn missing_exits_are_counted_as_physical_restarts() {
        let graph = BranchGraph {
            branches: vec![vec![]; 3],
            threshold: 0.0,
            last_branch_point: 0,
        };
        let simulation = simulate(&graph, DEFAULT_SEED, 10);
        assert_eq!(simulation.wraps, 3);
        assert_eq!(simulation.jumps, 0);
    }
}
