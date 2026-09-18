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

Press Ctrl-C to stop. Use `eternal analyse song.mp3` to inspect or export the
analysis without playing it.

## License

MIT

