//! Lead-time metrics.
//!
//! A trace's lead time is the gap between its first and last event. Medians
//! are never added across edges — paths are *measured*: every variant gets its
//! own distribution, and the happy path (most frequent variant) is compared
//! against the measured rest. Rework counts activities repeating within a
//! trace.

use std::collections::HashMap;

use ocel::Ocel;
use serde::Serialize;

use crate::trace;

/// Lead-time distribution of one variant.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VariantLead {
    pub activities: Vec<String>,
    pub count: usize,
    pub median_secs: f64,
    pub mean_secs: f64,
    pub p90_secs: f64,
}

/// An activity that repeats within traces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReworkMetric {
    pub activity: String,
    /// Traces where the activity occurs more than once.
    pub traces: usize,
    /// Occurrences beyond the first, summed over those traces.
    pub extra_occurrences: usize,
}

/// Lead-time metrics of one object type.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LeadTimeReport {
    pub object_type: String,
    /// Traces measured (objects with at least one event).
    pub measured: usize,
    pub median_secs: f64,
    pub mean_secs: f64,
    pub p90_secs: f64,
    /// Median over traces NOT following the most frequent variant.
    pub rest_median_secs: f64,
    pub rest_count: usize,
    /// Per-variant distributions, sorted by descending count (ties by sequence).
    pub variants: Vec<VariantLead>,
    /// Rework, sorted by descending affected traces (ties by activity).
    pub rework: Vec<ReworkMetric>,
}

// Lead times in seconds fit f64 exactly for realistic logs; intentional casts.
#[allow(clippy::cast_precision_loss)]
fn stats(sorted: &[i64]) -> (f64, f64, f64) {
    let n = sorted.len();
    if n == 0 {
        return (0.0, 0.0, 0.0);
    }
    let median = if n % 2 == 1 {
        sorted[n / 2] as f64
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) as f64 / 2.0
    };
    let mean = sorted.iter().sum::<i64>() as f64 / n as f64;
    // nearest-rank percentile
    let p90 = sorted[(n * 9).div_ceil(10).max(1) - 1] as f64;
    (median, mean, p90)
}

/// Compute lead-time metrics for `object_type`.
#[must_use]
pub fn lead_times(log: &Ocel, object_type: &str) -> LeadTimeReport {
    let traces = trace::build(log, object_type);

    let mut per_variant: HashMap<Vec<u16>, Vec<i64>> = HashMap::new();
    let mut all: Vec<i64> = Vec::new();
    let mut rework_agg: HashMap<u16, (usize, usize)> = HashMap::new();
    let mut occurrences: HashMap<u16, usize> = HashMap::new();
    for steps in &traces.steps {
        let (Some(&(_, first)), Some(&(_, last))) = (steps.first(), steps.last()) else {
            continue;
        };
        let lead = (last - first).num_seconds();
        all.push(lead);
        let sequence: Vec<u16> = steps.iter().map(|&(a, _)| a).collect();
        per_variant.entry(sequence).or_default().push(lead);

        occurrences.clear();
        for &(activity, _) in steps {
            *occurrences.entry(activity).or_insert(0) += 1;
        }
        for (&activity, &count) in &occurrences {
            if count >= 2 {
                let entry = rework_agg.entry(activity).or_insert((0, 0));
                entry.0 += 1;
                entry.1 += count - 1;
            }
        }
    }
    all.sort_unstable();
    let (median_secs, mean_secs, p90_secs) = stats(&all);

    let name = |id: u16| traces.activity_names[id as usize].to_owned();
    let mut keyed: Vec<(Vec<u16>, VariantLead)> = per_variant
        .into_iter()
        .map(|(sequence, mut leads)| {
            leads.sort_unstable();
            let (median, mean, p90) = stats(&leads);
            let lead = VariantLead {
                activities: sequence.iter().map(|&a| name(a)).collect(),
                count: leads.len(),
                median_secs: median,
                mean_secs: mean,
                p90_secs: p90,
            };
            (sequence, lead)
        })
        .collect();
    keyed.sort_unstable_by(|a, b| {
        b.1.count
            .cmp(&a.1.count)
            .then_with(|| a.1.activities.cmp(&b.1.activities))
    });

    let (rest_median_secs, rest_count) = if let Some((top_key, _)) = keyed.first() {
        let top_len = top_key.len();
        let top_key = top_key.clone();
        // rest = every trace not following the top variant; recompute leads
        let mut rest: Vec<i64> = traces
            .steps
            .iter()
            .filter(|steps| {
                !steps.is_empty()
                    && (steps.len() != top_len
                        || steps.iter().zip(&top_key).any(|(&(a, _), &k)| a != k))
            })
            .map(|steps| (steps[steps.len() - 1].1 - steps[0].1).num_seconds())
            .collect();
        rest.sort_unstable();
        let (median, _, _) = stats(&rest);
        (median, rest.len())
    } else {
        (0.0, 0)
    };
    let variants: Vec<VariantLead> = keyed.into_iter().map(|(_, lead)| lead).collect();

    let mut rework: Vec<ReworkMetric> = rework_agg
        .into_iter()
        .map(|(activity, (traces_hit, extra))| ReworkMetric {
            activity: name(activity),
            traces: traces_hit,
            extra_occurrences: extra,
        })
        .collect();
    rework.sort_unstable_by(|a, b| {
        b.traces
            .cmp(&a.traces)
            .then_with(|| a.activity.cmp(&b.activity))
    });

    LeadTimeReport {
        object_type: object_type.to_owned(),
        measured: all.len(),
        median_secs,
        mean_secs,
        p90_secs,
        rest_median_secs,
        rest_count,
        variants,
        rework,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::log_from_sequences;

    // helper spacing: events are 1 minute apart within a sequence
    #[test]
    // whole-minute leads are exactly representable in f64
    #[allow(clippy::float_cmp)]
    fn per_variant_distributions_and_rest() {
        let log = log_from_sequences(&[
            &["a", "b"],           // lead 60s
            &["a", "b"],           // lead 60s
            &["a", "b", "c", "d"], // lead 180s
        ]);
        let report = lead_times(&log, "case");
        assert_eq!(report.measured, 3);
        assert_eq!(report.median_secs, 60.0);
        assert_eq!(report.variants[0].activities, ["a", "b"]);
        assert_eq!(report.variants[0].count, 2);
        assert_eq!(report.variants[0].median_secs, 60.0);
        assert_eq!(report.rest_count, 1);
        assert_eq!(report.rest_median_secs, 180.0);
    }

    #[test]
    fn rework_counts_repeats() {
        let log = log_from_sequences(&[&["a", "b", "b", "b", "c"], &["a", "c"]]);
        let report = lead_times(&log, "case");
        assert_eq!(report.rework.len(), 1);
        assert_eq!(report.rework[0].activity, "b");
        assert_eq!(report.rework[0].traces, 1);
        assert_eq!(report.rework[0].extra_occurrences, 2);
    }

    #[test]
    fn empty_type_is_all_zero() {
        let log = log_from_sequences(&[]);
        let report = lead_times(&log, "case");
        assert_eq!(report.measured, 0);
        assert!(report.variants.is_empty());
        assert!(report.rework.is_empty());
    }
}
