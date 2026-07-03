# ADR 0001: Scope and per-type semantics

- **Date:** 2026-07-03
- **Status:** Accepted

## Context

ocel-mine is the analysis layer of the ocel family. Its boundary rule: **anything
that is a deterministic computation over an OCEL log belongs here**; anything that
touches state, configuration, or the outside world (UI, credentials, child
processes) belongs in [ocel-studio](https://github.com/katsut/ocel-studio).

Object-centric event data punishes naive flattening. When a log is squashed into a
single case notion, events shared by several objects are double-counted
(convergence) and unrelated object lifecycles are interleaved into false orderings
(divergence). Any analysis this library ships must not manufacture those lies.

Performance is a first-class requirement, not an afterthought: the reason to use a
Rust analysis layer over PM4Py's pandas pipelines is speed on large logs. Hot paths
avoid per-item allocation, intern activity names, and prefer one global sort over
per-trace sorts. Claims are backed by timings on public datasets.

## Decisions

### 1. Scope: the light tier, correctness-checked

In: per-type trace variants, per-type DFG (frequency and duration annotations),
OC-DFG (per-type edges overlaid, object-count annotations), and process metrics
(lead time distributions, activity dwell, rework loops). Out (for now): process
model discovery (inductive miner, OC Petri nets) and alignment-based conformance
checking. The API must not block adding them later.

### 2. Per-type semantics

- A **trace** is one object's E2O-linked events ordered by `(time, event index)`;
  an event linked to the same object more than once (multiple qualifiers) counts
  once. The event index tie-break makes results deterministic.
- A **variant** is a trace's activity sequence, reported with its frequency and an
  example object id.
- Variants and DFGs are computed **within one object type only**. Cross-type
  variants are not offered — there is no established semantics for them.
- The **OC-DFG** is the overlay of per-type DFGs with per-type object counts
  annotated on activity nodes. No edge ever spans a flattened, mixed-type case.

### 3. Output is data, rendering is elsewhere

Every result is a plain struct deriving `serde::Serialize`, ready for JSON.
No drawing, no colors, no layout. Consumers (ocel-studio, notebooks, CLIs)
render as they see fit.

### 4. Verification against PM4Py

Correctness is cross-checked against PM4Py's flattening-based results on official
public datasets (PM4Py `ocel20_example`, Zenodo Order Management). Known acceptable
divergence: tie ordering of same-timestamp events may differ between libraries;
such cases are documented in tests rather than papered over.

## Consequences

- Depends only on the `ocel` crate (+ serde). No async, no I/O of its own.
- Benchmarks/timings on the 21K Order Management log accompany each analysis.
- crates.io publication planned once the first analyses stabilize.
