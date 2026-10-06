use serde::{Deserialize, Serialize};

use crate::{Analysis, Features};

#[derive(Clone, Debug)]
pub struct BranchConfig {
    pub maximum_branches: usize,
    pub minimum_separation: usize,
    pub target_branch_fraction: f32,
    pub threshold: Option<f32>,
    /// Spider traps shorter than this are stripped of all jumps.
    /// `None` (or `Some(0.0)`) disables the pruning. The default sits just
    /// above the largest known outro trap, with margin for decoder jitter:
    /// resampling the same track can move a trap by a second or more.
    pub island_threshold_seconds: Option<f64>,
}

impl Default for BranchConfig {
    fn default() -> Self {
        Self {
            maximum_branches: 4,
            minimum_separation: 4,
            target_branch_fraction: 1.0 / 10.0,
            threshold: None,
            island_threshold_seconds: Some(45.0),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Branch {
    pub destination: usize,
    pub distance: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BranchGraph {
    pub branches: Vec<Vec<Branch>>,
    pub threshold: f32,
    pub last_branch_point: usize,
}

impl BranchGraph {
    #[must_use]
    pub fn build(analysis: &Analysis, config: &BranchConfig) -> Self {
        let count = analysis.beats.len();
        if count == 0 {
            return Self {
                branches: Vec::new(),
                threshold: 0.0,
                last_branch_point: 0,
            };
        }

        let normalisation = Normalisation::from_analysis(analysis);
        let candidates: Vec<Vec<Branch>> = analysis
            .beats
            .iter()
            .map(|source| {
                let mut nearest = Vec::new();
                for destination in analysis.beats.iter().filter(|destination| {
                    source.index.abs_diff(destination.index) >= config.minimum_separation
                        && source.index % 4 == destination.index % 4
                }) {
                    retain_nearest(
                        &mut nearest,
                        Branch {
                            destination: destination.index,
                            distance: transition_distance(
                                analysis,
                                source.index,
                                destination.index,
                                &normalisation,
                            ),
                        },
                        config.maximum_branches,
                    );
                }
                nearest
            })
            .collect();

        let threshold = config.threshold.unwrap_or_else(|| {
            adaptive_threshold(&candidates, count, config.target_branch_fraction)
        });
        let mut branches: Vec<Vec<Branch>> = candidates
            .iter()
            .map(|items| {
                items
                    .iter()
                    .filter(|branch| branch.distance <= threshold)
                    .cloned()
                    .collect()
            })
            .collect();

        ensure_long_backward_branch(&candidates, &mut branches, threshold);
        prune_spider_traps(&mut branches, analysis, threshold, config);
        let last_branch_point = best_last_branch(&branches);
        remove_end_traps(&mut branches, last_branch_point);
        Self {
            branches,
            threshold,
            last_branch_point,
        }
    }

    #[must_use]
    pub fn branch_count(&self) -> usize {
        self.branches.iter().map(Vec::len).sum()
    }

    /// Contiguous beat ranges split wherever no jump crosses the boundary.
    ///
    /// A diagnostic view of the jump structure: each range has jumps inside
    /// but none leading out. Only the range sealing the forced exit is a real
    /// trap (see [`Self::spider_trap`]); the rest are traversed sequentially
    /// and their jumps are harmless local loops.
    #[must_use]
    pub fn jump_segments(&self) -> Vec<std::ops::Range<usize>> {
        jump_segments(&self.branches)
    }

    /// Last source with a backward edge within threshold, if any.
    ///
    /// The planner forces a jump here, so this beat seals every trap behind
    /// it. Shared with the planner so the two can never disagree.
    #[must_use]
    pub fn forced_exit_source(&self) -> Option<usize> {
        forced_exit_source(&self.branches, self.threshold)
    }

    /// Beats of the spider trap: entered sequentially, never left.
    ///
    /// Computed on the directed graph of moves the planner can actually take
    /// (sequential steps, minus the forced exit's, plus all jumps except the
    /// ones the exit logic zeroes). Empty when no forced exit exists. The
    /// trap is always one contiguous range ending at the forced exit.
    #[must_use]
    pub fn spider_trap(&self) -> Vec<usize> {
        spider_trap_beats(&self.branches, self.threshold)
    }
}

fn retain_nearest(nearest: &mut Vec<Branch>, branch: Branch, limit: usize) {
    if limit == 0
        || nearest.last().is_some_and(|last| {
            nearest.len() == limit
                && branch.distance.total_cmp(&last.distance) != std::cmp::Ordering::Less
        })
    {
        return;
    }
    let position = nearest.partition_point(|existing| {
        existing.distance.total_cmp(&branch.distance) != std::cmp::Ordering::Greater
    });
    nearest.insert(position, branch);
    nearest.truncate(limit);
}

struct Normalisation {
    loudness: f32,
    onset: f32,
}

impl Normalisation {
    fn from_analysis(analysis: &Analysis) -> Self {
        let loudness = range(analysis.beats.iter().map(|beat| beat.features.loudness_db));
        let onset = range(
            analysis
                .beats
                .iter()
                .map(|beat| beat.features.onset_strength),
        );
        Self {
            loudness: loudness.max(1.0),
            onset: onset.max(1.0e-6),
        }
    }
}

fn range(values: impl Iterator<Item = f32>) -> f32 {
    let (minimum, maximum) = values.fold((f32::INFINITY, f32::NEG_INFINITY), |acc, value| {
        (acc.0.min(value), acc.1.max(value))
    });
    maximum - minimum
}

fn feature_distance(left: &Features, right: &Features, norm: &Normalisation) -> f32 {
    let chroma = euclidean(&left.chroma, &right.chroma);
    let timbre = euclidean(&left.timbre, &right.timbre);
    let spectral_bands = euclidean(&left.spectral_bands, &right.spectral_bands);
    let loudness = (left.loudness_db - right.loudness_db).abs() / norm.loudness;
    let onset = (left.onset_strength - right.onset_strength).abs() / norm.onset;
    let onset_profile = euclidean(&left.onset_profile, &right.onset_profile) / norm.onset;
    10.0 * chroma + 4.0 * timbre + 3.0 * spectral_bands + 2.0 * loudness + onset + onset_profile
}

fn transition_distance(
    analysis: &Analysis,
    source: usize,
    destination: usize,
    norm: &Normalisation,
) -> f32 {
    let beats = &analysis.beats;
    let direct =
        0.55 * feature_distance(
            &beats[source].start_features,
            &beats[destination].start_features,
            norm,
        ) + 0.25 * feature_distance(&beats[source].features, &beats[destination].features, norm);

    // Matching neighbouring beats avoids locally similar attacks that belong
    // to incompatible phrases. Missing context at the track edges is neutral.
    let previous = source
        .checked_sub(1)
        .zip(destination.checked_sub(1))
        .map_or(0.0, |(left, right)| {
            0.10 * feature_distance(&beats[left].features, &beats[right].features, norm)
        });
    let next = beats
        .get(source + 1)
        .zip(beats.get(destination + 1))
        .map_or(0.0, |(left, right)| {
            0.10 * feature_distance(&left.features, &right.features, norm)
        });
    direct + previous + next
}

fn euclidean<const N: usize>(left: &[f32; N], right: &[f32; N]) -> f32 {
    left.iter()
        .zip(right)
        .map(|(left, right)| (left - right).powi(2))
        .sum::<f32>()
        .sqrt()
}

fn adaptive_threshold(candidates: &[Vec<Branch>], beat_count: usize, target_fraction: f32) -> f32 {
    let mut best_distances: Vec<_> = candidates
        .iter()
        .filter_map(|branches| branches.first().map(|branch| branch.distance))
        .collect();
    if best_distances.is_empty() {
        return 0.0;
    }
    best_distances.sort_by(f32::total_cmp);
    let target =
        ((beat_count as f32 * target_fraction).ceil() as usize).clamp(1, best_distances.len());
    best_distances[target - 1]
}

fn ensure_long_backward_branch(
    candidates: &[Vec<Branch>],
    branches: &mut [Vec<Branch>],
    threshold: f32,
) {
    // Short backward edges can form a tiny closed loop near the outro. Ensure
    // there is at least one structural loop spanning a quarter of the track.
    let minimum_span = (branches.len() / 4).max(1);
    if branches.iter().enumerate().any(|(source, items)| {
        items
            .iter()
            .any(|branch| source.saturating_sub(branch.destination) >= minimum_span)
    }) {
        return;
    }
    let maximum_distance = threshold * 1.75;
    let best = candidates
        .iter()
        .enumerate()
        .flat_map(|(source, items)| {
            items
                .iter()
                .filter(move |branch| {
                    source.saturating_sub(branch.destination) >= minimum_span
                        && branch.distance <= maximum_distance
                })
                .map(move |branch| (source, branch))
        })
        .max_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| right.1.distance.total_cmp(&left.1.distance))
        });
    if let Some((source, branch)) = best
        && !branches[source]
            .iter()
            .any(|existing| existing.destination == branch.destination)
    {
        branches[source].push(branch.clone());
    }
}

fn best_last_branch(branches: &[Vec<Branch>]) -> usize {
    branches
        .iter()
        .enumerate()
        .rev()
        .find(|(source, items)| items.iter().any(|branch| branch.destination < *source))
        .map_or(0, |(index, _)| index)
}

fn remove_end_traps(branches: &mut [Vec<Branch>], last_branch_point: usize) {
    for items in branches.iter_mut().take(last_branch_point) {
        items.retain(|branch| branch.destination < last_branch_point);
    }
}

/// Split beats into contiguous ranges wherever no jump crosses the boundary.
///
/// A jump between beats `low` and `high` seals every boundary in between, so
/// each returned range has jumps inside but none leading out of it.
fn jump_segments(branches: &[Vec<Branch>]) -> Vec<std::ops::Range<usize>> {
    let count = branches.len();
    if count == 0 {
        return Vec::new();
    }
    let mut crossing = vec![false; count.saturating_sub(1)];
    for (source, items) in branches.iter().enumerate() {
        for branch in items {
            let destination = branch.destination.min(count - 1);
            let (low, high) = if source <= destination {
                (source, destination)
            } else {
                (destination, source)
            };
            crossing[low..high].fill(true);
        }
    }
    let mut segments = Vec::new();
    let mut start = 0;
    for (boundary, crossed) in crossing.iter().enumerate() {
        if !crossed {
            segments.push(start..boundary + 1);
            start = boundary + 1;
        }
    }
    segments.push(start..count);
    segments
}

/// Strip all jumps from short spider traps, back to front.
///
/// Clearing one trap moves the forced exit earlier, which can seal a new trap
/// upstream, so this iterates to a fixpoint. The chain is only applied when
/// it resolves healthily (no exit left, or a long structural loop kept);
/// a chain collapsing into the opening beats would trade one trap for an
/// intro loop, so then nothing is pruned at all.
fn prune_spider_traps(
    branches: &mut [Vec<Branch>],
    analysis: &Analysis,
    threshold: f32,
    config: &BranchConfig,
) {
    let Some(island_seconds) = config.island_threshold_seconds else {
        return;
    };
    if island_seconds <= 0.0 {
        return;
    }
    let durations: Vec<f64> = analysis.beats.iter().map(|beat| beat.duration).collect();
    let chain = resolve_spider_traps(branches, &durations, threshold, island_seconds);
    if !chain.healthy {
        return;
    }
    for step in &chain.steps {
        for &beat in step {
            branches[beat].clear();
        }
    }
}

/// Outcome of resolving spider traps to a fixpoint.
///
/// `steps` lists the cleared traps in order, but is only meaningful when
/// `healthy` is true: an unhealthy chain is reported for diagnostics and
/// must not be applied.
#[derive(Clone, Debug, PartialEq)]
pub struct TrapChain {
    pub steps: Vec<Vec<usize>>,
    pub healthy: bool,
}

/// Find the traps [`prune_spider_traps`] would clear, without clearing them.
#[must_use]
pub fn resolve_spider_traps(
    branches: &[Vec<Branch>],
    durations: &[f64],
    threshold: f32,
    island_seconds: f64,
) -> TrapChain {
    let mut working = branches.to_vec();
    let mut steps = Vec::new();
    let healthy = loop {
        let trap = spider_trap_beats(&working, threshold);
        if trap.is_empty() {
            break true;
        }
        if trap.contains(&0) {
            break false;
        }
        let seconds: f64 = trap.iter().map(|&beat| durations[beat]).sum();
        if seconds >= island_seconds {
            break true;
        }
        for &beat in &trap {
            working[beat].clear();
        }
        steps.push(trap);
    };
    TrapChain { steps, healthy }
}

/// Beats the walk can reach from the forced exit and return from: the spider
/// trap. Empty when no forced exit exists. Always contiguous, ending at the
/// exit, since every beat between the trap's start and the exit reaches the
/// exit sequentially and is reached back the same way.
fn spider_trap_beats(branches: &[Vec<Branch>], threshold: f32) -> Vec<usize> {
    let Some(exit) = forced_exit_source(branches, threshold) else {
        return Vec::new();
    };
    let successors = available_successors(branches, threshold, exit);
    let descendants = reachable(&successors, exit);
    let mut predecessors: Vec<Vec<usize>> = vec![Vec::new(); successors.len()];
    for (source, targets) in successors.iter().enumerate() {
        for &target in targets {
            predecessors[target].push(source);
        }
    }
    let ancestors = reachable(&predecessors, exit);
    descendants
        .into_iter()
        .enumerate()
        .filter_map(|(beat, descendant)| (descendant && ancestors[beat]).then_some(beat))
        .collect()
}

fn forced_exit_source(branches: &[Vec<Branch>], threshold: f32) -> Option<usize> {
    branches
        .iter()
        .enumerate()
        .rev()
        .find_map(|(source, items)| {
            items
                .iter()
                .any(|branch| branch.destination < source && branch.distance <= threshold)
                .then_some(source)
        })
}

/// Moves the planner can actually take: sequential steps (the forced exit
/// must jump instead), the wrap at the physical end, and every jump except
/// the ones the exit logic zeroes (skipping the exit, or relaxed at it).
fn available_successors(branches: &[Vec<Branch>], threshold: f32, exit: usize) -> Vec<Vec<usize>> {
    let count = branches.len();
    let mut successors = vec![Vec::new(); count];
    for (source, items) in branches.iter().enumerate() {
        if source != exit {
            successors[source].push(if source + 1 < count { source + 1 } else { 0 });
        }
        for branch in items {
            let destination = branch.destination;
            if destination >= count {
                continue;
            }
            let skips_exit = source <= exit && destination >= exit;
            let relaxed_at_exit = source == exit && branch.distance > threshold;
            if !skips_exit && !relaxed_at_exit {
                successors[source].push(destination);
            }
        }
    }
    successors
}

fn reachable(adjacency: &[Vec<usize>], start: usize) -> Vec<bool> {
    let mut seen = vec![false; adjacency.len()];
    let mut stack = vec![start];
    seen[start] = true;
    while let Some(current) = stack.pop() {
        for &next in &adjacency[current] {
            if !seen[next] {
                seen[next] = true;
                stack.push(next);
            }
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Beat;

    #[test]
    fn bounded_selection_matches_stable_sort_and_truncate() {
        let distances = [3.0, 1.0, 2.0, 1.0, f32::NAN, -0.0, 0.0, 0.5];
        for limit in 0..=distances.len() {
            let mut expected: Vec<_> = distances
                .iter()
                .enumerate()
                .map(|(destination, &distance)| Branch {
                    destination,
                    distance,
                })
                .collect();
            expected.sort_by(|left, right| left.distance.total_cmp(&right.distance));
            expected.truncate(limit);

            let mut actual = Vec::new();
            for (destination, &distance) in distances.iter().enumerate() {
                retain_nearest(
                    &mut actual,
                    Branch {
                        destination,
                        distance,
                    },
                    limit,
                );
            }

            assert_eq!(
                actual
                    .iter()
                    .map(|branch| (branch.destination, branch.distance.to_bits()))
                    .collect::<Vec<_>>(),
                expected
                    .iter()
                    .map(|branch| (branch.destination, branch.distance.to_bits()))
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn graph_contains_a_backward_escape() {
        let beats = (0..24)
            .map(|index| Beat {
                index,
                start: index as f64 / 2.0,
                duration: 0.5,
                features: Features {
                    chroma: [index.rem_euclid(4) as f32 / 4.0; 12],
                    ..Features::default()
                },
                start_features: Features {
                    chroma: [index.rem_euclid(4) as f32 / 4.0; 12],
                    ..Features::default()
                },
            })
            .collect();
        let analysis = Analysis {
            duration: 12.0,
            sample_rate: 44_100,
            channels: 2,
            tempo: 120.0,
            beats,
        };
        let graph = BranchGraph::build(&analysis, &BranchConfig::default());
        assert!(
            graph
                .branches
                .iter()
                .enumerate()
                .any(|(source, items)| { items.iter().any(|branch| branch.destination < source) })
        );
    }

    #[test]
    fn emergency_branch_must_span_a_quarter_of_the_track() {
        let mut candidates = vec![vec![]; 100];
        candidates[90] = vec![
            Branch {
                destination: 82,
                distance: 0.05,
            },
            Branch {
                destination: 40,
                distance: 0.15,
            },
        ];
        let mut branches = vec![vec![]; 100];
        branches[90].push(candidates[90][0].clone());

        ensure_long_backward_branch(&candidates, &mut branches, 0.1);

        assert_eq!(branches[90].len(), 2);
        assert_eq!(branches[90][1].destination, 40);
    }

    fn branch_to(destination: usize) -> Vec<Branch> {
        vec![Branch {
            destination,
            distance: 0.1,
        }]
    }

    fn branch_to_dist(destination: usize, distance: f32) -> Vec<Branch> {
        vec![Branch {
            destination,
            distance,
        }]
    }

    #[test]
    fn segments_split_where_no_jump_crosses() {
        let branches = vec![vec![], branch_to(3), vec![], branch_to(1), vec![], vec![]];
        assert_eq!(jump_segments(&branches), vec![0..1, 1..4, 4..5, 5..6]);
    }

    #[test]
    fn long_jump_seals_every_boundary_between_its_beats() {
        let mut branches: Vec<Vec<Branch>> = vec![vec![]; 5];
        branches[0] = branch_to(4);
        assert_eq!(jump_segments(&branches), vec![0..5]);
    }

    #[test]
    fn forced_exit_ignores_relaxed_edges() {
        let mut branches: Vec<Vec<Branch>> = vec![vec![]; 6];
        branches[3] = branch_to_dist(1, 0.9);
        branches[5] = branch_to(2);
        assert_eq!(forced_exit_source(&branches, 0.5), Some(5));

        branches[5].clear();
        assert_eq!(forced_exit_source(&branches, 0.5), None);
    }

    #[test]
    fn spider_trap_finds_closed_beats() {
        // Exit 7 seals beats 4..=7: the only way out would cross the exit.
        let mut branches: Vec<Vec<Branch>> = vec![vec![]; 10];
        branches[2] = branch_to(0);
        branches[6] = branch_to(4);
        branches[7] = branch_to(5);
        assert_eq!(spider_trap_beats(&branches, 0.5), vec![4, 5, 6, 7]);
    }

    #[test]
    fn spider_trap_is_empty_without_forced_exit() {
        let mut branches: Vec<Vec<Branch>> = vec![vec![]; 10];
        branches[1] = branch_to(5);
        branches[3] = branch_to_dist(1, 0.9);
        assert!(spider_trap_beats(&branches, 0.5).is_empty());
    }

    fn flat(chain: &TrapChain) -> Vec<usize> {
        let mut beats: Vec<usize> = chain.steps.iter().flatten().copied().collect();
        beats.sort_unstable();
        beats
    }

    #[test]
    fn resolve_prunes_trap_chain_to_healthy_loop() {
        // Trap [5..=9] clears first, then [1..=2]; no exit is left, so the
        // walk plays through and wraps. The unrelated island [12..=16] keeps
        // its jump throughout.
        let mut branches: Vec<Vec<Branch>> = vec![vec![]; 20];
        branches[2] = branch_to(1);
        branches[9] = branch_to(5);
        branches[12] = branch_to(16);
        let chain = resolve_spider_traps(&branches, &[1.0; 20], 0.5, 6.0);
        assert!(chain.healthy);
        assert_eq!(flat(&chain), vec![1, 2, 5, 6, 7, 8, 9]);
        assert_eq!(branches[12].len(), 1);
    }

    #[test]
    fn resolve_reverts_chain_collapsing_to_intro() {
        // Clearing [4..=7] would seal [0..=2] next: an intro loop. Report the
        // chain, but mark it so the caller prunes nothing.
        let branches: Vec<Vec<Branch>> = {
            let mut branches: Vec<Vec<Branch>> = vec![vec![]; 10];
            branches[2] = branch_to(0);
            branches[6] = branch_to(4);
            branches[7] = branch_to(5);
            branches
        };
        let chain = resolve_spider_traps(&branches, &[1.0; 10], 0.5, 10.0);
        assert!(!chain.healthy);
        assert_eq!(chain.steps, vec![vec![4, 5, 6, 7]]);
    }

    #[test]
    fn resolve_keeps_long_structural_loop() {
        // The [5..=9] trap spans five seconds: a loop worth keeping.
        let branches: Vec<Vec<Branch>> = {
            let mut branches: Vec<Vec<Branch>> = vec![vec![]; 20];
            branches[2] = branch_to(1);
            branches[9] = branch_to(5);
            branches[12] = branch_to(16);
            branches
        };
        let chain = resolve_spider_traps(&branches, &[1.0; 20], 0.5, 3.0);
        assert!(chain.healthy);
        assert!(chain.steps.is_empty());
    }
}
