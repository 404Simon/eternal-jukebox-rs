# Playback simulator

`eternal-debug` analyses a track and runs the planner without audio output. It
uses seed `1474317007` by default and caches analysis under
`target/eternal-debug-cache`, making repeated planner experiments fast.

```sh
cargo run --release -p eternal-debug -- song.mp3
```

The default is four consecutive seeds and 20,000 planned beats per seed.
Useful options are:

```text
--seed N                 first seed
--runs N                 consecutive seeds
--steps N                beats per run
--inspect BEAT           branches near a beat; repeatable
--edge SOURCE:DEST       inspect one exact candidate; repeatable
--max-branches N         graph candidate experiment
--branch-fraction F      adaptive-threshold experiment
--threshold D            fixed-threshold experiment
--no-cache               repeat audio analysis
```

Output is deliberately compact:

- `TRACK`: beat grid and accepted graph density;
- `GRAPH longest` / `best_long`: longest and closest quarter-track backward
  edges as `source>destination:distance`;
- `BRANCH`: accepted destinations and distances near inspected beats;
- `cover`: beats reached at least once;
- `cv`: coefficient of variation of beat visits (lower is more even);
- `q%`: visit shares for the four track quarters;
- `hot`: hottest 5%-wide beat range and its share of all playback;
- `jumps`: total/backward/long-backward transitions;
- `wraps`: times playback reached the physical end and restarted;
- `local_max`: longest confinement to roughly one-twelfth of the track, using
  a 64-beat observation window.

Compare several seeds before accepting planner changes. A useful change lowers
`cv`, `hot`, and `local_max` without making `wraps` approach the natural
non-looping rate (`steps / beats`) or admitting poor-distance graph edges.
