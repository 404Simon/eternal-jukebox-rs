# Eternal Jukebox RS

A local, dependency-light reimplementation of the Eternal Jukebox in Rust. It
analyses an audio file, finds musically similar beats, and keeps playback going
by jumping between them.

The project is split into two crates:

- `eternal-core`: decoding, local beat/feature analysis, graph construction,
  and the infinite playback planner.
- `eternal-cli`: the `eternal` command-line application and audio output.

## Quick start

```sh
cargo run --release -p eternal-cli -- play song.mp3
```

Press Ctrl-C to stop. The first start compiles the release binary; to install
it on your path instead, run:

```sh
cargo install --path crates/eternal-cli
eternal play song.mp3
```

Linux builds need ALSA development headers (`libasound2-dev` on Debian/Ubuntu,
`alsa-lib-devel` on Fedora). MP3 decoding and analysis are implemented in Rust;
`ffmpeg` is not needed at runtime.

## Commands

```text
eternal play <FILE> [--threshold <DISTANCE>] [--seed <NUMBER>]
eternal analyse <FILE> [--threshold <DISTANCE>] [-o analysis.json]
```

`play` decodes and analyses the entire file, then continuously queues individual
beats. It usually plays the next beat, occasionally selects a similar beat, and
forces a backward transition before reaching the end. `--seed` makes those
choices reproducible. A lower `--threshold` permits fewer, closer matches.

`analyse` writes the detected tempo, beat boundaries and features together with
the complete branch graph as JSON. This is useful for debugging or for another
frontend built on `eternal-core`.

## Architecture

The implementation has no dependency on the old Kotlin server, Spotify audio
analysis, or browser JavaScript:

- `audio` decodes interleaved floating-point PCM with Symphonia.
- `analysis` uses spectral flux and autocorrelation for the beat grid, then
  extracts chroma, spectral shape, loudness and onset strength for every beat.
- `graph` ports the original nearest-neighbour idea: only same-position beats
  are compared, attacks and neighbouring beats contribute to the transition
  score, a dynamic threshold targets useful sparsity, and a final branch
  boundary prevents playback from falling off the end.
- `planner` performs the infinite probabilistic walk and rotates alternatives so
  repeated visits do not always make the same jump.
- `eternal-cli` queues short, edge-faded PCM slices through Rodio for gapless
  live playback.

The local analyser is intentionally deterministic and self-contained. Its beat
tracking is best on music with a steady pulse; unusual metres, strong tempo
changes, or ambient material may benefit from an explicit `--threshold`, but a
threshold cannot repair an incorrectly detected beat grid.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## License

MIT
