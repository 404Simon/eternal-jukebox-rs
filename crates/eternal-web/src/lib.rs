use eternal_core::{AnalysisConfig, Audio, BranchConfig, BranchGraph, PlaybackPlanner, analyse};
use serde::Serialize;
use wasm_bindgen::prelude::*;

#[derive(Serialize)]
struct PreparedTrack {
    analysis: eternal_core::Analysis,
    graph: BranchGraph,
}

/// Analyse interleaved PCM and construct its transition graph.
#[wasm_bindgen]
pub fn prepare_track(
    samples: Vec<f32>,
    sample_rate: u32,
    channels: u16,
    threshold: Option<f32>,
) -> Result<String, JsError> {
    if samples.is_empty() || sample_rate == 0 || channels == 0 {
        return Err(JsError::new("audio contains no samples"));
    }
    let audio = Audio::new(samples, sample_rate, channels);
    let analysis = analyse(&audio, &AnalysisConfig::default())
        .map_err(|error| JsError::new(&error.to_string()))?;
    let graph = BranchGraph::build(
        &analysis,
        &BranchConfig {
            threshold,
            ..BranchConfig::default()
        },
    );
    serde_json::to_string(&PreparedTrack { analysis, graph })
        .map_err(|error| JsError::new(&error.to_string()))
}

/// Stateful adaptive walk exposed as a compact JSON boundary.
#[wasm_bindgen]
pub struct WebPlanner {
    inner: PlaybackPlanner,
}

#[wasm_bindgen]
impl WebPlanner {
    #[wasm_bindgen(constructor)]
    pub fn new(graph_json: &str, seed: Option<u64>) -> Result<WebPlanner, JsError> {
        let graph: BranchGraph =
            serde_json::from_str(graph_json).map_err(|error| JsError::new(&error.to_string()))?;
        let inner = seed.map_or_else(
            || PlaybackPlanner::new(graph.clone()),
            |value| PlaybackPlanner::with_seed(graph.clone(), value),
        );
        Ok(Self { inner })
    }

    pub fn plan_next(&mut self) -> Result<String, JsError> {
        let probabilities = self.inner.next_probabilities();
        let step = self
            .inner
            .next_step()
            .ok_or_else(|| JsError::new("the playback graph contains no beats"))?;
        let probability = probabilities
            .iter()
            .find(|choice| {
                choice.destination == step.beat
                    && choice.is_sequential == step.jumped_from.is_none()
            })
            .map_or(1.0, |choice| choice.probability);
        serde_json::to_string(&(step.beat, step.jumped_from, probability))
            .map_err(|error| JsError::new(&error.to_string()))
    }

    pub fn continue_from(&mut self, beat: usize) -> bool {
        self.inner.continue_from(beat)
    }

    pub fn probabilities(&self) -> Result<String, JsError> {
        serde_json::to_string(&self.inner.next_probabilities())
            .map_err(|error| JsError::new(&error.to_string()))
    }
}
