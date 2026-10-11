# eternal-web

WebAssembly bindings for [Eternal Jukebox](https://github.com/404Simon/eternal-jukebox-rs).

This crate exposes the [`eternal-core`](https://crates.io/crates/eternal-core) analysis and planner to JavaScript through `wasm-bindgen`. It is intended for the browser frontend in the repository's `web/` directory, which decodes audio with the Web Audio API and runs analysis in a Web Worker.

```text
prepare_track(samples, sample_rate, channels, threshold?, island_threshold?) -> JSON
WebPlanner::new(graph_json, seed?)                                           -> planner
WebPlanner::plan_next()                                                      -> [beat, jumped_from, probability]
WebPlanner::continue_from(beat)                                              -> bool
WebPlanner::probabilities()                                                  -> JSON
```

Not published to crates.io; build it with `wasm-pack` as described in the
repository README.
