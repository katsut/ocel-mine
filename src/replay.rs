//! Replay fitness: how much of the log a discovered model explains.
//!
//! [`tree_replay`] decides **exact language membership** per variant. The
//! inductive miner's cuts partition the alphabet, so the children of every
//! operator have pairwise disjoint alphabets — membership reduces to routing
//! each symbol to the child that owns it (loops need a run-bounded
//! reachability pass). No token-game heuristics, no approximation.
//!
//! [`net_replay`] is token-based replay on an alpha net (which has no silent
//! transitions): a trace fits iff every transition fires without missing
//! tokens and the final marking is exactly the sink places.
//!
//! Honesty note: the basic inductive miner fits 100% at noise 0 by
//! construction, and a flower model replays anything over its alphabet —
//! fitness must be read together with model simplicity.

use std::collections::HashMap;

use ocel::Ocel;
use serde::Serialize;

use crate::alpha::PetriNet;
use crate::inductive::ProcessTree;
use crate::trace;

/// A trace variant the model cannot replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MisfitVariant {
    pub activities: Vec<String>,
    /// Traces with this exact sequence.
    pub count: usize,
    /// One object id exhibiting the variant.
    pub example: String,
}

/// Replay result over one object type's traces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayReport {
    pub object_type: String,
    /// Traces with at least one event.
    pub traces: usize,
    /// Traces the model replays exactly.
    pub fitting: usize,
    pub variants: usize,
    pub fitting_variants: usize,
    /// Non-replayable variants, sorted by descending count.
    pub misfits: Vec<MisfitVariant>,
}

/// (sequence, count, example object id) per distinct variant.
fn collect_variants<'a>(traces: &trace::Traces<'a>) -> Vec<(Vec<u16>, usize, &'a str)> {
    let mut index: HashMap<Vec<u16>, usize> = HashMap::new();
    let mut variants: Vec<(Vec<u16>, usize, &str)> = Vec::new();
    for (slot, steps) in traces.steps.iter().enumerate() {
        if steps.is_empty() {
            continue;
        }
        let sequence: Vec<u16> = steps.iter().map(|&(a, _)| a).collect();
        if let Some(&at) = index.get(&sequence) {
            variants[at].1 += 1;
        } else {
            index.insert(sequence.clone(), variants.len());
            variants.push((sequence, 1, traces.object_ids[slot]));
        }
    }
    variants
}

fn report(
    object_type: &str,
    names: &[&str],
    variants: Vec<(Vec<u16>, usize, &str)>,
    fits: impl Fn(&[u16]) -> bool,
) -> ReplayReport {
    let mut traces = 0usize;
    let mut fitting = 0usize;
    let mut fitting_variants = 0usize;
    let mut misfits: Vec<MisfitVariant> = Vec::new();
    let total_variants = variants.len();
    for (sequence, count, example) in variants {
        traces += count;
        if fits(&sequence) {
            fitting += count;
            fitting_variants += 1;
        } else {
            misfits.push(MisfitVariant {
                activities: sequence
                    .iter()
                    .map(|&a| names[a as usize].to_owned())
                    .collect(),
                count,
                example: example.to_owned(),
            });
        }
    }
    misfits.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.activities.cmp(&b.activities))
    });
    ReplayReport {
        object_type: object_type.to_owned(),
        traces,
        fitting,
        variants: total_variants,
        fitting_variants,
        misfits,
    }
}

/// The compiled tree: interned labels, per-node ownership routing.
struct Compiled {
    kinds: Vec<Kind>,
    nullable: Vec<bool>,
    /// Per node: symbol -> index of the child owning it (empty for leaves).
    owner: Vec<HashMap<u16, usize>>,
}

enum Kind {
    Activity(u16),
    Tau,
    Sequence(Vec<usize>),
    Exclusive(Vec<usize>),
    Parallel(Vec<usize>),
    Loop(Vec<usize>),
}

fn compile(
    tree: &ProcessTree,
    intern: &mut HashMap<String, u16>,
    out: &mut Compiled,
) -> (usize, Vec<u16>) {
    let (kind, nullable, alphabet, owner) = match tree {
        ProcessTree::Activity { label } => {
            let next = u16::try_from(intern.len()).expect("more than u16::MAX activities");
            let id = *intern.entry(label.clone()).or_insert(next);
            (Kind::Activity(id), false, vec![id], HashMap::new())
        }
        ProcessTree::Tau => (Kind::Tau, true, Vec::new(), HashMap::new()),
        ProcessTree::Sequence { children }
        | ProcessTree::Exclusive { children }
        | ProcessTree::Parallel { children }
        | ProcessTree::Loop { children } => {
            let mut ids = Vec::with_capacity(children.len());
            let mut alphabet: Vec<u16> = Vec::new();
            let mut owner: HashMap<u16, usize> = HashMap::new();
            let mut nullables = Vec::with_capacity(children.len());
            for (index, child) in children.iter().enumerate() {
                let (id, child_alphabet) = compile(child, intern, out);
                nullables.push(out.nullable[id]);
                ids.push(id);
                for symbol in child_alphabet {
                    // cuts partition the alphabet, so ownership is unique
                    debug_assert!(!owner.contains_key(&symbol), "overlapping child alphabets");
                    owner.insert(symbol, index);
                    alphabet.push(symbol);
                }
            }
            let (kind, nullable) = match tree {
                ProcessTree::Sequence { .. } => (Kind::Sequence(ids), nullables.iter().all(|&n| n)),
                ProcessTree::Exclusive { .. } => {
                    (Kind::Exclusive(ids), nullables.iter().any(|&n| n))
                }
                ProcessTree::Parallel { .. } => (Kind::Parallel(ids), nullables.iter().all(|&n| n)),
                ProcessTree::Loop { .. } => (Kind::Loop(ids), nullables[0]),
                ProcessTree::Activity { .. } | ProcessTree::Tau => unreachable!(),
            };
            (kind, nullable, alphabet, owner)
        }
    };
    out.kinds.push(kind);
    out.nullable.push(nullable);
    out.owner.push(owner);
    (out.kinds.len() - 1, alphabet)
}

impl Compiled {
    fn accepts(&self, node: usize, w: &[u16]) -> bool {
        match &self.kinds[node] {
            Kind::Activity(id) => w.len() == 1 && w[0] == *id,
            Kind::Tau => w.is_empty(),
            Kind::Exclusive(children) => {
                if w.is_empty() {
                    return children.iter().any(|&c| self.nullable[c]);
                }
                let owner = &self.owner[node];
                let Some(&child) = owner.get(&w[0]) else {
                    return false;
                };
                if w.iter().any(|s| owner.get(s) != Some(&child)) {
                    return false;
                }
                self.accepts(children[child], w)
            }
            Kind::Sequence(children) => {
                let owner = &self.owner[node];
                let mut current = 0usize;
                let mut start = 0usize;
                for (i, s) in w.iter().enumerate() {
                    let Some(&child) = owner.get(s) else {
                        return false;
                    };
                    if child < current {
                        return false;
                    }
                    if child > current {
                        if !self.accepts(children[current], &w[start..i]) {
                            return false;
                        }
                        if (current + 1..child).any(|skip| !self.nullable[children[skip]]) {
                            return false;
                        }
                        current = child;
                        start = i;
                    }
                }
                if !self.accepts(children[current], &w[start..]) {
                    return false;
                }
                !(current + 1..children.len()).any(|skip| !self.nullable[children[skip]])
            }
            Kind::Parallel(children) => {
                // disjoint alphabets: any interleaving is allowed, so a word
                // is in the shuffle iff every child accepts its projection
                let owner = &self.owner[node];
                let mut parts: Vec<Vec<u16>> = vec![Vec::new(); children.len()];
                for &s in w {
                    let Some(&child) = owner.get(&s) else {
                        return false;
                    };
                    parts[child].push(s);
                }
                children
                    .iter()
                    .zip(&parts)
                    .all(|(&c, part)| self.accepts(c, part))
            }
            Kind::Loop(children) => self.accepts_loop(node, children, w),
        }
    }

    /// Loop = body (redo body)*. Reachability over "parsed a prefix ending
    /// after a body instance"; instances only span symbols their part owns,
    /// so the search is bounded by ownership runs.
    fn accepts_loop(&self, node: usize, children: &[usize], w: &[u16]) -> bool {
        let owner = &self.owner[node];
        let body = children[0];
        let n = w.len();
        // furthest end of a body-owned run starting at `from`
        let body_run = |from: usize| {
            let mut m = from;
            while m < n && owner.get(&w[m]) == Some(&0) {
                m += 1;
            }
            m
        };
        let redo_nullable = children[1..].iter().any(|&c| self.nullable[c]);

        let mut reach = vec![false; n + 1];
        let mut queue: Vec<usize> = Vec::new();
        for j in 0..=body_run(0) {
            if self.accepts(body, &w[..j]) {
                reach[j] = true;
                queue.push(j);
            }
        }
        while let Some(j) = queue.pop() {
            if reach[n] {
                break;
            }
            // an empty redo (some redo part accepts ε) allows another body
            if redo_nullable {
                for m in j + 1..=body_run(j) {
                    if !reach[m] && self.accepts(body, &w[j..m]) {
                        reach[m] = true;
                        queue.push(m);
                    }
                }
            }
            if j >= n {
                continue;
            }
            let Some(&child) = owner.get(&w[j]) else {
                continue;
            };
            if child == 0 {
                continue; // body symbols cannot start a redo instance
            }
            // redo instance within this child's ownership run, then a body
            let mut end = j;
            while end < n && owner.get(&w[end]) == Some(&child) {
                end += 1;
            }
            for k in j + 1..=end {
                if !self.accepts(children[child], &w[j..k]) {
                    continue;
                }
                for m in k..=body_run(k) {
                    if !reach[m] && self.accepts(body, &w[k..m]) {
                        reach[m] = true;
                        queue.push(m);
                    }
                }
            }
        }
        reach[n]
    }
}

/// Exact replay of one object type's traces on a process tree from this
/// crate's inductive miner (whose cuts guarantee disjoint child alphabets).
#[must_use]
pub fn tree_replay(log: &Ocel, object_type: &str, tree: &ProcessTree) -> ReplayReport {
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
    let mut compiled = Compiled {
        kinds: Vec::new(),
        nullable: Vec::new(),
        owner: Vec::new(),
    };
    let (root, _) = compile(tree, &mut intern, &mut compiled);
    let variants = collect_variants(&traces);
    report(object_type, &traces.activity_names, variants, |w| {
        compiled.accepts(root, w)
    })
}

/// Token-based replay of one object type's traces on an alpha net (no silent
/// transitions): a trace fits iff no token was ever missing and the final
/// marking is exactly one token on each sink place.
#[must_use]
pub fn net_replay(log: &Ocel, object_type: &str, net: &PetriNet) -> ReplayReport {
    let traces = trace::build(log, object_type);

    // per activity id: places consumed from / produced into
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
    let sinks: Vec<usize> = (0..net.places.len())
        .filter(|&p| net.places[p].outputs.is_empty())
        .collect();

    let variants = collect_variants(&traces);
    let fits = |w: &[u16]| -> bool {
        if net.places.is_empty() {
            return false;
        }
        let mut marking = vec![0usize; net.places.len()];
        for &p in &sources {
            marking[p] = 1;
        }
        for &a in w {
            let Some(t) = activity_to_transition[a as usize] else {
                return false;
            };
            for &p in &consumes[t] {
                if marking[p] == 0 {
                    return false;
                }
                marking[p] -= 1;
            }
            for &p in &produces[t] {
                marking[p] += 1;
            }
        }
        marking
            .iter()
            .enumerate()
            .all(|(p, &tokens)| tokens == usize::from(sinks.contains(&p)))
    };
    report(object_type, &traces.activity_names, variants, fits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::log_from_sequences;
    use crate::{alpha, inductive};

    fn replay_on_own_log(sequences: &[&[&str]]) -> ReplayReport {
        let log = log_from_sequences(sequences);
        let tree = inductive(&log, "case", 0.0);
        tree_replay(&log, "case", &tree)
    }

    #[test]
    fn basic_miner_fits_its_own_log_exactly() {
        let report = replay_on_own_log(&[
            &["a", "b", "c", "d"],
            &["a", "c", "b", "d"],
            &["a", "e", "d"],
            &["a", "b", "c", "d"],
        ]);
        assert_eq!((report.traces, report.fitting), (4, 4));
        assert!(report.misfits.is_empty());
    }

    #[test]
    fn parallel_interleavings_and_loops_fit() {
        let report =
            replay_on_own_log(&[&["a", "b"], &["b", "a"], &["a", "b", "a", "b", "a"], &["a"]]);
        assert_eq!(report.fitting, report.traces);
    }

    #[test]
    fn noise_filtered_variant_is_a_misfit() {
        let mut sequences: Vec<&[&str]> = vec![&["a", "b", "c"]; 10];
        sequences.push(&["b", "a", "c"]);
        let log = log_from_sequences(&sequences);
        let tree = inductive(&log, "case", 0.2); // seq(a, b, c): the swap is noise
        let report = tree_replay(&log, "case", &tree);
        assert_eq!((report.traces, report.fitting), (11, 10));
        assert_eq!(report.variants, 2);
        assert_eq!(report.fitting_variants, 1);
        assert_eq!(report.misfits.len(), 1);
        assert_eq!(report.misfits[0].activities, vec!["b", "a", "c"]);
        assert_eq!(report.misfits[0].count, 1);
        assert!(report.misfits[0].example.starts_with('o'));
    }

    #[test]
    fn foreign_activity_never_fits() {
        let log = log_from_sequences(&[&["a", "b"], &["a", "x", "b"]]);
        let tree = crate::ProcessTree::Sequence {
            children: vec![
                crate::ProcessTree::Activity { label: "a".into() },
                crate::ProcessTree::Activity { label: "b".into() },
            ],
        };
        let report = tree_replay(&log, "case", &tree);
        assert_eq!((report.traces, report.fitting), (2, 1));
        assert_eq!(report.misfits[0].activities, vec!["a", "x", "b"]);
    }

    #[test]
    fn flower_fits_anything_over_its_alphabet() {
        let log = log_from_sequences(&[&["a", "b", "b", "a"], &["b"], &["a", "a", "a"]]);
        let flower = crate::ProcessTree::Loop {
            children: vec![
                crate::ProcessTree::Tau,
                crate::ProcessTree::Activity { label: "a".into() },
                crate::ProcessTree::Activity { label: "b".into() },
            ],
        };
        let report = tree_replay(&log, "case", &flower);
        assert_eq!(report.fitting, report.traces);
    }

    #[test]
    fn alpha_net_replays_structured_log() {
        let sequences: &[&[&str]] = &[
            &["a", "b", "c", "d"],
            &["a", "c", "b", "d"],
            &["a", "e", "d"],
        ];
        let log = log_from_sequences(sequences);
        let net = alpha(&log, "case");
        let report = net_replay(&log, "case", &net);
        assert_eq!(report.fitting, report.traces);
    }

    #[test]
    fn alpha_net_cannot_replay_optional_steps() {
        // b is optional; alpha has no silent transitions, so the b-place
        // chain makes b mandatory and the short trace misfits
        let log = log_from_sequences(&[&["a", "b", "c"], &["a", "c"]]);
        let net = alpha(&log, "case");
        let report = net_replay(&log, "case", &net);
        assert_eq!((report.traces, report.fitting), (2, 1));
        assert_eq!(report.misfits[0].activities, vec!["a", "c"]);
    }

    #[test]
    fn alpha_self_loop_transition_is_disconnected_and_free() {
        // the self-looping b joins no place (textbook: b # b fails), so it
        // fires freely — both traces replay, the warning carries the honesty
        let log = log_from_sequences(&[&["a", "b", "c"], &["a", "b", "b", "c"]]);
        let net = alpha(&log, "case");
        assert!(!net.warnings.is_empty());
        let report = net_replay(&log, "case", &net);
        assert_eq!(report.fitting, report.traces);
    }
}
