# eternal-core

Audio analysis and branching playback engine for [Eternal Jukebox](https://github.com/404Simon/eternal-jukebox-rs).

`eternal-core` turns an audio file into a graph of musically similar beats and then walks that graph forever. It is the engine shared by the [`eternal-tui`](https://crates.io/crates/eternal-tui) terminal player and the [WebAssembly browser app](https://404simon.github.io/eternal-jukebox-rs/), and it compiles for both native and `wasm32-unknown-unknown`.

The pipeline is four stages:

1. **Decode**: interleaved floating-point PCM via [Symphonia](https://github.com/pdeljanov/Symphonia), with an Opus adapter. On WebAssembly the browser decodes instead and hands in raw samples.
2. **Analyse**: spectral flux and autocorrelation build a beat grid refined between FFT hops; each beat is described by chroma, spectral shape, loudness, and onset strength.
3. **Graph**: beats at the same bar position are compared and accepted as branch edges when they match closely at the transition. Spider traps (dead-end regions) are pruned.
4. **Plan**: a seeded, weighted infinite random walk over the graph, with novelty and coverage weighting so it keeps escaping its own loops.

## Usage

```rust
use eternal_core::{
    analyse, decode, AnalysisConfig, BranchConfig, BranchGraph, PlaybackPlanner,
};

let audio = decode("song.mp3")?;
let analysis = analyse(&audio, &AnalysisConfig::default())?;
let graph = BranchGraph::build(&analysis, &BranchConfig::default());

let mut planner = PlaybackPlanner::new(graph);
while let Some(step) = planner.next_step() {
    let beat = &analysis.beats[step.beat];
    println!("{} at {:.3}s", step.beat, beat.start);
}
```

Use `PlaybackPlanner::with_seed` for a reproducible walk, and `next_probabilities` to inspect the live weighting before each step.
