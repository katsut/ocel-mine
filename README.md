# ocel-mine

Fast object-centric process mining analysis for [OCEL 2.0](https://www.ocel-standard.org/)
event logs: per-type trace variants, directly-follows graphs, OC-DFG, and process
metrics — in Rust, on top of the [`ocel`](https://crates.io/crates/ocel) crate.

Deterministic computation only: OCEL in, analysis structures (serde-JSON-ready) out.
No I/O of its own beyond what `ocel` provides, no UI, no configuration state — those
live in [ocel-studio](https://github.com/katsut/ocel-studio).

## Status

Shipped: per-type trace variants, per-type DFG (frequency / distinct objects /
gap statistics, start-end counts), the OC-DFG overlay, and per-type model
discovery — the **alpha algorithm** (educational; its textbook limits are
returned as warnings) and the **basic inductive miner** (practical; sound by
construction, flower fall-through). Metrics and IMf-style noise handling are next.

Discovery honesty notes: alpha cannot model self-loops and caps at 20
activities. The basic inductive miner matches PM4Py exactly on structured
logs (e.g. the orders type below); on heavily interleaved types (items) it
falls through to a flower where PM4Py's IMf variant finds further concurrent
cuts — a known gap, tracked as follow-up.

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
| read the sqlite log | 60 ms | 420 ms |

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
