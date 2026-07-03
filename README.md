# ocel-mine

Fast object-centric process mining analysis for [OCEL 2.0](https://www.ocel-standard.org/)
event logs: per-type trace variants, directly-follows graphs, OC-DFG, and process
metrics — in Rust, on top of the [`ocel`](https://crates.io/crates/ocel) crate.

Deterministic computation only: OCEL in, analysis structures (serde-JSON-ready) out.
No I/O of its own beyond what `ocel` provides, no UI, no configuration state — those
live in [ocel-studio](https://github.com/katsut/ocel-studio).

## Status

First analysis shipped: per-type trace variants. DFG / OC-DFG / metrics are next.

## Quickstart

```rust
let log = ocel::io::read_path("order-management.sqlite")?;
let report = ocel_mine::variants(&log, "orders");
for v in report.variants.iter().take(5) {
    println!("{:>6}  {}", v.count, v.activities.join(" -> "));
}
```

Or from the command line:

```sh
cargo run --release --example variants -- order-management.sqlite orders
```

## Performance

Official Zenodo [Order Management](https://zenodo.org/records/18373906) log
(21,008 events), Apple Silicon laptop, single run, warm cache:

| | ocel-mine | PM4Py 2.x (flatten + pandas groupby) |
|---|---|---|
| `variants("orders")` — 2,000 traces, 5 variants | **3.7 ms** | 24 ms |
| `variants("items")` — 7,659 traces, 286 variants | **5.4 ms** | 91 ms |
| read the sqlite log | 60 ms | 420 ms |

Variant counts match PM4Py's flattening exactly on both types. Note when
reproducing the PM4Py side: compute variants from the flattened DataFrame with an
explicit per-case timestamp sort + groupby — `pm4py.get_variants(df)` applied
directly to a flattened OCEL frame returns scrambled sequences.

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
