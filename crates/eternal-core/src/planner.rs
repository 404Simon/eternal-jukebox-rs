use rand::{Rng, SeedableRng, rngs::SmallRng};

use crate::BranchGraph;

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
        Self {
            graph,
            current: None,
            branch_chance: 0.18,
            minimum_branch_chance: 0.18,
            maximum_branch_chance: 0.50,
            branch_chance_delta: 0.018,
            next_branch,
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
        let source = sequential.min(self.graph.branches.len() - 1);
        let must_jump =
            source >= self.graph.last_branch_point || sequential >= self.graph.branches.len();
        self.branch_chance =
            (self.branch_chance + self.branch_chance_delta).min(self.maximum_branch_chance);
        let should_jump = must_jump || self.rng.random::<f32>() < self.branch_chance;

        let (beat, jumped_from) = if should_jump && !self.graph.branches[source].is_empty() {
            let branch_index = self.next_branch[source] % self.graph.branches[source].len();
            self.next_branch[source] += 1;
            self.branch_chance = self.minimum_branch_chance;
            (
                self.graph.branches[source][branch_index].destination,
                Some(source),
            )
        } else {
            (source, None)
        };
        self.current = Some(beat);
        Some(Step { beat, jumped_from })
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
}
