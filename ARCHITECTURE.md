# Architecture

How ocel-mine computes what it computes.

## Semantics: per object type, never flattened

Every analysis is scoped to one object type: a **trace** is one object's
events ordered by `(time, event index in the log)` — a deterministic total
order; equal timestamps resolve by the log's own event order. Cross-type
views (OC-DFG) overlay per-type results instead of flattening the log into a
single case notion, so convergence and divergence never inflate counts: an
event shared by three objects counts once per trace it appears in, and shared
nodes report honest per-type breakdowns.

## One shared trace builder

`trace::build` interns activity names to `u16`, sorts once globally, and hands
each analysis the same `(activity, time)` step slices. Variants, DFG, lead
times, discovery, replay, and precision all consume it, which keeps them
mutually consistent by construction.

## Discovery

- **inductive**: exclusive → sequence → parallel → loop cuts with an
  IMf-style `noise_threshold` (rare directly-follows edges below a fraction
  of the strongest edge are ignored), group repair for parallel cuts, a
  once-per-trace fall-through, and a flower fallback. Cuts partition the
  alphabet — a property the replay layer relies on.
- **heuristics**: dependency measure `(a−b)/(a+b+1)` with L1/L2 loop
  measures and a PM4Py-compatible pre-cleaning pass; reports
  covered/total successions since heuristic nets are not replayable.
- **alpha**: the textbook algorithm, including the strict `a # a`
  independence requirement (self-looping activities join no place).
- **POWL**: like inductive, but with a partial-order cut that also expresses
  "A and B in either order, both before C". Pair categorization is the
  heart: directly-follows in both directions means concurrent (with an
  IMf-style frequency-dominance rule under noise), ordering comes from
  one-way reachability over the full closure, and mutual reachability is
  adjudicated by the causal-edges-only closure. Groups are merge-repaired
  (never rejected), with a sequential fallback when the strict stage
  collapses.

## Replay and precision are exact, not token-game approximations

Because inductive cuts partition the alphabet, each symbol belongs to exactly
one child of every operator. `tree_replay` decides **exact language
membership** by routing symbols to their owners (loops use a run-bounded
reachability pass); `tree_precision` adds a prefix-membership predicate and
computes ETC escaping-edges precision over the log's prefix states. POWL
extends the same routing: a partial-order node accepts a word iff each child
accepts its projection and every ordered pair is fully sequential. Alpha nets
get token-based replay and marking-based precision (no silent transitions).
Both are cross-validated against PM4Py to exact equality on the official
sample log.

## Noise robustness is measured, not assumed

`inject_noise` perturbs one type's traces with a known, seeded rate of
adjacent swaps, drops, or duplicates — the perturbed order is written into
both timestamps and event-vec position, so it takes effect even between
same-second events. The harness (`examples/noise.rs`) discovers on the noisy
log and scores fitness/precision **against the clean one**, turning "which
miner and which threshold" into a measured decision per noise kind.

## Performance

Interned symbols, one global sort, and per-variant (not per-trace) decisions
keep everything in the low-millisecond range on the official 21K-event log —
fast enough to recompute on every slider move in a UI.
