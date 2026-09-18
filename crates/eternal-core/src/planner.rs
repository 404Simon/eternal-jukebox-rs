use rand::{Rng, SeedableRng, rngs::SmallRng};

use crate::BranchGraph;

const BRANCH_COOLDOWN_STEPS: u64 = 64;

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
    next_branch: Vec<usize>,
    last_branch_use: Vec<Vec<Option<u64>>>,
    steps: u64,
    playing_outro: bool,
    rng: SmallRng,
}

impl PlaybackPlanner {
    #[must_use]
    pub fn new(graph: BranchGraph) -> Self {
        Self::with_seed(graph, rand::rng().random())
    }

    #[must_use]
    pub fn with_seed(graph: BranchGraph, seed: u64) -> Self {
        let next_branch = vec![0; graph.branches.len()];
        let last_branch_use = graph
            .branches
            .iter()
            .map(|branches| vec![None; branches.len()])
            .collect();
        Self {
            graph,
            current: None,
            branch_chance: 0.18,
            minimum_branch_chance: 0.18,
            maximum_branch_chance: 0.50,
            branch_chance_delta: 0.018,
            next_branch,
            last_branch_use,
            steps: 0,
            playing_outro: false,
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
        self.steps += 1;
        let sequential = self.current.map_or(0, |current| current + 1);
        if sequential >= self.graph.branches.len() {
            let previous = self.current;
            self.current = Some(0);
            self.playing_outro = false;
            self.branch_chance = self.minimum_branch_chance;
            return Some(Step {
                beat: 0,
                jumped_from: previous,
            });
        }
        let source = sequential.min(self.graph.branches.len() - 1);
        let must_jump = !self.playing_outro && source >= self.graph.last_branch_point;
        self.branch_chance =
            (self.branch_chance + self.branch_chance_delta).min(self.maximum_branch_chance);
        let should_jump = must_jump || self.rng.random::<f32>() < self.branch_chance;

        let available = self.available_branch(source);
        let selected = should_jump.then_some(available).flatten();
        let (beat, jumped_from) = if let Some(branch_index) = selected {
            self.next_branch[source] = branch_index + 1;
            self.last_branch_use[source][branch_index] = Some(self.steps);
            self.branch_chance = self.minimum_branch_chance;
            let destination = self.graph.branches[source][branch_index].destination;
            (destination, Some(source))
        } else {
            if must_jump {
                self.playing_outro = true;
            }
            (source, None)
        };
        self.current = Some(beat);
        Some(Step { beat, jumped_from })
    }

    fn available_branch(&mut self, source: usize) -> Option<usize> {
        let branch_count = self.graph.branches[source].len();
        if branch_count == 0 {
            return None;
        }
        let first = self.next_branch[source] % branch_count;
        (0..branch_count)
            .map(|offset| (first + offset) % branch_count)
            .find(|&index| {
                self.last_branch_use[source][index]
                    .is_none_or(|used| self.steps - used >= BRANCH_COOLDOWN_STEPS)
            })
    }
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
    fn planner_plays_outro_when_escape_is_cooling_down() {
        let graph = BranchGraph {
            branches: vec![
                vec![],
                vec![],
                vec![Branch {
                    destination: 0,
                    distance: 0.0,
                }],
                vec![],
            ],
            threshold: 0.0,
            last_branch_point: 2,
        };
        let steps: Vec<_> = PlaybackPlanner::with_seed(graph, 42).take(10).collect();
        let beats: Vec<_> = steps.iter().map(|step| step.beat).collect();
        assert_eq!(beats, vec![0, 1, 0, 1, 2, 3, 0, 1, 2, 3]);
        assert_eq!(steps[6].jumped_from, Some(3));
    }
}
