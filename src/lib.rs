//! Fast object-centric process mining analysis for OCEL 2.0 event logs.
//!
//! Deterministic computation only: an [`ocel::Ocel`] goes in, serde-ready
//! analysis structures come out. Semantics are per object type — a trace is one
//! object's events ordered by time; cross-type views overlay per-type results
//! instead of flattening the log (see `ARCHITECTURE.md`).

pub mod alpha;
pub mod cases;
pub mod cluster;
pub mod dfg;
pub mod heuristics;
pub mod inductive;
pub mod metrics;
pub mod noise;
pub mod powl;
pub mod powl_replay;
pub mod precision;
pub mod replay;
pub mod stats;
mod trace;
pub mod variants;

#[cfg(test)]
mod test_util;

pub use alpha::{alpha, PetriNet, Place};
pub use cases::{cases, CaseSummary};
pub use cluster::{variant_clusters, Cluster, ClusterReport};
pub use dfg::{dfg, oc_dfg, Dfg, DfgEdge, DfgNode, OcActivity, OcDfg, OcDfgEdge, OcTypeCount};
pub use heuristics::{
    heuristics, HeuristicActivity, HeuristicEdge, HeuristicsNet, HeuristicsParams,
};
pub use inductive::{inductive, ProcessTree};
pub use metrics::{lead_times, LeadTimeReport, ReworkMetric, VariantLead};
pub use noise::{inject_noise, NoiseSpec};
pub use powl::{powl, Powl};
pub use powl_replay::{powl_precision, powl_replay};
pub use precision::{net_precision, tree_precision, PrecisionReport};
pub use replay::{net_replay, tree_replay, MisfitVariant, ReplayReport};
pub use stats::{type_stats, TypeStats};
pub use variants::{variants, Variant, VariantsReport};
