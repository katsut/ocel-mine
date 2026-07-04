//! Heuristics miner: per-type dependency graph discovery (the noise-robust tier).
//!
//! Classic dependency measures (Weijters & van der Aalst) over one object
//! type's traces: direct-succession counts become dependency values, and only
//! edges above the tunable thresholds survive. Length-1 and length-2 loops get
//! their dedicated measures, so short loops are kept where the alpha algorithm
//! fails. The output is deliberately graph-shaped — it states what the
//! measures support and nothing more.

use std::collections::{HashMap, HashSet};

use ocel::Ocel;
use serde::Serialize;

use crate::trace;

/// Tunable thresholds of [`heuristics`].
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeuristicsParams {
    /// Minimum dependency value for an edge, in `0..=1`; higher keeps only
    /// relations the log rarely contradicts.
    pub dependency_threshold: f64,
    /// Minimum length-2 loop measure for an `a ⇄ b` pair, in `0..=1`.
    pub loop2_threshold: f64,
    /// Pre-cleaning: drop a succession whose count is below this fraction of
    /// the weaker endpoint's strongest edge, before any measure is computed
    /// (`PM4Py`-compatible; 0.0 disables).
    pub dfg_noise_threshold: f64,
    /// Drop edges observed fewer times than this.
    pub min_edge_frequency: usize,
    /// Drop activities observed fewer times than this (their edges go too).
    pub min_activity_frequency: usize,
}

impl Default for HeuristicsParams {
    fn default() -> Self {
        HeuristicsParams {
            dependency_threshold: 0.5,
            loop2_threshold: 0.5,
            dfg_noise_threshold: 0.05,
            min_edge_frequency: 1,
            min_activity_frequency: 1,
        }
    }
}

/// One activity node of the dependency graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeuristicActivity {
    pub activity: String,
    /// Trace steps with this activity.
    pub frequency: usize,
    /// Traces starting here.
    pub starts: usize,
    /// Traces ending here.
    pub ends: usize,
}

/// A directed dependency edge.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeuristicEdge {
    pub from: String,
    pub to: String,
    /// Direct-succession occurrences.
    pub frequency: usize,
    /// The measure that admitted the edge: plain dependency for `a → b`,
    /// the length-1 measure for self-loops, the length-2 measure for pairs.
    pub dependency: f64,
}

/// The dependency graph of one object type.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeuristicsNet {
    pub object_type: String,
    pub objects: usize,
    pub with_events: usize,
    /// Nodes sorted by descending frequency, ties by activity name.
    pub activities: Vec<HeuristicActivity>,
    /// Edges sorted by descending frequency, ties by (from, to).
    pub edges: Vec<HeuristicEdge>,
    /// Observed direct successions the kept edges explain (the graph is not
    /// replayable, so coverage is its honest fitness stand-in).
    pub covered_successions: usize,
    /// All observed direct successions, before any cleaning.
    pub total_successions: usize,
}

// Succession counts of realistic logs are far below 2^53, so the usize → f64
// casts in the measures are exact.
#[allow(clippy::cast_precision_loss)]
fn dependency_measure(forward: usize, reverse: usize) -> f64 {
    (forward as f64 - reverse as f64) / (forward as f64 + reverse as f64 + 1.0)
}

#[allow(clippy::cast_precision_loss)]
fn loop_measure(occurrences: usize) -> f64 {
    occurrences as f64 / (occurrences as f64 + 1.0)
}

/// Per-trace tallies feeding the measures.
struct Tally {
    /// Trace steps per activity.
    counts: Vec<usize>,
    starts: Vec<usize>,
    ends: Vec<usize>,
    /// Direct-succession counts, pre-cleaned when requested.
    succession: HashMap<(u16, u16), usize>,
    /// `a b a` pattern counts (raw — length-2 loops must see the noise).
    aba: HashMap<(u16, u16), usize>,
    with_events: usize,
}

fn tally(steps_per_trace: &[Vec<(u16, chrono::DateTime<chrono::Utc>)>], n: usize) -> Tally {
    let mut t = Tally {
        counts: vec![0; n],
        starts: vec![0; n],
        ends: vec![0; n],
        succession: HashMap::new(),
        aba: HashMap::new(),
        with_events: 0,
    };
    for steps in steps_per_trace {
        let (Some(&(first, _)), Some(&(last, _))) = (steps.first(), steps.last()) else {
            continue;
        };
        t.with_events += 1;
        t.starts[first as usize] += 1;
        t.ends[last as usize] += 1;
        for &(a, _) in steps {
            t.counts[a as usize] += 1;
        }
        for pair in steps.windows(2) {
            *t.succession.entry((pair[0].0, pair[1].0)).or_insert(0) += 1;
        }
        for window in steps.windows(3) {
            if window[0].0 == window[2].0 && window[0].0 != window[1].0 {
                *t.aba.entry((window[0].0, window[1].0)).or_insert(0) += 1;
            }
        }
    }
    t
}

impl Tally {
    /// Drop successions far below both endpoints' strongest edge — noise must
    /// not distort the measures.
    // counts are far below 2^53, so the usize → f64 casts are exact
    #[allow(clippy::cast_precision_loss)]
    fn pre_clean(&mut self, noise_threshold: f64) {
        let mut max_count = vec![0usize; self.counts.len()];
        for (&(a, b), &frequency) in &self.succession {
            max_count[a as usize] = max_count[a as usize].max(frequency);
            max_count[b as usize] = max_count[b as usize].max(frequency);
        }
        self.succession.retain(|&(a, b), &mut frequency| {
            let floor = max_count[a as usize].min(max_count[b as usize]) as f64 * noise_threshold;
            frequency as f64 >= floor
        });
    }
}

/// Discover the heuristics dependency graph for `object_type`.
///
/// Objects without events are ignored (they are not part of the process).
#[must_use]
pub fn heuristics(log: &Ocel, object_type: &str, params: &HeuristicsParams) -> HeuristicsNet {
    let traces = trace::build(log, object_type);
    let n = traces.activity_names.len();

    let mut t = tally(&traces.steps, n);
    let total_successions: usize = t.succession.values().sum();
    if params.dfg_noise_threshold > 0.0 {
        t.pre_clean(params.dfg_noise_threshold);
    }
    let Tally {
        counts,
        starts,
        ends,
        succession,
        aba,
        with_events,
    } = t;

    let kept = |a: u16| counts[a as usize] >= params.min_activity_frequency;
    let succ = |a: u16, b: u16| succession.get(&(a, b)).copied().unwrap_or(0);

    // plain dependency and length-1 loops
    let mut accepted: HashMap<(u16, u16), f64> = HashMap::new();
    for (&(a, b), &frequency) in &succession {
        if !kept(a) || !kept(b) || frequency < params.min_edge_frequency {
            continue;
        }
        let dependency = if a == b {
            loop_measure(frequency)
        } else {
            dependency_measure(frequency, succ(b, a))
        };
        if dependency >= params.dependency_threshold {
            accepted.insert((a, b), dependency);
        }
    }

    // length-2 loops: plain dependency cancels out on a ⇄ b, so admit both
    // directions when the dedicated measure holds and plain dependency saw
    // neither direction
    let pairs: HashSet<(u16, u16)> = aba
        .keys()
        .map(|&(a, b)| if a < b { (a, b) } else { (b, a) })
        .collect();
    let l2 = |a: u16, b: u16| aba.get(&(a, b)).copied().unwrap_or(0);
    for &(a, b) in &pairs {
        if !kept(a) || !kept(b) {
            continue;
        }
        if accepted.contains_key(&(a, b)) || accepted.contains_key(&(b, a)) {
            continue;
        }
        let measure = loop_measure(l2(a, b) + l2(b, a));
        if measure < params.loop2_threshold {
            continue;
        }
        for (x, y) in [(a, b), (b, a)] {
            let frequency = succ(x, y);
            if frequency > 0 && frequency >= params.min_edge_frequency {
                accepted.insert((x, y), measure);
            }
        }
    }

    let name = |a: u16| traces.activity_names[a as usize].to_owned();
    let mut edges: Vec<HeuristicEdge> = accepted
        .into_iter()
        .map(|((a, b), dependency)| HeuristicEdge {
            from: name(a),
            to: name(b),
            frequency: succ(a, b),
            dependency,
        })
        .collect();
    edges.sort_unstable_by(|a, b| {
        b.frequency
            .cmp(&a.frequency)
            .then_with(|| (a.from.as_str(), a.to.as_str()).cmp(&(b.from.as_str(), b.to.as_str())))
    });

    let mut activities: Vec<HeuristicActivity> = (0..n)
        .filter(|&id| counts[id] > 0 && counts[id] >= params.min_activity_frequency)
        .map(|id| HeuristicActivity {
            activity: traces.activity_names[id].to_owned(),
            frequency: counts[id],
            starts: starts[id],
            ends: ends[id],
        })
        .collect();
    activities.sort_unstable_by(|a, b| {
        b.frequency
            .cmp(&a.frequency)
            .then_with(|| a.activity.cmp(&b.activity))
    });

    let covered_successions = edges.iter().map(|e| e.frequency).sum();
    HeuristicsNet {
        object_type: object_type.to_owned(),
        objects: traces.object_ids.len(),
        with_events,
        activities,
        edges,
        covered_successions,
        total_successions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::log_from_sequences;

    fn edge<'a>(net: &'a HeuristicsNet, from: &str, to: &str) -> Option<&'a HeuristicEdge> {
        net.edges.iter().find(|e| e.from == from && e.to == to)
    }

    #[test]
    // the measures are simple f64 ratios computed identically here and there
    #[allow(clippy::float_cmp)]
    fn threshold_drops_contradicted_edge() {
        let log = log_from_sequences(&[
            &["a", "b", "c"],
            &["a", "b", "c"],
            &["a", "b", "c"],
            &["a", "b", "c"],
            &["a", "c"],
        ]);
        let params = HeuristicsParams {
            dependency_threshold: 0.6,
            ..HeuristicsParams::default()
        };
        let net = heuristics(&log, "case", &params);

        let ab = edge(&net, "a", "b").expect("a -> b kept");
        assert_eq!(ab.frequency, 4);
        assert_eq!(ab.dependency, 4.0 / 5.0);
        assert!(edge(&net, "b", "c").is_some());
        // a -> c has dependency 1/2 < 0.6
        assert!(edge(&net, "a", "c").is_none());
        assert_eq!(net.edges.len(), 2);
        // coverage counts against the raw successions: ab 4 + bc 4 of 9
        assert_eq!((net.covered_successions, net.total_successions), (8, 9));

        let a = net.activities.iter().find(|x| x.activity == "a").unwrap();
        assert_eq!((a.frequency, a.starts, a.ends), (5, 5, 0));
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn length_one_loop_uses_own_measure() {
        let log = log_from_sequences(&[&["a", "b", "b", "b", "c"]]);
        let net = heuristics(&log, "case", &HeuristicsParams::default());
        let bb = edge(&net, "b", "b").expect("self loop kept");
        assert_eq!(bb.frequency, 2);
        assert_eq!(bb.dependency, 2.0 / 3.0);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn length_two_loop_admits_both_directions() {
        let log = log_from_sequences(&[&["a", "b", "a", "b", "a"]]);
        let net = heuristics(&log, "case", &HeuristicsParams::default());
        // plain dependency is 0 in both directions; the l2 measure is
        // (2 + 1) / (2 + 1 + 1)
        let ab = edge(&net, "a", "b").expect("a -> b kept");
        let ba = edge(&net, "b", "a").expect("b -> a kept");
        assert_eq!(ab.dependency, 3.0 / 4.0);
        assert_eq!(ba.dependency, 3.0 / 4.0);
        assert_eq!((ab.frequency, ba.frequency), (2, 2));
    }

    #[test]
    fn pre_cleaning_drops_rare_succession() {
        // a -> c once, against hundred-strong flows on both endpoints
        let mut sequences: Vec<&[&str]> = vec![&["a", "b", "c"]; 100];
        sequences.push(&["a", "c"]);
        let log = log_from_sequences(&sequences);

        // dependency alone would admit it (1/2 >= 0.5) ...
        let raw = HeuristicsParams {
            dfg_noise_threshold: 0.0,
            ..HeuristicsParams::default()
        };
        assert!(edge(&heuristics(&log, "case", &raw), "a", "c").is_some());
        // ... but pre-cleaning removes the succession before any measure
        let net = heuristics(&log, "case", &HeuristicsParams::default());
        assert!(edge(&net, "a", "c").is_none());
        assert_eq!(net.edges.len(), 2);
    }

    #[test]
    fn min_activity_frequency_drops_rare_activity() {
        let log = log_from_sequences(&[
            &["a", "b"],
            &["a", "b"],
            &["a", "b"],
            &["a", "b"],
            &["a", "b"],
            &["a", "x", "b"],
        ]);
        let params = HeuristicsParams {
            min_activity_frequency: 2,
            ..HeuristicsParams::default()
        };
        let net = heuristics(&log, "case", &params);
        assert!(net.activities.iter().all(|a| a.activity != "x"));
        assert!(net.edges.iter().all(|e| e.from != "x" && e.to != "x"));
        assert_eq!(edge(&net, "a", "b").unwrap().frequency, 5);
    }

    #[test]
    fn empty_type_yields_empty_net() {
        let log = log_from_sequences(&[&["a"]]);
        let net = heuristics(&log, "missing", &HeuristicsParams::default());
        assert_eq!(net.objects, 0);
        assert!(net.activities.is_empty());
        assert!(net.edges.is_empty());
    }
}
