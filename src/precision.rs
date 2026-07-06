//! ETC-style precision: how much behavior the model allows beyond the log.
//!
//! For every prefix state the log visits (deduplicated, weighted by visits):
//! `allowed` = activities the model enables after that prefix, `observed` =
//! activities that actually follow in the log, `escaping` = allowed −
//! observed. Precision = 1 − Σ w·|escaping| / Σ w·|allowed| (Muñoz-Gama &
//! Carmona's escaping-edges measure, the same family as `PM4Py`'s
//! `precision_token_based_replay`).
//!
//! States are taken *before* each event of each trace (the empty prefix
//! included, the end-of-trace state excluded), mirroring `PM4Py`. Traces stop
//! contributing at their first non-replayable event (`truncated_traces`
//! reports how many stopped early) — `PM4Py`'s measure does the same, and the
//! numbers match it exactly on the official log (orders/items × noise 0/0.2,
//! alpha on orders — including the 95.4%-fit items model: 0.352038 both).
//!
//! Read together with fitness: a flower model is 100% fit and very
//! imprecise; a linear model of the top variant is precise and unfit.

use std::collections::HashMap;

use ocel::Ocel;
use serde::Serialize;

use crate::alpha::PetriNet;
use crate::inductive::ProcessTree;
use crate::replay::{compile, Compiled};
use crate::trace;

/// Escaping-edges precision of a model over one object type's traces.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrecisionReport {
    pub object_type: String,
    /// 1 − escaping/allowed (1.0 when the model allows nothing anywhere).
    pub precision: f64,
    /// Σ visits·|allowed| over visited prefix states.
    pub allowed: usize,
    /// Σ visits·|escaping| over visited prefix states.
    pub escaping: usize,
    /// Traces that stopped contributing at a non-replayable event.
    pub truncated_traces: usize,
}

/// One visited prefix state: how often, and what followed in the log.
pub(crate) struct State {
    pub(crate) visits: usize,
    pub(crate) observed: Vec<bool>,
}

/// Walk every trace, recording each valid prefix state (before each event)
/// with its visit count and observed next activities. `valid` decides how far
/// a trace keeps contributing.
pub(crate) fn visit_states(
    sequences: &[(Vec<u16>, usize)],
    activities: usize,
    valid: impl Fn(&[u16]) -> bool,
) -> (HashMap<Vec<u16>, State>, usize) {
    let mut states: HashMap<Vec<u16>, State> = HashMap::new();
    let mut truncated = 0usize;
    for (sequence, count) in sequences {
        let mut fully_replayed = true;
        for i in 0..sequence.len() {
            let prefix = &sequence[..i];
            if !valid(prefix) {
                fully_replayed = false;
                break;
            }
            let state = states.entry(prefix.to_vec()).or_insert_with(|| State {
                visits: 0,
                observed: vec![false; activities],
            });
            state.visits += count;
            state.observed[sequence[i] as usize] = true;
        }
        if !fully_replayed {
            truncated += count;
        }
    }
    (states, truncated)
}

/// `allowed_of` returns the allowed activities in the *logged* alphabet plus
/// the count of allowed model moves outside it (never observed by
/// definition, so always escaping) — enabled transitions the log never
/// exercises still widen the model, exactly as in `PM4Py`'s measure.
pub(crate) fn score(
    object_type: &str,
    states: &HashMap<Vec<u16>, State>,
    truncated_traces: usize,
    allowed_of: impl Fn(&[u16]) -> (Vec<bool>, usize),
) -> PrecisionReport {
    let mut allowed_sum = 0usize;
    let mut escaping_sum = 0usize;
    for (prefix, state) in states {
        let (allowed, unlogged) = allowed_of(prefix);
        let allowed_count = allowed.iter().filter(|&&a| a).count() + unlogged;
        let escaping_count = allowed
            .iter()
            .zip(&state.observed)
            .filter(|&(&a, &o)| a && !o)
            .count()
            + unlogged;
        allowed_sum += state.visits * allowed_count;
        escaping_sum += state.visits * escaping_count;
    }
    let precision = if allowed_sum == 0 {
        1.0
    } else {
        1.0 - ratio(escaping_sum, allowed_sum)
    };
    PrecisionReport {
        object_type: object_type.to_owned(),
        precision,
        allowed: allowed_sum,
        escaping: escaping_sum,
        truncated_traces,
    }
}

// counts fit in f64 exactly for realistic logs; intentional cast
#[allow(clippy::cast_precision_loss)]
fn ratio(num: usize, den: usize) -> f64 {
    num as f64 / den as f64
}

pub(crate) fn sequences_of(traces: &trace::Traces<'_>) -> Vec<(Vec<u16>, usize)> {
    let mut index: HashMap<Vec<u16>, usize> = HashMap::new();
    let mut sequences: Vec<(Vec<u16>, usize)> = Vec::new();
    for steps in &traces.steps {
        if steps.is_empty() {
            continue;
        }
        let sequence: Vec<u16> = steps.iter().map(|&(a, _)| a).collect();
        if let Some(&at) = index.get(&sequence) {
            sequences[at].1 += 1;
        } else {
            index.insert(sequence.clone(), sequences.len());
            sequences.push((sequence, 1));
        }
    }
    sequences
}

/// Escaping-edges precision of a process tree (exact prefix semantics via the
/// same ownership routing as [`crate::tree_replay`]).
#[must_use]
pub fn tree_precision(log: &Ocel, object_type: &str, tree: &ProcessTree) -> PrecisionReport {
    let traces = trace::build(log, object_type);
    let mut intern: HashMap<String, u16> = traces
        .activity_names
        .iter()
        .enumerate()
        .map(|(id, &name)| {
            (
                name.to_owned(),
                u16::try_from(id).expect("checked in build"),
            )
        })
        .collect();
    let mut compiled = Compiled::new();
    let (root, _) = compile(tree, &mut intern, &mut compiled);
    let logged = traces.activity_names.len();
    // compile() interns model-only labels after the logged ones
    let symbols = intern.len();

    let sequences = sequences_of(&traces);
    let (states, truncated) = visit_states(&sequences, logged, |prefix| {
        compiled.prefix_ok(root, prefix)
    });
    score(object_type, &states, truncated, |prefix| {
        let mut allowed = vec![false; logged];
        let mut unlogged = 0usize;
        let mut extended = Vec::with_capacity(prefix.len() + 1);
        extended.extend_from_slice(prefix);
        extended.push(0);
        #[allow(clippy::needless_range_loop)] // id doubles as the symbol value
        for id in 0..symbols {
            *extended.last_mut().expect("just pushed") =
                u16::try_from(id).expect("interned as u16");
            if compiled.prefix_ok(root, &extended) {
                if id < logged {
                    allowed[id] = true;
                } else {
                    unlogged += 1;
                }
            }
        }
        (allowed, unlogged)
    })
}

/// Escaping-edges precision of an alpha net: allowed = transitions enabled at
/// the marking each prefix reaches (the net has no silent transitions).
#[must_use]
pub fn net_precision(log: &Ocel, object_type: &str, net: &PetriNet) -> PrecisionReport {
    let traces = trace::build(log, object_type);
    let activities = traces.activity_names.len();

    let transition_of: HashMap<&str, usize> = net
        .transitions
        .iter()
        .enumerate()
        .map(|(i, name)| (name.as_str(), i))
        .collect();
    let mut consumes: Vec<Vec<usize>> = vec![Vec::new(); net.transitions.len()];
    let mut produces: Vec<Vec<usize>> = vec![Vec::new(); net.transitions.len()];
    for (place_index, place) in net.places.iter().enumerate() {
        for name in &place.outputs {
            if let Some(&t) = transition_of.get(name.as_str()) {
                consumes[t].push(place_index);
            }
        }
        for name in &place.inputs {
            if let Some(&t) = transition_of.get(name.as_str()) {
                produces[t].push(place_index);
            }
        }
    }
    let activity_to_transition: Vec<Option<usize>> = traces
        .activity_names
        .iter()
        .map(|name| transition_of.get(name).copied())
        .collect();
    let sources: Vec<usize> = (0..net.places.len())
        .filter(|&p| net.places[p].inputs.is_empty())
        .collect();

    // marking after replaying `prefix`, or None on a missing token
    let marking_after = |prefix: &[u16]| -> Option<Vec<usize>> {
        if net.places.is_empty() {
            return None;
        }
        let mut marking = vec![0usize; net.places.len()];
        for &p in &sources {
            marking[p] = 1;
        }
        for &a in prefix {
            let t = activity_to_transition[a as usize]?;
            for &p in &consumes[t] {
                if marking[p] == 0 {
                    return None;
                }
                marking[p] -= 1;
            }
            for &p in &produces[t] {
                marking[p] += 1;
            }
        }
        Some(marking)
    };

    // transitions whose label never occurs in the traces
    let logged_transition: Vec<bool> = {
        let mut logged = vec![false; net.transitions.len()];
        for t in activity_to_transition.iter().flatten() {
            logged[*t] = true;
        }
        logged
    };

    let sequences = sequences_of(&traces);
    let (states, truncated) = visit_states(&sequences, activities, |prefix| {
        marking_after(prefix).is_some()
    });
    score(object_type, &states, truncated, |prefix| {
        let mut allowed = vec![false; activities];
        let mut unlogged = 0usize;
        let Some(marking) = marking_after(prefix) else {
            return (allowed, unlogged);
        };
        // no input places (e.g. a disconnected alpha self-loop) = fires
        // freely, same as in net_replay — allowed everywhere
        let enabled = |t: usize| consumes[t].iter().all(|&p| marking[p] > 0);
        for (id, slot) in allowed.iter_mut().enumerate() {
            if let Some(t) = activity_to_transition[id] {
                *slot = enabled(t);
            }
        }
        for (t, &is_logged) in logged_transition.iter().enumerate() {
            if !is_logged && enabled(t) {
                unlogged += 1;
            }
        }
        (allowed, unlogged)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::log_from_sequences;
    use crate::{alpha, inductive, ProcessTree};

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }

    #[test]
    fn a_linear_model_of_a_linear_log_is_perfectly_precise() {
        let log = log_from_sequences(&[&["a", "b", "c"], &["a", "b", "c"]]);
        let tree = inductive(&log, "case", 0.0);
        let report = tree_precision(&log, "case", &tree);
        approx(report.precision, 1.0);
        assert_eq!(report.truncated_traces, 0);
    }

    #[test]
    fn a_flower_is_much_less_precise_than_a_sequence() {
        let sequences: Vec<&[&str]> = vec![&["a", "b", "c"]; 5];
        let log = log_from_sequences(&sequences);
        let sequence = inductive(&log, "case", 0.0);
        let flower = ProcessTree::Loop {
            children: vec![
                ProcessTree::Tau,
                ProcessTree::Activity { label: "a".into() },
                ProcessTree::Activity { label: "b".into() },
                ProcessTree::Activity { label: "c".into() },
            ],
        };
        let strict = tree_precision(&log, "case", &sequence);
        let loose = tree_precision(&log, "case", &flower);
        approx(strict.precision, 1.0);
        // flower: every state allows all 3 activities, the log follows 1
        approx(loose.precision, 1.0 / 3.0);
    }

    #[test]
    fn exclusive_choice_counts_the_unchosen_branch_as_escaping() {
        // model allows a|b after start; the log only ever takes a
        let log = log_from_sequences(&[&["s", "a"], &["s", "a"]]);
        let tree = ProcessTree::Sequence {
            children: vec![
                ProcessTree::Activity { label: "s".into() },
                ProcessTree::Exclusive {
                    children: vec![
                        ProcessTree::Activity { label: "a".into() },
                        ProcessTree::Activity { label: "b".into() },
                    ],
                },
            ],
        };
        let report = tree_precision(&log, "case", &tree);
        // states: ε (allows s, observed s) and [s] (allows a+b, observed a),
        // each visited twice: 2·1 + 2·2
        assert_eq!(report.allowed, 6);
        assert_eq!(report.escaping, 2);
        approx(report.precision, 1.0 - 2.0 / 6.0);
        // NOTE: b never appears in the log, so it is not in the trace
        // alphabet — allowed_of can only see logged activities. Guard that
        // assumption explicitly:
        assert!(!log_has_activity(&log, "b"));
    }

    fn log_has_activity(log: &ocel::Ocel, name: &str) -> bool {
        log.events.iter().any(|e| e.event_type == name)
    }

    #[test]
    fn deviating_trace_stops_contributing_and_is_counted() {
        let log = log_from_sequences(&[&["a", "b"], &["b", "a"]]);
        let tree = ProcessTree::Sequence {
            children: vec![
                ProcessTree::Activity { label: "a".into() },
                ProcessTree::Activity { label: "b".into() },
            ],
        };
        let report = tree_precision(&log, "case", &tree);
        assert_eq!(report.truncated_traces, 1);
        // the fit trace visits ε and [a]; each allows exactly the observed
        approx(report.precision, 1.0);
    }

    #[test]
    fn net_precision_matches_tree_semantics_on_a_structured_log() {
        let sequences: &[&[&str]] = &[
            &["a", "b", "c", "d"],
            &["a", "c", "b", "d"],
            &["a", "e", "d"],
        ];
        let log = log_from_sequences(sequences);
        let net = alpha(&log, "case");
        let report = net_precision(&log, "case", &net);
        assert_eq!(report.truncated_traces, 0);
        assert!(report.precision > 0.5 && report.precision <= 1.0);
    }
}
