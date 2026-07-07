//! POWL miner: discovery with partial-order cuts (Kourani et al.).
//!
//! The inductive miner's sequence and parallel cuts only express
//! series-parallel structure. The partial-order cut generalizes both: the
//! alphabet is partitioned into groups that are pairwise either strictly
//! ordered (one completes before the other starts) or fully concurrent, with
//! transitivity enforced — so non-series-parallel orders (an N-shaped
//! a≺c, b≺c, b≺d with a ∥ d) become one node instead of an overly general
//! structure. Exclusive and loop cuts, noise filtering, and the
//! fall-throughs mirror [`crate::inductive`].

use std::collections::HashMap;

use ocel::Ocel;
use serde::{Deserialize, Serialize};

use crate::inductive::{count_groups, Graph, Log};
use crate::trace;

/// A POWL model node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Powl {
    /// A leaf activity.
    Activity { label: String },
    /// The silent step.
    Tau,
    /// Exactly one child happens.
    Exclusive { children: Vec<Powl> },
    /// `children[0]` is the body; the rest are redo parts.
    Loop { children: Vec<Powl> },
    /// Every child happens exactly once. `order` lists `(i, j)` pairs
    /// (transitively reduced): child `i` completes before child `j` starts;
    /// unordered children interleave freely. An empty order is full
    /// concurrency; a total order is a sequence.
    PartialOrder {
        children: Vec<Powl>,
        order: Vec<(usize, usize)>,
    },
}

struct Miner<'a> {
    names: &'a [&'a str],
    noise_threshold: f64,
}

/// Pairwise category, alpha-style: direct edges in both directions mean
/// true concurrency; ordering comes from the closure of *causal* edges
/// (one-directional only), so it cannot leak through concurrent pairs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cat {
    /// `a` completes before `b` starts, never the other way.
    Lt,
    /// `b` before `a`.
    Gt,
    /// Direct edges both ways: interleaving.
    Conc,
    /// Unrelated (alternatives) or causally cyclic (loops): must share a
    /// group — the recursion or the loop cut sorts them out.
    None,
}

/// Pairwise categories for the whole alphabet. Under a noise threshold a
/// bidirectional pair whose weaker direction is below `noise` × the stronger
/// one counts as ordered, not concurrent — the `IMf` frequency idea applied at
/// the pair level, which is what lets the partial-order cut keep structure
/// on noisy logs.
// counts are far below 2^53, so the usize → f64 casts are exact
#[allow(clippy::cast_precision_loss)]
fn categories(graph: &Graph, noise: f64) -> HashMap<(u16, u16), Cat> {
    let count = |a: u16, b: u16| graph.follows.get(&(a, b)).copied().unwrap_or(0);
    let dominant = |a: u16, b: u16| {
        // true when a→b clearly dominates b→a
        let (forward, backward) = (count(a, b), count(b, a));
        forward > 0 && backward > 0 && (backward as f64) < (forward as f64) * noise
    };
    let n = graph.alphabet.len();
    let index: HashMap<u16, usize> = graph
        .alphabet
        .iter()
        .enumerate()
        .map(|(i, &a)| (a, i))
        .collect();
    let closure = |edges: &[(u16, u16)]| {
        let mut reach = vec![vec![false; n]; n];
        for &(a, b) in edges {
            reach[index[&a]][index[&b]] = true;
        }
        for k in 0..n {
            let via = reach[k].clone();
            for row in &mut reach {
                if row[k] {
                    for (j, &reachable) in via.iter().enumerate() {
                        if reachable {
                            row[j] = true;
                        }
                    }
                }
            }
        }
        reach
    };
    // full reachability orders whatever is one-way at the global level;
    // causal reachability (edges that are one-directional, or dominant
    // under noise) breaks the ties the full closure leaks through
    // concurrent pairs
    let all_edges: Vec<(u16, u16)> = graph.follows.keys().copied().collect();
    let causal_edges: Vec<(u16, u16)> = graph
        .follows
        .keys()
        .filter(|&&(a, b)| !graph.has(b, a) || dominant(a, b))
        .copied()
        .collect();
    let full = closure(&all_edges);
    let causal = closure(&causal_edges);

    let mut out = HashMap::new();
    for (&a, &i) in &index {
        for (&b, &j) in &index {
            if a == b {
                continue;
            }
            let concurrent =
                graph.has(a, b) && graph.has(b, a) && !dominant(a, b) && !dominant(b, a);
            let cat = if concurrent {
                Cat::Conc
            } else {
                match (full[i][j], full[j][i]) {
                    (true, false) => Cat::Lt,
                    (false, true) => Cat::Gt,
                    (false, false) => Cat::None,
                    (true, true) => match (causal[i][j], causal[j][i]) {
                        (true, false) => Cat::Lt,
                        (false, true) => Cat::Gt,
                        _ => Cat::None,
                    },
                }
            };
            out.insert((a, b), cat);
        }
    }
    out
}

/// The partial-order grouping: start from singletons and merge until every
/// group pair is uniformly Lt/Gt/Conc, Lt is transitive, and every
/// ≺-minimal group holds a start activity (≺-maximal, an end). Groups that
/// violate the start/end guard are merged into a neighbor, IM-style, instead
/// of abandoning the cut.
fn po_groups(graph: &Graph, cats: &HashMap<(u16, u16), Cat>) -> Vec<Vec<u16>> {
    let mut groups: Vec<Vec<u16>> = graph.alphabet.iter().map(|&a| vec![a]).collect();
    loop {
        let mut merge: Option<(usize, usize)> = None;

        // uniformity: all cross pairs of two groups must agree
        'uniform: for i in 0..groups.len() {
            for j in i + 1..groups.len() {
                let mut seen: Option<Cat> = None;
                for &a in &groups[i] {
                    for &b in &groups[j] {
                        let cat = cats[&(a, b)];
                        if cat == Cat::None || seen.is_some_and(|s| s != cat) {
                            merge = Some((i, j));
                            break 'uniform;
                        }
                        seen = Some(cat);
                    }
                }
            }
        }

        let cat_of = |groups: &[Vec<u16>], i: usize, j: usize| cats[&(groups[i][0], groups[j][0])];

        // transitivity of Lt: i<j and j<k require i<k
        if merge.is_none() {
            'transitive: for i in 0..groups.len() {
                for j in 0..groups.len() {
                    if i == j || cat_of(&groups, i, j) != Cat::Lt {
                        continue;
                    }
                    for k in 0..groups.len() {
                        if k == i || k == j || cat_of(&groups, j, k) != Cat::Lt {
                            continue;
                        }
                        if cat_of(&groups, i, k) != Cat::Lt {
                            merge = Some((i.min(k), i.max(k)));
                            break 'transitive;
                        }
                    }
                }
            }
        }

        // start/end guard with repair: a ≺-minimal group without a start
        // (or ≺-maximal without an end) cannot stand alone — merge it into
        // its nearest ordered neighbor (or any other group)
        if merge.is_none() && groups.len() > 1 {
            'guard: for i in 0..groups.len() {
                let minimal = (0..groups.len()).all(|j| j == i || cat_of(&groups, j, i) != Cat::Lt);
                let maximal = (0..groups.len()).all(|j| j == i || cat_of(&groups, i, j) != Cat::Lt);
                let lacks_start = minimal && !groups[i].iter().any(|a| graph.starts.contains(a));
                let lacks_end = maximal && !groups[i].iter().any(|a| graph.ends.contains(a));
                if lacks_start || lacks_end {
                    let partner = (0..groups.len())
                        .filter(|&j| j != i)
                        .min_by_key(|&j| match cat_of(&groups, i, j) {
                            Cat::Lt | Cat::Gt => 0,
                            _ => 1,
                        })
                        .expect("len > 1");
                    merge = Some((i.min(partner), i.max(partner)));
                    break 'guard;
                }
            }
        }

        match merge {
            Some((i, j)) => {
                let absorbed = groups.remove(j);
                groups[i].extend(absorbed);
            }
            None => return groups,
        }
    }
}

impl Miner<'_> {
    fn mine(&self, log: &Log) -> Powl {
        if log.is_empty() {
            return Powl::Tau;
        }
        if log.keys().any(Vec::is_empty) {
            let rest: Log = log
                .iter()
                .filter(|(k, _)| !k.is_empty())
                .map(|(k, &v)| (k.clone(), v))
                .collect();
            if rest.is_empty() {
                return Powl::Tau;
            }
            return Powl::Exclusive {
                children: vec![Powl::Tau, self.mine(&rest)],
            };
        }

        let graph = Graph::build(log, self.noise_threshold);
        if graph.alphabet.len() == 1 {
            let activity = Powl::Activity {
                label: self.names[graph.alphabet[0] as usize].to_owned(),
            };
            if log.keys().all(|k| k.len() == 1) {
                return activity;
            }
            return Powl::Loop {
                children: vec![activity, Powl::Tau],
            };
        }

        if let Some(model) = self.exclusive_cut(log, &graph) {
            return model;
        }
        if let Some(model) = self.po_cut(log, &graph) {
            return model;
        }
        if let Some(model) = self.loop_cut(log, &graph) {
            return model;
        }
        if let Some(model) = self.once_per_trace(log, &graph) {
            return model;
        }

        let mut children = vec![Powl::Tau];
        children.extend(graph.alphabet.iter().map(|&a| Powl::Activity {
            label: self.names[a as usize].to_owned(),
        }));
        Powl::Loop { children }
    }

    fn exclusive_cut(&self, log: &Log, graph: &Graph) -> Option<Powl> {
        let component = graph.components(|a, b| graph.has(a, b));
        let k = count_groups(&component);
        if k < 2 {
            return None;
        }
        let mut parts: Vec<Log> = vec![Log::new(); k];
        for (sequence, &count) in log {
            // majority component wins; foreign activities are noise
            let mut votes = vec![0usize; k];
            for &a in sequence {
                votes[component[&a]] += 1;
            }
            let mut part = 0;
            for (i, &v) in votes.iter().enumerate() {
                if v > votes[part] {
                    part = i;
                }
            }
            let filtered: Vec<u16> = sequence
                .iter()
                .copied()
                .filter(|a| component[a] == part)
                .collect();
            *parts[part].entry(filtered).or_insert(0) += count;
        }
        Some(Powl::Exclusive {
            children: parts.iter().map(|p| self.mine(p)).collect(),
        })
    }

    /// The partial-order cut, in two stages. The strict stage keeps
    /// concurrent pairs in separate (incomparable) groups; if the merge
    /// fixpoint collapses everything, the sequential stage treats
    /// concurrency like a within-group affair (IM's sequence grouping) so a
    /// noisy hairball can still be ordered around — the recursion re-finds
    /// concurrency inside the groups.
    fn po_cut(&self, log: &Log, graph: &Graph) -> Option<Powl> {
        let mut cats = categories(graph, self.noise_threshold);
        let mut groups = po_groups(graph, &cats);
        if groups.len() < 2 {
            for cat in cats.values_mut() {
                if *cat == Cat::Conc {
                    *cat = Cat::None;
                }
            }
            groups = po_groups(graph, &cats);
        }
        let k = groups.len();
        if k < 2 {
            return None;
        }

        let lt = |i: usize, j: usize| i != j && cats[&(groups[i][0], groups[j][0])] == Cat::Lt;

        // full order relation, then transitive reduction for the model
        let mut order: Vec<(usize, usize)> = Vec::new();
        for i in 0..k {
            for j in 0..k {
                if lt(i, j) {
                    let direct = !(0..k).any(|m| lt(i, m) && lt(m, j));
                    if direct {
                        order.push((i, j));
                    }
                }
            }
        }

        let index: HashMap<u16, usize> = groups
            .iter()
            .enumerate()
            .flat_map(|(i, group)| group.iter().map(move |&a| (a, i)))
            .collect();
        let mut parts: Vec<Log> = vec![Log::new(); k];
        for (sequence, &count) in log {
            let mut projections: Vec<Vec<u16>> = vec![Vec::new(); k];
            for &a in sequence {
                projections[index[&a]].push(a);
            }
            for (i, projection) in projections.into_iter().enumerate() {
                *parts[i].entry(projection).or_insert(0) += count;
            }
        }
        Some(Powl::PartialOrder {
            children: parts.iter().map(|p| self.mine(p)).collect(),
            order,
        })
    }

    fn loop_cut(&self, log: &Log, graph: &Graph) -> Option<Powl> {
        let component = graph.components(|a, b| {
            graph.has(a, b) && !graph.ends.contains(&a) && !graph.starts.contains(&b)
        });
        let body_groups: Vec<usize> = graph
            .starts
            .iter()
            .chain(graph.ends.iter())
            .map(|a| component[a])
            .collect();
        let is_body = |a: u16| body_groups.contains(&component[&a]);
        let redo_groups: Vec<usize> = {
            let mut g: Vec<usize> = graph
                .alphabet
                .iter()
                .map(|&a| component[&a])
                .filter(|g| !body_groups.contains(g))
                .collect();
            g.sort_unstable();
            g.dedup();
            g
        };
        if redo_groups.is_empty() {
            return None;
        }
        for &(a, b) in graph.follows.keys() {
            match (is_body(a), is_body(b)) {
                (true, false) if !graph.ends.contains(&a) => return None,
                (false, true) if !graph.starts.contains(&b) => return None,
                _ => {}
            }
        }

        let redo_index: HashMap<usize, usize> = redo_groups
            .iter()
            .enumerate()
            .map(|(i, &g)| (g, i + 1))
            .collect();
        let mut parts: Vec<Log> = vec![Log::new(); redo_groups.len() + 1];
        for (sequence, &count) in log {
            let mut current: Vec<u16> = Vec::new();
            let mut current_part = 0usize;
            for &a in sequence {
                let part = if is_body(a) {
                    0
                } else {
                    redo_index[&component[&a]]
                };
                if part != current_part {
                    *parts[current_part]
                        .entry(std::mem::take(&mut current))
                        .or_insert(0) += count;
                    current_part = part;
                }
                current.push(a);
            }
            *parts[current_part].entry(current).or_insert(0) += count;
            if current_part != 0 {
                *parts[0].entry(Vec::new()).or_insert(0) += count;
            }
        }
        Some(Powl::Loop {
            children: parts.iter().map(|p| self.mine(p)).collect(),
        })
    }

    /// Fall-through before the flower, mirroring the inductive miner: an
    /// activity occurring exactly once per trace runs concurrently (an
    /// unordered partial-order node) to whatever the rest does.
    fn once_per_trace(&self, log: &Log, graph: &Graph) -> Option<Powl> {
        let mut candidates: Vec<u16> = graph.alphabet.clone();
        for sequence in log.keys() {
            candidates.retain(|&a| sequence.iter().filter(|&&x| x == a).count() == 1);
            if candidates.is_empty() {
                return None;
            }
        }
        let chosen = candidates
            .into_iter()
            .min_by_key(|&a| self.names[a as usize])?;
        let mut rest: Log = Log::new();
        for (sequence, &count) in log {
            let projected: Vec<u16> = sequence.iter().copied().filter(|&a| a != chosen).collect();
            *rest.entry(projected).or_insert(0) += count;
        }
        let mut children = vec![Powl::Activity {
            label: self.names[chosen as usize].to_owned(),
        }];
        match self.mine(&rest) {
            Powl::PartialOrder {
                children: nested,
                order,
            } if order.is_empty() => children.extend(nested),
            other => children.push(other),
        }
        Some(Powl::PartialOrder {
            children,
            order: Vec::new(),
        })
    }
}

/// Discover a POWL model for `object_type`.
///
/// `noise_threshold` works exactly as in [`crate::inductive`]: `0.0` is the
/// exact miner, higher values ignore infrequent directly-follows edges.
#[must_use]
pub fn powl(log: &Ocel, object_type: &str, noise_threshold: f64) -> Powl {
    let traces = trace::build(log, object_type);
    let mut variants: Log = HashMap::new();
    for steps in &traces.steps {
        if steps.is_empty() {
            continue;
        }
        let sequence: Vec<u16> = steps.iter().map(|&(a, _)| a).collect();
        *variants.entry(sequence).or_insert(0) += 1;
    }
    let miner = Miner {
        names: &traces.activity_names,
        noise_threshold,
    };
    miner.mine(&variants)
}
