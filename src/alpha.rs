//! Alpha algorithm: per-type Petri net discovery (the educational tier).
//!
//! The classic α-miner (van der Aalst): footprint relations over one object
//! type's traces → maximal (A, B) causal pairs → places. Its textbook limits
//! are surfaced as warnings instead of hidden: length-1 loops (self-loops)
//! cannot be modeled, length-2 loops confuse the causal relation, and there is
//! no noise tolerance. For real logs prefer the inductive miner.

use std::collections::{BTreeSet, HashSet};

use ocel::Ocel;
use serde::Serialize;

use crate::trace;

/// Pair enumeration is exponential in the number of activities; refuse beyond
/// this rather than hang.
const MAX_ACTIVITIES: usize = 20;

/// A place: incoming and outgoing transitions (empty inputs = source place,
/// empty outputs = sink place).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    pub id: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

/// The discovered Petri net of one object type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PetriNet {
    pub object_type: String,
    pub transitions: Vec<String>,
    pub places: Vec<Place>,
    /// Honest limits hit on this log (self-loops, size cutoff, ...).
    pub warnings: Vec<String>,
}

fn extend_independent(
    from: usize,
    n: usize,
    independent: &[Vec<bool>],
    current: &mut Vec<u16>,
    out: &mut Vec<Vec<u16>>,
) {
    for next in from..n {
        if current
            .iter()
            .all(|&a| independent[a as usize][next] && independent[next][a as usize])
        {
            current.push(u16::try_from(next).expect("checked size"));
            out.push(current.clone());
            extend_independent(next + 1, n, independent, current, out);
            current.pop();
        }
    }
}

/// All non-empty subsets of `0..n` that are pairwise `independent`.
fn independent_subsets(n: usize, independent: &[Vec<bool>]) -> Vec<Vec<u16>> {
    let mut out = Vec::new();
    let mut current: Vec<u16> = Vec::new();
    extend_independent(0, n, independent, &mut current, &mut out);
    out
}

/// X of the alpha algorithm: pairs (A, B) with A, B pairwise-independent and
/// A x B fully causal, reduced to the maximal ones (Y).
fn maximal_pairs(
    n: usize,
    causal: &[Vec<bool>],
    independent: &[Vec<bool>],
) -> Vec<(Vec<u16>, Vec<u16>)> {
    let mut pairs: Vec<(Vec<u16>, Vec<u16>)> = Vec::new();
    for a_set in independent_subsets(n, independent) {
        let common: Vec<usize> = (0..n)
            .filter(|&b| a_set.iter().all(|&a| causal[a as usize][b]))
            .collect();
        if common.is_empty() {
            continue;
        }
        let sub_independent: Vec<Vec<bool>> = common
            .iter()
            .map(|&x| common.iter().map(|&y| independent[x][y]).collect())
            .collect();
        for b_local in independent_subsets(common.len(), &sub_independent) {
            let b_set: Vec<u16> = b_local
                .iter()
                .map(|&i| u16::try_from(common[i as usize]).expect("checked size"))
                .collect();
            pairs.push((a_set.clone(), b_set));
        }
    }

    let as_sets: Vec<(HashSet<u16>, HashSet<u16>)> = pairs
        .iter()
        .map(|(a, b)| {
            (
                a.iter().copied().collect::<HashSet<u16>>(),
                b.iter().copied().collect::<HashSet<u16>>(),
            )
        })
        .collect();
    pairs
        .iter()
        .enumerate()
        .filter(|&(i, _)| {
            !as_sets.iter().enumerate().any(|(j, (a2, b2))| {
                j != i
                    && as_sets[i].0.is_subset(a2)
                    && as_sets[i].1.is_subset(b2)
                    && (as_sets[i].0.len() < a2.len() || as_sets[i].1.len() < b2.len())
            })
        })
        .map(|(_, p)| p.clone())
        .collect()
}

/// Discover a Petri net for `object_type` with the alpha algorithm.
#[must_use]
pub fn alpha(log: &Ocel, object_type: &str) -> PetriNet {
    let traces = trace::build(log, object_type);
    let n = traces.activity_names.len();
    let mut warnings: Vec<String> = Vec::new();

    let mut follows = vec![vec![false; n]; n];
    let mut starts: BTreeSet<u16> = BTreeSet::new();
    let mut ends: BTreeSet<u16> = BTreeSet::new();
    for steps in &traces.steps {
        let (Some(&(first, _)), Some(&(last, _))) = (steps.first(), steps.last()) else {
            continue;
        };
        starts.insert(first);
        ends.insert(last);
        for pair in steps.windows(2) {
            follows[pair[0].0 as usize][pair[1].0 as usize] = true;
        }
    }

    let name = |id: u16| traces.activity_names[id as usize].to_owned();
    let transitions: Vec<String> = {
        let mut t: Vec<String> = (0..n)
            .map(|i| traces.activity_names[i].to_owned())
            .collect();
        t.sort_unstable();
        t
    };

    for (i, row) in follows.iter().enumerate() {
        if row[i] {
            warnings.push(format!(
                "self-loop on '{}' cannot be modeled by the alpha algorithm; use the inductive miner",
                traces.activity_names[i]
            ));
        }
    }
    if n > MAX_ACTIVITIES {
        warnings.push(format!(
            "{n} activities exceed the alpha pair-enumeration cutoff ({MAX_ACTIVITIES}); returning transitions only"
        ));
        return PetriNet {
            object_type: object_type.to_owned(),
            transitions,
            places: Vec::new(),
            warnings,
        };
    }

    let mut causal = vec![vec![false; n]; n];
    let mut independent = vec![vec![false; n]; n];
    for a in 0..n {
        for b in 0..n {
            causal[a][b] = follows[a][b] && !follows[b][a];
            independent[a][b] = !follows[a][b] && !follows[b][a];
        }
    }

    let maximal = maximal_pairs(n, &causal, &independent);

    let mut places: Vec<Place> = maximal
        .iter()
        .map(|(a_set, b_set)| Place {
            id: String::new(),
            inputs: a_set.iter().map(|&a| name(a)).collect(),
            outputs: b_set.iter().map(|&b| name(b)).collect(),
        })
        .collect();
    for place in &mut places {
        place.inputs.sort_unstable();
        place.outputs.sort_unstable();
    }
    places.sort_unstable_by(|a, b| (&a.inputs, &a.outputs).cmp(&(&b.inputs, &b.outputs)));
    for (i, place) in places.iter_mut().enumerate() {
        place.id = format!("p{}", i + 1);
    }
    places.insert(
        0,
        Place {
            id: "source".into(),
            inputs: Vec::new(),
            outputs: {
                let mut v: Vec<String> = starts.iter().map(|&a| name(a)).collect();
                v.sort_unstable();
                v
            },
        },
    );
    places.push(Place {
        id: "sink".into(),
        inputs: {
            let mut v: Vec<String> = ends.iter().map(|&a| name(a)).collect();
            v.sort_unstable();
            v
        },
        outputs: Vec::new(),
    });

    PetriNet {
        object_type: object_type.to_owned(),
        transitions,
        places,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::log_from_sequences;

    #[test]
    fn textbook_l1_yields_the_known_net() {
        // van der Aalst L1: [<a,b,c,d>^3, <a,c,b,d>^2, <a,e,d>]
        let log = log_from_sequences(&[
            &["a", "b", "c", "d"],
            &["a", "b", "c", "d"],
            &["a", "b", "c", "d"],
            &["a", "c", "b", "d"],
            &["a", "c", "b", "d"],
            &["a", "e", "d"],
        ]);
        let net = alpha(&log, "case");
        assert!(net.warnings.is_empty());
        assert_eq!(net.transitions, ["a", "b", "c", "d", "e"]);

        let shapes: Vec<(Vec<&str>, Vec<&str>)> = net
            .places
            .iter()
            .map(|p| {
                (
                    p.inputs.iter().map(String::as_str).collect(),
                    p.outputs.iter().map(String::as_str).collect(),
                )
            })
            .collect();
        let expected: Vec<(Vec<&str>, Vec<&str>)> = vec![
            (vec![], vec!["a"]), // source
            (vec!["a"], vec!["b", "e"]),
            (vec!["a"], vec!["c", "e"]),
            (vec!["b", "e"], vec!["d"]),
            (vec!["c", "e"], vec!["d"]),
            (vec!["d"], vec![]), // sink
        ];
        assert_eq!(shapes, expected);
    }

    #[test]
    fn self_loops_produce_warnings() {
        let log = log_from_sequences(&[&["a", "b", "b", "c"]]);
        let net = alpha(&log, "case");
        assert!(net.warnings.iter().any(|w| w.contains("'b'")));
    }

    #[test]
    fn empty_type_yields_empty_net() {
        let log = log_from_sequences(&[]);
        let net = alpha(&log, "case");
        assert!(net.transitions.is_empty());
        // just source and sink, both empty
        assert_eq!(net.places.len(), 2);
    }
}
