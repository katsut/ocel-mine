# ocel-mine

Fast object-centric process mining analysis for [OCEL 2.0](https://www.ocel-standard.org/)
event logs: per-type trace variants, directly-follows graphs, OC-DFG, and process
metrics — in Rust, on top of the [`ocel`](https://crates.io/crates/ocel) crate.

Deterministic computation only: OCEL in, analysis structures (serde-JSON-ready) out.
No I/O of its own beyond what `ocel` provides, no UI, no configuration state — those
live in [ocel-studio](https://github.com/katsut/ocel-studio).

## Status

Shipped: per-type trace variants (plus deterministic **variant clustering** —
`variant_clusters` groups variants into behavioral families by cosine
similarity over activity/bigram count vectors, centroid-linkage agglomerative,
no ML), per-type DFG (frequency / distinct objects /
gap statistics, start-end counts), the OC-DFG overlay, per-type model
discovery — the **alpha algorithm** (educational; its textbook limits are
returned as warnings), the **inductive miner** (practical; sound by
construction, once-per-trace fall-through before the flower, tunable
`IMf`-style noise threshold), and the **heuristics miner** (noise-robust;
dependency graph with tunable thresholds, PM4Py-compatible 5% pre-cleaning,
dedicated length-1/length-2 loop measures), and the **POWL miner**
(partial-order cuts that also express "A and B in either order, both before
C"; frequency-dominant ordering under the same noise threshold) — plus the
evaluators: **replay fitness** (`tree_replay` decides exact language
membership per variant — the miner's cuts partition the alphabet, so
membership is ownership routing, not a token-game approximation;
`powl_replay` extends the routing to partial-order nodes; `net_replay`
token-replays alpha nets; the heuristics net reports how many observed direct
successions its kept edges explain), **ETC precision**
(`tree_precision` / `powl_precision` / `net_precision`, escaping-edges,
cross-checked against PM4Py), and a **noise-robustness harness**
(`inject_noise` perturbs one type's traces with seeded swap / drop /
duplicate noise; `examples/noise.rs` discovers on the noisy log and scores
fitness and precision against the clean one, so miner and threshold choices
rest on measured degradation curves instead of guesses). The discovery model
types (`ProcessTree`, `Powl`, `PetriNet`) also implement `Deserialize` and
round-trip through JSON.

Discovery honesty notes: alpha cannot model self-loops (a self-looping
activity joins no place and its transition fires freely — textbook behavior)
and caps at 20 activities. The inductive miner matches PM4Py exactly on
structured logs (e.g. the orders type below, at any noise level); on heavily
interleaved types (items) the trees agree up to how optional stages nest
(ours marks the out-of-stock pair optional per activity, PM4Py per pair —
both sound). The noise threshold implements the `IMf` frequency filter (edges
below the fraction of the source's strongest outgoing edge are ignored at
every recursion step), not the complete `IMf` fall-through set. Read fitness
together with simplicity: the basic miner fits 100% at noise 0 by
construction, and a flower replays anything over its alphabet.

## Quickstart

```rust
let log = ocel::io::read_path("order-management.sqlite")?;

let report = ocel_mine::variants(&log, "orders");
for v in report.variants.iter().take(5) {
    println!("{:>6}  {}", v.count, v.activities.join(" -> "));
}

let graph = ocel_mine::dfg(&log, "orders");           // nodes + edges
let overlay = ocel_mine::oc_dfg(&log, &["orders", "items"]); // per-type edges, honest totals
println!("{} edges", graph.edges.len() + overlay.edges.len());
```

Or from the command line:

```sh
cargo run --release --example variants -- order-management.sqlite orders
cargo run --release --example dfg -- order-management.sqlite orders
cargo run --release --example replay -- order-management.sqlite orders inductive 0.1
cargo run --release --example noise -- order-management.sqlite orders
```

## Performance

Official Zenodo [Order Management](https://zenodo.org/records/18373906) log
(21,008 events), Apple Silicon laptop, single run, warm cache:

| | ocel-mine | PM4Py 2.x (on the flattened type) |
|---|---|---|
| `variants("orders")` — 2,000 traces, 5 variants | **3.7 ms** | 24 ms |
| `variants("items")` — 7,659 traces, 286 variants | **5.4 ms** | 91 ms |
| `dfg("orders")` — 5 edges | **3.6 ms** | 17 ms |
| `dfg("items")` — 56 edges | **6.5 ms** | 34 ms |
| `inductive("orders")` — tree identical to PM4Py | **4.0 ms** | 4 ms |
| `inductive("items")` | 6.7 ms | 29 ms |
| `heuristics("items")` — 21 edges identical to PM4Py | **6.9 ms** | 82 ms |
| `inductive` + `tree_replay("items")` | **11 ms** | 1s+ (convert + token replay) |
| read the sqlite log | 60 ms | 420 ms |

Replay percentages match `pm4py.fitness_token_based_replay` exactly on
orders and items at noise 0.0 and 0.2 (100 / 100 / 100 / 95.40%) and on the
alpha net; validating this uncovered and fixed an alpha bug (independent
sets must require `a # a`, so self-looping activities join no place).

Variant counts, DFG edge frequencies, and start/end counts match PM4Py's
flattening exactly on both types. Note when reproducing the PM4Py variants:
compute them from the flattened DataFrame with an explicit per-case timestamp
sort + groupby — `pm4py.get_variants(df)` applied directly to a flattened OCEL
frame returns scrambled sequences.

## Semantics

Object-centric logs punish naive flattening (divergence/convergence). ocel-mine
computes per object type: a trace is one object's events ordered by time, a variant
is its activity sequence. Cross-type views (OC-DFG) overlay per-type edges with
object-count annotations instead of squashing everything into one log. Results are
cross-checked against PM4Py on public datasets.

## The ocel family

| Layer | Repo | License |
|---|---|---|
| Core model, I/O, validation | [ocel-rs](https://github.com/katsut/ocel-rs) (crates.io: [`ocel`](https://crates.io/crates/ocel)) | MIT |
| ETL engine | [ocel-etl](https://github.com/katsut/ocel-etl) | MIT |
| Backlog connector | [ocel-etl-backlog](https://github.com/katsut/ocel-etl-backlog) | MIT |
| **Analysis (this repo)** | ocel-mine | MIT |
| Studio (UI + data sources) | [ocel-studio](https://github.com/katsut/ocel-studio) | ELv2 |

## License

MIT
