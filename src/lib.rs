//! Fast object-centric process mining analysis for OCEL 2.0 event logs.
//!
//! Deterministic computation only: an [`ocel::Ocel`] goes in, serde-ready
//! analysis structures come out. Semantics are per object type — a trace is one
//! object's events ordered by time; cross-type views overlay per-type results
//! instead of flattening the log (see `docs/adr/0001-scope-and-semantics.md`).

pub mod alpha;
pub mod dfg;
pub mod inductive;
mod trace;
pub mod variants;

#[cfg(test)]
mod test_util;

pub use alpha::{alpha, PetriNet, Place};
pub use dfg::{dfg, oc_dfg, Dfg, DfgEdge, DfgNode, OcActivity, OcDfg, OcDfgEdge, OcTypeCount};
pub use inductive::{inductive, ProcessTree};
pub use variants::{variants, Variant, VariantsReport};
