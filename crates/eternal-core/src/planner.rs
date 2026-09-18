use rand::{Rng, SeedableRng, rngs::SmallRng};

use crate::BranchGraph;

// Reaching the final safe branch should strongly favour another musical loop.
// The novelty weights below still let the outro win eventually.
const FINAL_BRANCH_PROBABILITY: f32 = 0.98;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub beat: usize,
    pub jumped_from: Option<usize>,
}

/// Stateful infinite walk over a beat graph.
pub struct PlaybackPlanner {
    graph: BranchGraph,
    current: Option<usize>,
    branch_chance: f32,
    minimum_branch_chance: f32,
    maximum_branch_chance: f32,
    branch_chance_delta: f32,
    sequential_uses: Vec<u32>,
    branch_uses: Vec<Vec<u32>>,
    beat_visits: Vec<u32>,
    rng: SmallRng,
}

impl PlaybackPlanner {
    #[must_use]
    pub fn new(graph: BranchGraph) -> Self {
        Self::with_seed(graph, rand::rng().random())
    }

    #[must_use]
    pub fn with_seed(graph: BranchGraph, seed: u64) -> Self {
        let sequential_uses = vec![0; graph.branches.len()];
        let branch_uses = graph
            .branches
            .iter()
            .map(|branches| vec![0; branches.len()])
            .collect();
        let beat_visits = vec![0; graph.branches.len()];
        Self {
            graph,
            current: None,
            branch_chance: 0.18,
            minimum_branch_chance: 0.18,
            maximum_branch_chance: 0.50,
            branch_chance_delta: 0.018,
            sequential_uses,
            branch_uses,
            beat_visits,
            rng: SmallRng::seed_from_u64(seed),
        }
    }

    #[must_use]
    pub fn graph(&self) -> &BranchGraph {
        &self.graph
    }

    pub fn next_step(&mut self) -> Option<Step> {
        if self.graph.branches.is_empty() {
            return None;
        }
        let sequential = self.current.map_or(0, |current| current + 1);
        if sequential >= self.graph.branches.len() {
            let previous = self.current;
            self.current = Some(0);
            self.beat_visits[0] += 1;
            self.branch_chance = self.minimum_branch_chance;
            return Some(Step {
                beat: 0,
                jumped_from: previous,
            });
        }
        let source = sequential.min(self.graph.branches.len() - 1);
        let force_branch = source == self.graph.last_branch_point;
        self.branch_chance =
            (self.branch_chance + self.branch_chance_delta).min(self.maximum_branch_chance);
        let selected = self.select_transition(source, force_branch);
        let (beat, jumped_from) = if let Some(branch_index) = selected {
            self.branch_uses[source][branch_index] += 1;
            self.branch_chance = self.minimum_branch_chance;
            let destination = self.graph.branches[source][branch_index].destination;
            (destination, Some(source))
        } else {
            self.sequential_uses[source] += 1;
            (source, None)
        };
        self.beat_visits[beat] += 1;
        self.current = Some(beat);
        Some(Step { beat, jumped_from })
    }

    /// Choose between normal continuation and all outgoing branches. Repeated
    /// transitions and frequently visited destinations lose weight but never
    /// become impossible, similar to a reinforced random walk in reverse.
    fn select_transition(&mut self, source: usize, force_branch: bool) -> Option<usize> {
        let branch_count = self.graph.branches[source].len();
        if branch_count == 0 {
            return None;
        }
        let branch_probability = if force_branch {
            FINAL_BRANCH_PROBABILITY
        } else {
            self.branch_chance
        };
        let sequential_weight = (1.0 - branch_probability)
            * novelty(self.sequential_uses[source])
            * destination_novelty(self.beat_visits[source]);
        let branch_base = branch_probability / branch_count as f32;
        let branch_weights: Vec<_> = self.graph.branches[source]
            .iter()
            .enumerate()
            .map(|(index, branch)| {
                branch_base
                    * novelty(self.branch_uses[source][index])
                    * destination_novelty(self.beat_visits[branch.destination])
            })
            .collect();
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
            .or(Some(branch_count - 1))
    }
}

fn novelty(uses: u32) -> f32 {
    1.0 / ((uses + 1) as f32).sqrt()
}

fn destination_novelty(visits: u32) -> f32 {
    1.0 / ((visits + 1) as f32).sqrt()
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
    fn final_branch_remains_likely_after_repetition() {
        let repeated_branch = FINAL_BRANCH_PROBABILITY * novelty(10) * destination_novelty(10);
        let fresh_outro = (1.0 - FINAL_BRANCH_PROBABILITY) * novelty(0) * destination_novelty(0);
        let branch_probability = repeated_branch / (repeated_branch + fresh_outro);
        assert!(branch_probability > 0.75);
    }

    #[test]
    fn forced_loop_eventually_plays_the_outro() {
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
        assert!(beats.contains(&2));
    }
}
