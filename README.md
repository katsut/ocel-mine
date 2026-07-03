# ocel-mine

Fast object-centric process mining analysis for [OCEL 2.0](https://www.ocel-standard.org/)
event logs: per-type trace variants, directly-follows graphs, OC-DFG, and process
metrics — in Rust, on top of the [`ocel`](https://crates.io/crates/ocel) crate.

Deterministic computation only: OCEL in, analysis structures (serde-JSON-ready) out.
No I/O of its own beyond what `ocel` provides, no UI, no configuration state — those
live in [ocel-studio](https://github.com/katsut/ocel-studio).

## Status

Bootstrap. First analysis (per-type trace variants) is under construction.

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
