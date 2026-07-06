# CLAUDE.md — ocel-mine

Deterministic object-centric process-mining analysis: `ocel::Ocel` in,
serde-ready structures out. No I/O, no UI, no state. Concepts in
[ARCHITECTURE.md](ARCHITECTURE.md).

## Build, test, verify

```sh
cargo test
cargo clippy --all-targets -- -D warnings && cargo fmt --check
cargo run --release --example replay -- <log> <type> <inductive|powl|alpha> [noise]
cargo run --release --example noise  -- <log> <type> [seed]   # robustness harness
cargo run --release --example stats  -- <log>                 # case-like decision table
```

Fixtures: `sh ../ocel-rs/scripts/fetch-official-fixtures.sh --large`
(order-management, 21K events) — the log every PM4Py cross-check uses.

## Map

- `src/trace.rs` — THE shared trace builder (u16-interned activities, one
  global `(time, index)` sort); every analysis consumes it
- `src/variants.rs`, `src/dfg.rs` — variants, DFG/OC-DFG
- `src/inductive.rs` — IM with IMf-style noise filter; cuts partition the
  alphabet (replay relies on this)
- `src/powl.rs` — partial-order cuts; pair categorization (direct-both-ways =
  concurrent with frequency dominance, closure-based ordering) is the heart
- `src/heuristics.rs`, `src/alpha.rs` — dependency-graph miner; textbook alpha
- `src/replay.rs`, `src/powl_replay.rs` — exact language membership via
  ownership routing (no token-game approximation)
- `src/precision.rs` — ETC escaping-edges; `prefix_ok` is the subtle part
- `src/noise.rs` — seeded swap/drop/duplicate injection (xorshift, no deps)
- `src/metrics.rs`, `src/stats.rs`, `src/cases.rs` — lead times / rework,
  per-type stats (case-like signals), case summaries

## Invariants and traps

- **Trace order is `(time, event index)`** — deterministic; never sort by
  time alone. Variant keys are the activity sequence only (adding times
  shatters them — happened once).
- New analyses must consume `trace::build`, not re-derive traces.
- Numbers are cross-validated against PM4Py to exact equality where a PM4Py
  counterpart exists (variants, DFG edges, IM trees, token replay, ETC
  precision). A new evaluator needs the same treatment before its numbers
  are trusted.
- Performance is a feature: everything low-milliseconds on the 21K log —
  the studio recomputes on every slider move.
- Adding a pub field to a serialized struct is a breaking change even
  pre-1.0 (0.1.2 → 0.2.0 happened for `TypeStats`); crates.io publish needs
  the owner's GO.
- CI clippy runs latest stable — it may know lints your local toolchain
  doesn't (e.g. `Duration::from_mins` stabilization).

## Conventions

Issue → branch → PR → CI green → squash-merge. Design docs live in the
private ocel-workspace (`docs/mine/`), not here.
