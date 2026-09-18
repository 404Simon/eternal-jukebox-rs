use serde::{Deserialize, Serialize};

use crate::{Analysis, Features};

#[derive(Clone, Debug)]
pub struct BranchConfig {
    pub maximum_branches: usize,
    pub minimum_separation: usize,
    pub target_branch_fraction: f32,
    pub threshold: Option<f32>,
}

impl Default for BranchConfig {
    fn default() -> Self {
        Self {
            maximum_branches: 4,
            minimum_separation: 4,
            target_branch_fraction: 1.0 / 6.0,
            threshold: None,
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
                let mut nearest: Vec<_> = analysis
                    .beats
                    .iter()
                    .filter(|destination| {
                        source.index.abs_diff(destination.index) >= config.minimum_separation
                            && source.index % 4 == destination.index % 4
                    })
                    .map(|destination| Branch {
                        destination: destination.index,
                        distance: feature_distance(
                            &source.features,
                            &destination.features,
                            &normalisation,
                        ),
                    })
                    .collect();
                nearest.sort_by(|left, right| left.distance.total_cmp(&right.distance));
                nearest.truncate(config.maximum_branches);
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

        ensure_long_backward_branch(&candidates, &mut branches);
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
    let loudness = (left.loudness_db - right.loudness_db).abs() / norm.loudness;
    let onset = (left.onset_strength - right.onset_strength).abs() / norm.onset;
    10.0 * chroma + 4.0 * timbre + 2.0 * loudness + onset
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

fn ensure_long_backward_branch(candidates: &[Vec<Branch>], branches: &mut [Vec<Branch>]) {
    let best = candidates
        .iter()
        .enumerate()
        .flat_map(|(source, items)| {
            items
                .iter()
                .filter(move |branch| branch.destination < source)
                .map(move |branch| (source - branch.destination, source, branch))
        })
        .max_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| right.2.distance.total_cmp(&left.2.distance))
        });
    if let Some((_, source, branch)) = best
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Beat;

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
}
