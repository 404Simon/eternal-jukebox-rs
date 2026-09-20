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
--max-wraps N            fail when any seed exceeds N physical restarts
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

## DnB regression comparison (2026-09-20)

Integer-hop tempo estimates produced 172.3 BPM for both DnB tracks. Predicting
each beat from the previous snapped onset accumulated errors across breakdowns.
In `I Want`, a repeated passage separated by 256 musical beats appeared only
255 indexed beats apart and failed the same-bar-position candidate filter,
despite a feature distance of 0.024.

The analysis now refines tempo between FFT hops and fits phase across the track.
Individual attacks snap locally to this grid without shifting subsequent beats.
The resulting tempos are 175.0 BPM (`I Want`), 174.0 BPM (`Jungle`) and 140.0 BPM
(`ALLEIN`). This alone reduces restarts but does not eliminate the planner's
deliberate sequential option at its last exit. The planner now takes a backward
edge there only if it meets the ordinary similarity threshold; no threshold
relaxation is used to enforce looping.

Before these changes, eight seeds with 20,000 beats each produced:

| Track | Long backward edges | Physical restarts per run |
| --- | ---: | ---: |
| ALLEIN | 19 | 29–33 |
| I Want (Dnb Tester) | 0 | 33–35 |
| 1991 – Jungle | 0 | 28–31 |

For a longer check, use 16 seeds starting at the default seed, 100,000 beats
each, and make restarts fail the command:

```sh
cargo run --release -p eternal-debug -- \
  '/home/simon/Music/Juju - 44 ME/04 - ALLEIN.mp3' \
  '/home/simon/Music/Drums+Base+Chill/I Want (Dnb Tester) PATREON EXCLUSIVE [2087301219].mp3' \
  '/home/simon/Music/Drums+Base+Chill/016 - 1991 - Jungle.flac' \
  --runs 16 --steps 100000 --max-wraps 0
```

Measured after the changes (1.6 million planned beats per track):

| Track | Long backward edges | Restarts across all runs | Longest local confinement |
| --- | ---: | ---: | ---: |
| ALLEIN | 19 | 0 | 0 |
| I Want (Dnb Tester) | 30 | 0 | 0 |
| 1991 – Jungle | 27 | 0 | 0 |

Analysis caches are versioned so earlier feature/grid data is recomputed. The
tracks themselves are local test inputs, not repository fixtures. Simulation
checks loop behaviour; it does not replace listening to the actual transitions.
Skipping the unmatched tail also reduces full-track coverage, so interpret `cv`
and `cover` alongside `wraps` and `local_max`, not as independent quality scores.
