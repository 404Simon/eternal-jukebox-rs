# Eternal Jukebox RS

A local, dependency-light reimplementation of the Eternal Jukebox in Rust. It
analyses an audio file, finds musically similar beats, and keeps playback going
by jumping between them.

The project is split into two crates:

- `eternal-core`: decoding, local beat/feature analysis, graph construction,
  and the infinite playback planner.
- `eternal-tui`: the interactive `eternal` terminal application and audio output.

## Quick start

```sh
cargo run --release -p eternal-tui -- song.mp3
```

MP3 and Opus files are supported, including Opus audio in Ogg, WebM, and
Matroska containers.

Press Ctrl-C to stop. The first start compiles the release binary; to install
it on your path instead, run:

```sh
cargo install --path crates/eternal-tui
eternal song.mp3
```

The interactive Ratatui dashboard shows the audible and queued beats, live
stitching, recent jumps, and the planner's adaptive transition odds.

Linux builds need ALSA development headers (`libasound2-dev` on Debian/Ubuntu,
`alsa-lib-devel` on Fedora). Opus support bundles libopus and needs CMake and a C
compiler when building. `ffmpeg` is not needed at runtime.

## Commands

```text
eternal <FILE> [--threshold <DISTANCE>] [--seed <NUMBER>]
```

`eternal` decodes and analyses the entire file, then continuously queues individual
beats. It usually plays the next beat, occasionally selects a similar beat, and
strongly prefers a backward transition before reaching the end. `--seed` makes
those choices reproducible. A lower `--threshold` permits fewer, closer matches.

## Architecture

The implementation has no dependency on the old Kotlin server, Spotify audio
analysis, or browser JavaScript:

- `audio` decodes interleaved floating-point PCM with Symphonia, using a
  libopus-backed Symphonia adapter for Opus.
- `analysis` uses spectral flux and autocorrelation for the beat grid, then
  extracts chroma, spectral shape, loudness and onset strength for every beat.
- `graph` ports the original nearest-neighbour idea: only same-position beats
  are compared, attacks and neighbouring beats contribute to the transition
  score, a dynamic threshold targets useful sparsity, and a final branch
  boundary prevents playback from falling off the end.
- `planner` performs a weighted infinite random walk. Used transitions and
  frequently visited destinations gradually lose weight, so repeated loops
  make unexplored branches and the sequential path more likely.
- `eternal-tui` queues short, edge-faded PCM slices through Rodio for gapless
  live playback.

The local analyser is intentionally deterministic and self-contained. Its beat
tracking is best on music with a steady pulse; unusual metres, strong tempo
changes, or ambient material may benefit from an explicit `--threshold`, but a
threshold cannot repair an incorrectly detected beat grid.

See [Playback planning](docs/playback-planning.md) for a complete, approachable
description of the random walker and its weighting rules.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## License

MIT
