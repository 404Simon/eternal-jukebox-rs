# Eternal Jukebox RS

Make a song play forever. Eternal Jukebox analyses an audio file, finds beats that sound alike, and keeps playback going by jumping between them.

This is a from-scratch Rust implementation inspired by [EternalBox/EternalJukebox](https://github.com/EternalBox/EternalJukebox). It started as a port and has since grown into something bigger: analysis and playback run entirely on your machine, the planner was rewritten to loop more evenly, and the same engine now powers both a terminal player and a browser app.

**[Try the browser demo](https://404simon.github.io/eternal-jukebox-rs/)**, it runs entirely in your browser, nothing is uploaded.

## Install and run

```sh
cargo install eternal-tui
eternal song.mp3
```

Or straight from a checkout:

```sh
cargo run --release -p eternal-tui -- song.mp3
```

MP3 and Opus are supported, including Opus in Ogg, WebM, and Matroska containers. Press Ctrl-C to stop.

Linux builds need ALSA headers (`libasound2-dev` on Debian/Ubuntu, `alsa-lib-devel` on Fedora). Opus support bundles libopus and needs CMake and a C compiler at build time. `ffmpeg` is not needed at runtime.

## Controls

```text
eternal <FILE> [--threshold <DISTANCE>] [--seed <NUMBER>]
eternal play | pause | toggle | status
```

The TUI shows the audible and queued beats, live stitching, recent jumps, and the planner's current transition odds. A running player can be controlled from another terminal, and on Linux MPRIS adds desktop media keys and commands such as `playerctl --player=eternal play-pause`. Remote control needs a shared session D-Bus; local playback works without one.

`--seed` makes the jump choices reproducible, and `--threshold` tunes how close a match has to be. A lower threshold means fewer, tighter jumps.

## What's different from the original

- **No server, no cloud analysis.** Decoding, beat detection, and feature extraction all happen locally, no Spotify/ECHO NEST API, no Kotlin backend.
- **A stronger planner.** A seeded, reproducible random walk that escapes its own loops: used transitions and over-visited sections lose weight over time, and dead-end regions are pruned so playback can't get stuck.
- **Better beat tracking.** Tempo is refined between FFT hops and phase-fit across the whole track, fixing the doubled-tempo mistakes the old integer-hop estimator made on drum & bass.
- **Two frontends, one engine.** The same `eternal-core` drives the native TUI and a WebAssembly browser app.
- **Built to be measured.** A deterministic playback simulator measures coverage and looping over millions of planned beats.

## Layout

| Crate           | What it is                                              |
| --------------- | ------------------------------------------------------- |
| `eternal-core`  | Decoding, analysis, graph construction, and the planner |
| `eternal-tui`   | The `eternal` terminal player (Ratatui + Rodio + MPRIS) |
| `eternal-web`   | WebAssembly bindings for the browser app                |
| `eternal-debug` | Internal playback simulator (not published)             |

The `web/` directory holds the Vite + React client that plays audio through the Web Audio API.

## How it works

1. **Decode** the file to floating-point PCM with Symphonia.
2. **Analyse**: spectral flux and autocorrelation build the beat grid; each beat gets chroma, spectral shape, loudness, and onset strength.
3. **Graph**: only beats at the same position are compared, and accepted as branches when they sound alike at the transition.
4. **Plan**: a weighted infinite random walk picks the sequential edge or a branch, using novelty and coverage weighting to keep things fresh.

The [playback planning](docs/playback-planning.md) guide describes the walker in full; the [simulator](docs/debug-simulator.md) covers how it is measured.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# MPRIS integration test (needs dbus-run-session and playerctl)
dbus-run-session -- cargo test -p eternal-tui mpris_round_trip -- --ignored
```

### Browser app

```sh
cargo install wasm-pack --locked
cd web
pnpm install
pnpm wasm
pnpm dev
```

`pnpm build` rebuilds the WebAssembly package, type-checks the app, and writes the production site to `web/dist`. Analysis runs in a Web Worker so long tracks don't freeze the UI.

## Credits

Built on the original [Eternal Jukebox](https://github.com/EternalBox/EternalJukebox) by EternalBox and its contributors.
