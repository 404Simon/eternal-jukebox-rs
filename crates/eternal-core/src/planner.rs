use rand::{Rng, SeedableRng, rngs::SmallRng};
use serde::Serialize;

use crate::BranchGraph;

// Compare destinations over roughly five percent of the track. This catches a
// repeated verse/chorus-sized region rather than only the exact landing beat.
const COVERAGE_WINDOW_DIVISOR: usize = 20;
const LOOP_CLOSURE_STRENGTH: f32 = 0.35;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub beat: usize,
    pub jumped_from: Option<usize>,
}

/// A possible choice for the next beat, using the planner's live novelty state.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TransitionProbability {
    pub source: usize,
    pub destination: usize,
    pub probability: f32,
    pub distance: Option<f32>,
    pub is_sequential: bool,
}

/// Stateful infinite walk over a beat graph.
pub struct PlaybackPlanner {
    graph: BranchGraph,
    loop_exit: Option<usize>,
    current: Option<usize>,
    branch_chance: f32,
    minimum_branch_chance: f32,
    maximum_branch_chance: f32,
    branch_chance_delta: f32,
    sequential_uses: Vec<u32>,
    branch_uses: Vec<Vec<u32>>,
    beat_heat: Vec<f32>,
    rng: SmallRng,
}

impl PlaybackPlanner {
    #[must_use]
    pub fn new(graph: BranchGraph) -> Self {
        Self::with_seed(graph, rand::rng().random())
    }

    #[must_use]
    pub fn with_seed(graph: BranchGraph, seed: u64) -> Self {
        // Only an ordinary, quality-qualified backward edge may become
        // mandatory. A relaxed graph fallback must never force a bad splice.
        let loop_exit = graph
            .branches
            .iter()
            .enumerate()
            .rev()
            .find_map(|(source, branches)| {
                branches
                    .iter()
                    .any(|branch| branch.destination < source && branch.distance <= graph.threshold)
                    .then_some(source)
            });
        let sequential_uses = vec![0; graph.branches.len()];
        let branch_uses = graph
            .branches
            .iter()
            .map(|branches| vec![0; branches.len()])
            .collect();
        let beat_heat = vec![0.0; graph.branches.len()];
        Self {
            graph,
            loop_exit,
            current: None,
            branch_chance: 0.18,
            minimum_branch_chance: 0.18,
            maximum_branch_chance: 0.50,
            branch_chance_delta: 0.018,
            sequential_uses,
            branch_uses,
            beat_heat,
            rng: SmallRng::seed_from_u64(seed),
        }
    }

    #[must_use]
    pub fn graph(&self) -> &BranchGraph {
        &self.graph
    }

    /// Continue planning after `beat`, preserving the planner's novelty state.
    ///
    /// This is useful when playback is repositioned independently of the walk.
    pub fn continue_from(&mut self, beat: usize) -> bool {
        if beat >= self.graph.branches.len() {
            return false;
        }
        self.current = Some(beat);
        true
    }

    /// Return the choices that the next call to [`Self::next_step`] will make.
    #[must_use]
    pub fn next_probabilities(&self) -> Vec<TransitionProbability> {
        if self.graph.branches.is_empty() {
            return Vec::new();
        }
        let sequential = self.current.map_or(0, |current| current + 1);
        if sequential >= self.graph.branches.len() {
            return vec![TransitionProbability {
                source: self.current.unwrap_or(0),
                destination: 0,
                probability: 1.0,
                distance: None,
                is_sequential: false,
            }];
        }

        let source = sequential;
        let branches = &self.graph.branches[source];
        if branches.is_empty() {
            return vec![TransitionProbability {
                source,
                destination: source,
                probability: 1.0,
                distance: None,
                is_sequential: true,
            }];
        }
        let next_branch_chance =
            (self.branch_chance + self.branch_chance_delta).min(self.maximum_branch_chance);
        let (sequential_weight, branch_weights) =
            self.transition_weights(source, next_branch_chance);
        let mut choices = Vec::with_capacity(branches.len() + 1);
        choices.push(TransitionProbability {
            source,
            destination: source,
            probability: sequential_weight,
            distance: None,
            is_sequential: true,
        });
        choices.extend(branches.iter().zip(branch_weights).map(|(branch, weight)| {
            TransitionProbability {
                source,
                destination: branch.destination,
                probability: weight,
                distance: Some(branch.distance),
                is_sequential: false,
            }
        }));
        let total = choices.iter().map(|choice| choice.probability).sum::<f32>();
        for choice in &mut choices {
            choice.probability /= total;
        }
        choices
    }

    pub fn next_step(&mut self) -> Option<Step> {
        if self.graph.branches.is_empty() {
            return None;
        }
        let sequential = self.current.map_or(0, |current| current + 1);
        if sequential >= self.graph.branches.len() {
            let previous = self.current;
            self.current = Some(0);
            self.record_visit(0);
            self.branch_chance = self.minimum_branch_chance;
            return Some(Step {
                beat: 0,
                jumped_from: previous,
            });
        }
        let source = sequential.min(self.graph.branches.len() - 1);
        self.branch_chance =
            (self.branch_chance + self.branch_chance_delta).min(self.maximum_branch_chance);
        let selected = self.select_transition(source);
        let (beat, jumped_from) = if let Some(branch_index) = selected {
            self.branch_uses[source][branch_index] += 1;
            self.branch_chance = self.minimum_branch_chance;
            let destination = self.graph.branches[source][branch_index].destination;
            (destination, Some(source))
        } else {
            self.sequential_uses[source] += 1;
            (source, None)
        };
        self.record_visit(beat);
        self.current = Some(beat);
        Some(Step { beat, jumped_from })
    }

    /// Shared by playback and its probability preview. Coverage can rank safe
    /// exits, but cannot outweigh the last opportunity to stay in the song.
    fn transition_weights(&self, source: usize, branch_chance: f32) -> (f32, Vec<f32>) {
        let force_branch = self.loop_exit == Some(source);
        let branch_probability = if force_branch { 1.0 } else { branch_chance };
        let sequential_weight = (1.0 - branch_probability)
            * novelty(self.sequential_uses[source])
            * self.coverage_novelty(source);
        let branch_base = branch_probability / self.graph.branches[source].len().max(1) as f32;
        let branch_weights: Vec<_> = self.graph.branches[source]
            .iter()
            .enumerate()
            .map(|(index, branch)| {
                let skips_exit = self
                    .loop_exit
                    .is_some_and(|exit| source <= exit && branch.destination >= exit);
                if skips_exit || (force_branch && branch.distance > self.graph.threshold) {
                    return 0.0;
                }
                branch_base
                    * novelty(self.branch_uses[source][index])
                    * self.coverage_novelty(branch.destination)
                    * self.loop_closure_novelty(source, branch.destination)
            })
            .collect();
        (sequential_weight, branch_weights)
    }

    fn select_transition(&mut self, source: usize) -> Option<usize> {
        if self.graph.branches[source].is_empty() {
            return None;
        }
        let (sequential_weight, branch_weights) =
            self.transition_weights(source, self.branch_chance);
        let total = sequential_weight + branch_weights.iter().sum::<f32>();
        let mut draw = self.rng.random_range(0.0..total);
        if draw < sequential_weight {
            return None;
        }
        draw -= sequential_weight;
        branch_weights
            .iter()
            .position(|weight| {
                if draw < *weight {
                    true
                } else {
                    draw -= *weight;
                    false
                }
            })
            .or_else(|| branch_weights.iter().rposition(|weight| *weight > 0.0))
    }

    /// Exponentially suppress destinations inside an overplayed section. The
    /// scale follows the song-wide mean, so the pressure is strong when a loop
    /// forms but relaxes as overall coverage accumulates.
    fn coverage_novelty(&self, destination: usize) -> f32 {
        if self.beat_heat.is_empty() {
            return 1.0;
        }
        let radius = (self.beat_heat.len() / COVERAGE_WINDOW_DIVISOR).max(2);
        let start = destination.saturating_sub(radius);
        let end = (destination + radius + 1).min(self.beat_heat.len());
        let local_mean = self.beat_heat[start..end].iter().sum::<f32>() / (end - start) as f32;
        let global_mean = self.beat_heat.iter().sum::<f32>() / self.beat_heat.len() as f32;
        (-local_mean / (global_mean + 1.0)).max(-20.0).exp()
    }

    /// A backward branch closes a loop over the beats between its destination
    /// and source. Leave its first traversal untouched, then gently reduce its
    /// weight while that same interval remains recently replayed.
    fn loop_closure_novelty(&self, source: usize, destination: usize) -> f32 {
        if destination >= source {
            return 1.0;
        }
        let interval = &self.beat_heat[destination..=source];
        let mean_heat = interval.iter().sum::<f32>() / interval.len() as f32;
        (-LOOP_CLOSURE_STRENGTH * (mean_heat - 1.0).max(0.0))
            .max(-20.0)
            .exp()
    }

    fn record_visit(&mut self, beat: usize) {
        // Scale memory to the track: heat halves after roughly half a song.
        let decay = 0.5_f32.powf(2.0 / self.beat_heat.len() as f32);
        for heat in &mut self.beat_heat {
            *heat *= decay;
        }
        self.beat_heat[beat] += 1.0;
    }
}

fn novelty(uses: u32) -> f32 {
    1.0 / ((uses + 1) as f32).sqrt()
}

impl Iterator for PlaybackPlanner {
    type Item = Step;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_step()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Branch;

    #[test]
    fn planner_never_runs_past_the_end() {
        let graph = BranchGraph {
            branches: vec![
                vec![],
                vec![],
                vec![Branch {
                    destination: 0,
                    distance: 0.0,
                }],
            ],
            threshold: 0.0,
            last_branch_point: 2,
        };
        let beats: Vec<_> = PlaybackPlanner::with_seed(graph, 42)
            .take(100)
            .map(|step| step.beat)
            .collect();
        assert_eq!(beats.len(), 100);
        assert!(beats.iter().all(|beat| *beat < 3));
    }

    #[test]
    fn used_edges_lose_weight_against_alternatives() {
        let graph = BranchGraph {
            branches: vec![
                vec![
                    Branch {
                        destination: 1,
                        distance: 0.0,
                    },
                    Branch {
                        destination: 2,
                        distance: 0.0,
                    },
                ],
                vec![],
                vec![],
            ],
            threshold: 0.0,
            last_branch_point: 0,
        };
        let mut planner = PlaybackPlanner::with_seed(graph, 42);
        planner.branch_uses[0] = vec![8, 0];
        let used = novelty(planner.branch_uses[0][0]);
        let unused = novelty(planner.branch_uses[0][1]);
        assert!(unused > used);
    }

    #[test]
    fn reported_probabilities_are_normalised() {
        let graph = BranchGraph {
            branches: vec![
                vec![
                    Branch {
                        destination: 1,
                        distance: 0.25,
                    },
                    Branch {
                        destination: 2,
                        distance: 0.5,
                    },
                ],
                vec![],
                vec![],
            ],
            threshold: 1.0,
            last_branch_point: 2,
        };
        let planner = PlaybackPlanner::with_seed(graph, 42);
        let choices = planner.next_probabilities();
        assert_eq!(choices.len(), 3);
        assert!(
            (choices.iter().map(|choice| choice.probability).sum::<f32>() - 1.0).abs()
                < f32::EPSILON
        );
        assert!(choices.iter().all(|choice| choice.probability > 0.0));
    }

    #[test]
    fn final_exit_rejects_forward_and_relaxed_edges_even_when_overplayed() {
        let graph = BranchGraph {
            branches: vec![
                vec![],
                vec![
                    Branch {
                        destination: 0,
                        distance: 0.1,
                    },
                    Branch {
                        destination: 2,
                        distance: 0.05,
                    },
                    Branch {
                        destination: 0,
                        distance: 0.3,
                    },
                ],
                vec![],
            ],
            threshold: 0.2,
            last_branch_point: 1,
        };
        let mut planner = PlaybackPlanner::with_seed(graph, 42);
        planner.beat_heat.fill(1_000.0);
        planner.branch_uses[1][0] = 1_000_000;
        assert!(planner.continue_from(0));
        let probabilities = planner.next_probabilities();
        assert!((probabilities[1].probability - 1.0).abs() < f32::EPSILON);
        for index in [0, 2, 3] {
            assert!(probabilities[index].probability.abs() < f32::EPSILON);
        }
        assert_eq!(
            planner.next_step(),
            Some(Step {
                beat: 0,
                jumped_from: Some(1)
            })
        );
    }

    #[test]
    fn forward_jumps_cannot_skip_the_last_safe_exit() {
        let graph = BranchGraph {
            branches: vec![
                vec![
                    Branch {
                        destination: 2,
                        distance: 0.1,
                    },
                    Branch {
                        destination: 3,
                        distance: 0.1,
                    },
                ],
                vec![],
                vec![Branch {
                    destination: 0,
                    distance: 0.1,
                }],
                vec![],
            ],
            threshold: 0.2,
            last_branch_point: 2,
        };
        for seed in 0..16 {
            let mut planner = PlaybackPlanner::with_seed(graph.clone(), seed);
            for _ in 0..1_000 {
                let probabilities = planner.next_probabilities();
                let step = planner.next_step().unwrap();
                assert!(step.beat < 2);
                assert!(probabilities.iter().any(|choice| {
                    choice.destination == step.beat
                        && choice.is_sequential == step.jumped_from.is_none()
                        && choice.probability > 0.0
                }));
            }
        }
    }

    #[test]
    fn missing_quality_qualified_exit_keeps_sequential_fallback() {
        for branches in [
            vec![vec![]; 3],
            vec![
                vec![],
                vec![Branch {
                    destination: 0,
                    distance: 0.3,
                }],
                vec![],
            ],
        ] {
            let graph = BranchGraph {
                branches,
                threshold: 0.2,
                last_branch_point: 1,
            };
            let mut planner = PlaybackPlanner::with_seed(graph, 42);
            assert_eq!(planner.loop_exit, None);
            assert!(planner.continue_from(0));
            assert!(planner.next_probabilities()[0].probability > 0.0);
            assert!(planner.take(1_000).any(|step| step.beat == 2));
        }
    }

    #[test]
    fn coverage_penalises_an_overplayed_region() {
        let graph = BranchGraph {
            branches: vec![vec![]; 100],
            threshold: 0.0,
            last_branch_point: 99,
        };
        let mut planner = PlaybackPlanner::with_seed(graph, 42);
        planner.beat_heat[40..61].fill(12.0);

        assert!(planner.coverage_novelty(50) < planner.coverage_novelty(10) * 0.1);
    }

    #[test]
    fn repeated_backward_loop_loses_weight() {
        let graph = BranchGraph {
            branches: vec![vec![]; 20],
            threshold: 0.0,
            last_branch_point: 19,
        };
        let mut planner = PlaybackPlanner::with_seed(graph, 42);

        planner.beat_heat[5..=15].fill(1.0);
        assert!((planner.loop_closure_novelty(15, 5) - 1.0).abs() < f32::EPSILON);
        planner.beat_heat[5..=15].fill(5.0);
        assert!(planner.loop_closure_novelty(15, 5) < 0.3);
        assert!((planner.loop_closure_novelty(5, 15) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn unplayed_beats_cool_and_become_attractive_again() {
        let graph = BranchGraph {
            branches: vec![vec![]; 100],
            threshold: 0.0,
            last_branch_point: 99,
        };
        let mut planner = PlaybackPlanner::with_seed(graph, 42);
        planner.beat_heat[40..61].fill(5.0);
        let hot_weight = planner.coverage_novelty(50);

        for _ in 0..400 {
            planner.record_visit(0);
        }

        assert!(planner.coverage_novelty(50) > hot_weight * 2.0);
        assert!(planner.coverage_novelty(50) > 0.9);
    }

    #[test]
    fn qualified_final_exit_prevents_the_outro() {
        let graph = BranchGraph {
            branches: vec![
                vec![],
                vec![Branch {
                    destination: 0,
                    distance: 0.0,
                }],
                vec![],
            ],
            threshold: 0.0,
            last_branch_point: 1,
        };
        let beats: Vec<_> = PlaybackPlanner::with_seed(graph, 7)
            .take(100)
            .map(|step| step.beat)
            .collect();
        assert!(!beats.contains(&2));
    }
}
