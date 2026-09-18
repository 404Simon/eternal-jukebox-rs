//! Core analysis and playback-planning logic for Eternal Jukebox.

pub mod analysis;
pub mod audio;
pub mod graph;
pub mod planner;

pub use analysis::{Analysis, AnalysisConfig, Beat, Features, analyse};
pub use audio::{Audio, AudioError, decode};
pub use graph::{Branch, BranchConfig, BranchGraph};
pub use planner::{PlaybackPlanner, Step};

