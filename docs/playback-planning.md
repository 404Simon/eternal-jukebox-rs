# Playback planning

The playback planner decides which beat should be heard next. Its goal is to
keep the song playing indefinitely without getting trapped in a short,
unchanging loop.

The short version is:

1. Continue to the next beat most of the time.
2. Occasionally replace that next beat with a musically similar beat elsewhere.
3. Remember which choices have already been used.
4. Gradually favour less-used choices and less-visited parts of the song.

This is a weighted random walk with negative reinforcement. It is inspired by
the exploration aspect of algorithms such as PageRank, but it is not an
implementation of PageRank: weights change during playback, and frequently
used paths lose weight rather than gain rank.

## The graph

Audio analysis produces a graph whose nodes are beats:

```text
                        jump
                       ┌─────┐
                       │     v
... → beat 20 → beat 21 → beat 22 → beat 23 → ...
          │
          └──────────────────────→ beat 68
                         jump
```

Every beat has an implicit sequential edge to the following beat. Some beats
also have explicit branch edges to beats that sound similar at the transition.
The graph builder has already rejected candidates with poor timing, pitch,
timbre, loudness, or phrase context. The planner therefore chooses only among
musically acceptable branches; it does not recalculate audio similarity.

The branch source is the beat that would normally play next. For example, after
playing beat 19, the planner considers the normal transition to beat 20 and all
branch edges originating at beat 20. Choosing `20 → 68` plays beat 68 instead
of beat 20.

## State remembered by the walker

During one playback session the planner records:

- how often the sequential path was selected at each source beat;
- how often every explicit branch edge was selected;
- the recent playback "heat" of every beat and its surrounding section;
- the current general probability of making a branch.

The state is kept in memory and starts at zero for every new invocation of the
CLI. It does not alter the analysis JSON or the audio file.

## Branch probability

Away from the end of the song, the general branch probability starts at 18%.
Every sequential beat raises it by 1.8 percentage points, up to a maximum of
50%. Taking any branch resets it to 18%.

This produces stretches of ordinary playback while making a jump progressively
more likely when no jump has happened recently.

At the last source with a backward edge within the normal similarity threshold,
the planner must take a qualified backward edge. Coverage weights choose among
these exits but cannot select the outro or an edge accepted only by the graph's
relaxed fallback. Forward jumps cannot land on or beyond this source, since
landing on it would skip its decision (the source is the next beat to play).

This leaves the unmatched tail after the final exit out of automatic playback.
If no qualified backward edge exists, or playback is manually repositioned
beyond that exit, sequential playback and the end-of-file restart remain the
fallback. No new transition is invented to enforce looping.

## Novelty weights

Each possible transition receives a weight. A transition used `u` times gets
this novelty factor:

```text
transition novelty = 1 / sqrt(u + 1)
```

A destination also receives a section-coverage factor. Playing a beat adds one
to its heat; on every subsequent beat, all heat cools exponentially. Its
half-life is half the track's beat count, so sections become attractive again
after the walk spends meaningful time elsewhere without forgetting a long loop
before that loop even completes. The planner averages heat in a window spanning
roughly 10% of the song around a destination, then compares it with the
song-wide mean:

```text
coverage novelty = exp(-local mean heat / (global mean heat + 1))
```

The final weight is:

```text
sequential weight = sequential base probability
                  × novelty(sequential uses)
                  × coverage novelty(source section)

branch weight     = branch base probability / number of branches
                  × novelty(branch uses)
                  × coverage novelty(destination section)
                  × loop-closure novelty
```

This makes a repeatedly played region lose weight much faster than an
unexplored region, while the adaptive global scale avoids permanently excluding
any part of the track. Transition novelty remains gradual; regional coverage is
the stronger escape pressure.

Backward branches receive one additional factor because they close a loop over
the interval between their destination and source. The first traversal is not
penalised. Once that interval averages more than one visit per beat, the branch
decays exponentially:

```text
loop-closure novelty = exp(-0.35 × max(interval mean heat - 1, 0))
```

Forward branches always receive a loop-closure factor of 1. This prevents a
family of different backward edges within one short section from defeating the
per-edge novelty tracking, while preserving fresh backward jumps.

The planner adds all candidate weights, draws one random number across that
total, and selects the interval containing the draw. Only relative weights
matter; they do not need to add up to one before the draw.

## Worked example

Suppose a beat has two branches and the current general branch probability is
30%. The sequential path therefore starts with 70%, while each branch starts
with 15%.

Assume the song-wide mean heat is 2 and the following local section means
apply:

| Choice | Uses | Local mean | Resulting weight |
| --- | ---: | ---: | ---: |
| Sequential | 0 | 4 | `0.70 / sqrt(1) × exp(-4/3) = 0.185` |
| Branch A | 3 | 8 | `0.15 / sqrt(4) × exp(-8/3) = 0.005` |
| Branch B | 0 | 1 | `0.15 / sqrt(1) × exp(-1/3) = 0.107` |

After normalising the three weights, the approximate selection probabilities
are 62.2% for sequential playback, 1.7% for Branch A, and 36.1% for Branch B.
Branch B is now far more likely than the overused Branch A, although both
originally had the same base probability.

## How a repeated loop is escaped

Consider the observed loop `144 → 128`, followed by ordinary playback back to
beat 144:

```text
128 → 129 → ... → 143 → [144 → 128]
```

Every use lowers the weight of the `144 → 128` edge and increases the visit
count of beat 128. Other branches encountered inside the loop lose less weight
when they are used less often, so their relative probability rises. At beat 144
the sequential route into beat 144 and then the outro also competes with the
loop edge. Its base weight is deliberately small, which means the song usually
loops many times before playing out, but it can eventually escape.

If playback reaches the physical end of the track, the planner returns to beat
0, resets the general branch chance to 18%, and retains the usage history. The
next traversal therefore continues to prefer less-explored routes instead of
forgetting the loops it already used.

## Reproducibility

The choices are random by default. Passing a seed creates the same walk for the
same audio analysis and program version:

```sh
eternal song.mp3 --seed 42
```

This is useful when investigating a particular transition sequence. Without a
seed, each invocation starts with fresh random state.
