# eternal-tui

Terminal player for [Eternal Jukebox](https://github.com/404Simon/eternal-jukebox-rs): point it at an audio file and it plays forever, jumping between musically similar beats instead of reaching the end.

```sh
cargo install eternal-tui
eternal song.mp3
```

MP3 and Opus are supported, including Opus in Ogg, WebM, and Matroska containers. The TUI is built with [Ratatui](https://ratatui.rs) and audio plays through Rodio; press Ctrl-C to stop.

## Commands

```text
eternal <FILE> [--threshold <DISTANCE>] [--seed <NUMBER>]
eternal play | pause | toggle | status
```

The dashboard shows the audible and queued beats, live stitching, recent jumps, and the planner's current transition odds. A running player can be driven from another terminal, and on Linux MPRIS adds desktop media keys and commands such as `playerctl --player=eternal play-pause`. Remote control needs a shared session D-Bus; local playback works without one.

- `--seed N` makes the jump choices reproducible.
- `--threshold D` sets how close a match must be; lower means fewer, tighter
  jumps.
- `--island-threshold S` controls pruning of dead-end regions (0 disables it).

Linux builds need ALSA headers (`libasound2-dev` on Debian/Ubuntu, `alsa-lib-devel` on Fedora). Opus support bundles libopus and needs CMake and a C compiler at build time. `ffmpeg` is not needed at runtime.

The analysis and planning engine lives in [`eternal-core`](https://crates.io/crates/eternal-core); a browser frontend of the same engine is at <https://404simon.github.io/eternal-jukebox-rs/>.
