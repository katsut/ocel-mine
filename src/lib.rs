//! Fast object-centric process mining analysis for OCEL 2.0 event logs.
//!
//! Deterministic computation only: an [`ocel::Ocel`] goes in, serde-ready
//! analysis structures come out. Semantics are per object type — a trace is one
//! object's events ordered by time; cross-type views overlay per-type results
//! instead of flattening the log (see `docs/adr/0001-scope-and-semantics.md`).
