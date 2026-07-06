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
    /// Median first→last event span (seconds) over objects with at least two
    /// events; 0 when none.
    pub median_active_span_secs: f64,
    /// Distinct activities appearing in this type's traces. Cross-cutting
    /// participants (actors, reference masters) touch (nearly) every
    /// activity in the log; a case notion selects the coherent subset that
    /// is its lifecycle — compare against the log's activity count to tell
    /// them apart.
    pub activity_types: usize,
}

// Trace lengths and second counts are far below 2^53; the midpoint math is
// exact enough.
#[allow(clippy::cast_precision_loss)]
fn median(sorted: &[i64]) -> f64 {
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
            let mut lengths: Vec<i64> = traces
                .steps
                .iter()
                .map(Vec::len)
                .filter(|&len| len > 0)
                .map(|len| i64::try_from(len).expect("trace length fits i64"))
                .collect();
            lengths.sort_unstable();
            let mut spans: Vec<i64> = traces
                .steps
                .iter()
                .filter(|steps| steps.len() >= 2)
                .map(|steps| {
                    let first = steps[0].1;
                    let last = steps[steps.len() - 1].1;
                    (last - first).num_seconds()
                })
                .collect();
            spans.sort_unstable();
            let mut seen: Vec<bool> = vec![false; traces.activity_names.len()];
            for steps in &traces.steps {
                for &(activity, _) in steps {
                    seen[usize::from(activity)] = true;
                }
            }
            TypeStats {
                object_type: name.to_owned(),
                objects: traces.object_ids.len(),
                with_events: lengths.len(),
                median_trace_len: median(&lengths),
                median_active_span_secs: median(&spans),
                activity_types: seen.iter().filter(|&&s| s).count(),
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
        // traces of length 2, 3, 4 -> median 3; events are 1min apart, so
        // the active spans are 60, 120, 180 seconds -> median 120
        let log = log_from_sequences(&[&["a", "b"], &["a", "b", "c"], &["a", "b", "c", "d"]]);
        let stats = type_stats(&log);
        let case = stats.iter().find(|s| s.object_type == "case").unwrap();
        assert_eq!(case.objects, 3);
        assert_eq!(case.with_events, 3);
        assert!((case.median_trace_len - 3.0).abs() < f64::EPSILON);
        assert!((case.median_active_span_secs - 120.0).abs() < f64::EPSILON);
    }

    #[test]
    fn single_event_objects_do_not_drag_the_span_down() {
        // many one-event objects (span undefined) around one long-lived one
        let log = log_from_sequences(&[&["a"], &["a"], &["a"], &["a", "b", "c", "d", "e"]]);
        let stats = type_stats(&log);
        let case = stats.iter().find(|s| s.object_type == "case").unwrap();
        assert!((case.median_active_span_secs - 240.0).abs() < f64::EPSILON);
    }

    #[test]
    fn activity_alphabet_counts_distinct_activities_across_traces() {
        let log = log_from_sequences(&[&["a", "b"], &["c"], &["a"]]);
        let stats = type_stats(&log);
        let case = stats.iter().find(|s| s.object_type == "case").unwrap();
        assert_eq!(case.activity_types, 3);
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
