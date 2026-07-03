//! Per-type descriptive statistics.
//!
//! `type_stats` gives consumers the numbers to judge which object type is the
//! most "case-like" (a workable median trace length, high events coverage)
//! without re-deriving trace semantics themselves.

use ocel::Ocel;
use serde::Serialize;

use crate::trace;

/// Descriptive statistics of one object type.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeStats {
    pub object_type: String,
    /// Objects of the type in the log.
    pub objects: usize,
    /// Objects with at least one linked event.
    pub with_events: usize,
    /// Median trace length over objects with events (0 when none).
    pub median_trace_len: f64,
}

// Trace lengths are small integers; the i64 → f64 midpoint math is exact.
#[allow(clippy::cast_precision_loss)]
fn median(sorted: &[usize]) -> f64 {
    let n = sorted.len();
    if n == 0 {
        return 0.0;
    }
    if n % 2 == 1 {
        sorted[n / 2] as f64
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) as f64 / 2.0
    }
}

/// Compute [`TypeStats`] for every object type in the log, sorted by
/// descending object count (ties by name).
#[must_use]
pub fn type_stats(log: &Ocel) -> Vec<TypeStats> {
    let mut names: Vec<&str> = log.object_types.iter().map(|t| t.name.as_str()).collect();
    for object in &log.objects {
        if !names.contains(&object.object_type.as_str()) {
            names.push(object.object_type.as_str());
        }
    }

    let mut out: Vec<TypeStats> = names
        .iter()
        .map(|&name| {
            let traces = trace::build(log, name);
            let mut lengths: Vec<usize> = traces
                .steps
                .iter()
                .map(Vec::len)
                .filter(|&len| len > 0)
                .collect();
            lengths.sort_unstable();
            TypeStats {
                object_type: name.to_owned(),
                objects: traces.object_ids.len(),
                with_events: lengths.len(),
                median_trace_len: median(&lengths),
            }
        })
        .collect();
    out.sort_unstable_by(|a, b| {
        b.objects
            .cmp(&a.objects)
            .then_with(|| a.object_type.cmp(&b.object_type))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::log_from_sequences;

    #[test]
    fn medians_and_coverage() {
        // traces of length 2, 3, 4 -> median 3
        let log = log_from_sequences(&[&["a", "b"], &["a", "b", "c"], &["a", "b", "c", "d"]]);
        let stats = type_stats(&log);
        let case = stats.iter().find(|s| s.object_type == "case").unwrap();
        assert_eq!(case.objects, 3);
        assert_eq!(case.with_events, 3);
        assert!((case.median_trace_len - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn empty_log_yields_declared_types_with_zeros() {
        let log = log_from_sequences(&[]);
        let stats = type_stats(&log);
        assert_eq!(stats.len(), 1); // "case" is declared by the helper
        assert_eq!(stats[0].with_events, 0);
        assert!(stats[0].median_trace_len.abs() < f64::EPSILON);
    }
}
