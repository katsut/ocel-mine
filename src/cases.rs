//! Per-object case digests.
//!
//! The list a cases view paginates and filters: one row per object with
//! events, carrying its activity sequence and lead time. Variant filtering
//! and pagination are lookups and stay in the consumer.

use chrono::{DateTime, Utc};
use ocel::Ocel;
use serde::Serialize;

use crate::trace;

/// One object's trace digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseSummary {
    pub object_id: String,
    /// Activity sequence in trace order.
    pub activities: Vec<String>,
    /// Steps in the trace.
    pub events: usize,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub lead_secs: i64,
}

/// Digest every object of `object_type` that has events, sorted by start time
/// (ties by object id).
#[must_use]
pub fn cases(log: &Ocel, object_type: &str) -> Vec<CaseSummary> {
    let traces = trace::build(log, object_type);
    let mut out: Vec<CaseSummary> = traces
        .steps
        .iter()
        .enumerate()
        .filter_map(|(slot, steps)| {
            let (&(_, start), &(_, end)) = (steps.first()?, steps.last()?);
            Some(CaseSummary {
                object_id: traces.object_ids[slot].to_owned(),
                activities: steps
                    .iter()
                    .map(|&(a, _)| traces.activity_names[a as usize].to_owned())
                    .collect(),
                events: steps.len(),
                start,
                end,
                lead_secs: (end - start).num_seconds(),
            })
        })
        .collect();
    out.sort_unstable_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then_with(|| a.object_id.cmp(&b.object_id))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::log_from_sequences;

    #[test]
    fn digests_only_objects_with_events() {
        let log = log_from_sequences(&[&["a", "b", "c"], &["a"]]);
        let all = cases(&log, "case");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].object_id, "o0");
        assert_eq!(all[0].activities, ["a", "b", "c"]);
        assert_eq!(all[0].events, 3);
        assert_eq!(all[0].lead_secs, 120);
        assert_eq!(all[1].lead_secs, 0);
    }

    #[test]
    fn empty_type_is_empty() {
        let log = log_from_sequences(&[]);
        assert!(cases(&log, "case").is_empty());
    }
}
