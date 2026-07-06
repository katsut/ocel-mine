//! Inductive miner: per-type process tree discovery (the practical tier).
//!
//! Basic IM (Leemans): recursively detect exclusive / sequence / parallel /
//! loop cuts on the sub-log's directly-follows graph and split the log; the
//! fall-through is the flower model, so the result is always a sound tree.
//!
//! `noise_threshold > 0` adds `IMf`-style frequency filtering: at every
//! recursion step, directly-follows edges below the fraction of their source
//! activity's strongest outgoing edge are ignored, and start/end activities
//! are filtered the same way relative to the strongest one. Cuts run on the
//! filtered graph, splits on the full log, so the tree stays sound. This is
//! the frequency filter of `IMf`, not the complete `IMf` fall-through set.

use std::collections::HashMap;

use ocel::Ocel;
use serde::Serialize;

use crate::trace;

/// A process tree node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ProcessTree {
    /// A leaf activity.
    Activity { label: String },
    /// The silent step.
    Tau,
    /// Children in order.
    Sequence { children: Vec<ProcessTree> },
    /// Exactly one child happens.
    Exclusive { children: Vec<ProcessTree> },
    /// All children happen, interleaved.
    Parallel { children: Vec<ProcessTree> },
    /// `children[0]` is the body; the rest are redo parts.
    Loop { children: Vec<ProcessTree> },
}

/// Sub-log: variant -> multiplicity (activities are interned ids).
pub(crate) type Log = HashMap<Vec<u16>, usize>;

struct Miner<'a> {
    names: &'a [&'a str],
    noise_threshold: f64,
}

/// Directly-follows abstraction of a sub-log.
pub(crate) struct Graph {
    pub(crate) alphabet: Vec<u16>,
    pub(crate) follows: HashMap<(u16, u16), usize>,
    pub(crate) starts: Vec<u16>,
    pub(crate) ends: Vec<u16>,
}

/// Keep the ids whose count reaches `noise` × the strongest count.
// counts are far below 2^53, so the usize → f64 casts are exact
#[allow(clippy::cast_precision_loss)]
fn frequent(counts: &HashMap<u16, usize>, noise: f64) -> Vec<u16> {
    let strongest = counts.values().copied().max().unwrap_or(0);
    let mut kept: Vec<u16> = counts
        .iter()
        .filter(|&(_, &count)| count as f64 >= strongest as f64 * noise)
        .map(|(&a, _)| a)
        .collect();
    kept.sort_unstable();
    kept
}

impl Graph {
    pub(crate) fn build(log: &Log, noise_threshold: f64) -> Graph {
        let mut follows: HashMap<(u16, u16), usize> = HashMap::new();
        let mut alphabet: Vec<u16> = Vec::new();
        let mut start_counts: HashMap<u16, usize> = HashMap::new();
        let mut end_counts: HashMap<u16, usize> = HashMap::new();
        for (sequence, &count) in log {
            let (Some(&first), Some(&last)) = (sequence.first(), sequence.last()) else {
                continue;
            };
            *start_counts.entry(first).or_insert(0) += count;
            *end_counts.entry(last).or_insert(0) += count;
            for &a in sequence {
                if !alphabet.contains(&a) {
                    alphabet.push(a);
                }
            }
            for pair in sequence.windows(2) {
                *follows.entry((pair[0], pair[1])).or_insert(0) += count;
            }
        }
        alphabet.sort_unstable();
        if noise_threshold > 0.0 {
            let mut max_outgoing: HashMap<u16, usize> = HashMap::new();
            for (&(a, _), &count) in &follows {
                let strongest = max_outgoing.entry(a).or_insert(0);
                *strongest = (*strongest).max(count);
            }
            // counts are far below 2^53, so the usize → f64 casts are exact
            #[allow(clippy::cast_precision_loss)]
            follows.retain(|&(a, _), &mut count| {
                count as f64 >= max_outgoing[&a] as f64 * noise_threshold
            });
        }
        Graph {
            alphabet,
            follows,
            starts: frequent(&start_counts, noise_threshold),
            ends: frequent(&end_counts, noise_threshold),
        }
    }

    pub(crate) fn has(&self, a: u16, b: u16) -> bool {
        self.follows.contains_key(&(a, b))
    }

    /// Connected components under `linked`; returns activity -> component id.
    pub(crate) fn components(&self, linked: impl Fn(u16, u16) -> bool) -> HashMap<u16, usize> {
        let mut component: HashMap<u16, usize> = HashMap::new();
        let mut next = 0usize;
        for &seed in &self.alphabet {
            if component.contains_key(&seed) {
                continue;
            }
            let mut stack = vec![seed];
            component.insert(seed, next);
            while let Some(a) = stack.pop() {
                for &b in &self.alphabet {
                    if !component.contains_key(&b) && (linked(a, b) || linked(b, a)) {
                        component.insert(b, next);
                        stack.push(b);
                    }
                }
            }
            next += 1;
        }
        component
    }
}

pub(crate) fn count_groups(assignment: &HashMap<u16, usize>) -> usize {
    let mut seen: Vec<usize> = assignment.values().copied().collect();
    seen.sort_unstable();
    seen.dedup();
    seen.len()
}

impl Miner<'_> {
    fn mine(&self, log: &Log) -> ProcessTree {
        if log.is_empty() {
            return ProcessTree::Tau;
        }
        if log.keys().any(Vec::is_empty) {
            let rest: Log = log
                .iter()
                .filter(|(k, _)| !k.is_empty())
                .map(|(k, &v)| (k.clone(), v))
                .collect();
            if rest.is_empty() {
                return ProcessTree::Tau;
            }
            return ProcessTree::Exclusive {
                children: vec![ProcessTree::Tau, self.mine(&rest)],
            };
        }

        let graph = Graph::build(log, self.noise_threshold);
        if graph.alphabet.len() == 1 {
            let activity = ProcessTree::Activity {
                label: self.names[graph.alphabet[0] as usize].to_owned(),
            };
            if log.keys().all(|k| k.len() == 1) {
                return activity;
            }
            // <a>, <a,a>, ... : loop with a silent redo
            return ProcessTree::Loop {
                children: vec![activity, ProcessTree::Tau],
            };
        }

        if let Some(tree) = self.exclusive_cut(log, &graph) {
            return tree;
        }
        if let Some(tree) = self.sequence_cut(log, &graph) {
            return tree;
        }
        if let Some(tree) = self.parallel_cut(log, &graph) {
            return tree;
        }
        if let Some(tree) = self.loop_cut(log, &graph) {
            return tree;
        }
        if let Some(tree) = self.once_per_trace(log, &graph) {
            return tree;
        }

        // fall-through: the flower model
        let mut children = vec![ProcessTree::Tau];
        children.extend(graph.alphabet.iter().map(|&a| ProcessTree::Activity {
            label: self.names[a as usize].to_owned(),
        }));
        ProcessTree::Loop { children }
    }

    /// Fall-through before the flower: an activity occurring exactly once in
    /// every trace runs concurrently to whatever the rest does. One activity
    /// per pass (the alphabetically first, like `PM4Py`) so the cuts get
    /// another chance on the remainder; nested parallels are flattened.
    fn once_per_trace(&self, log: &Log, graph: &Graph) -> Option<ProcessTree> {
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
        let mut children = vec![ProcessTree::Activity {
            label: self.names[chosen as usize].to_owned(),
        }];
        match self.mine(&rest) {
            ProcessTree::Parallel { children: nested } => children.extend(nested),
            other => children.push(other),
        }
        Some(ProcessTree::Parallel { children })
    }

    fn exclusive_cut(&self, log: &Log, graph: &Graph) -> Option<ProcessTree> {
        let component = graph.components(|a, b| graph.has(a, b));
        let k = count_groups(&component);
        if k < 2 {
            return None;
        }
        let mut parts: Vec<Log> = vec![Log::new(); k];
        for (sequence, &count) in log {
            // majority component wins; under noise filtering a trace can
            // touch several components — its foreign activities are noise
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
        Some(ProcessTree::Exclusive {
            children: parts.iter().map(|p| self.mine(p)).collect(),
        })
    }

    fn sequence_cut(&self, log: &Log, graph: &Graph) -> Option<ProcessTree> {
        // strongly connected: mutual reachability
        let reach = reachability(graph);
        let mutual = |a: u16, b: u16| reach[&(a, b)] && reach[&(b, a)];
        let unordered = |a: u16, b: u16| !reach[&(a, b)] && !reach[&(b, a)];
        // group = same SCC or pairwise unordered
        let component = graph.components(|a, b| mutual(a, b) || unordered(a, b));
        let k = count_groups(&component);
        if k < 2 {
            return None;
        }
        // order groups by reachability
        let mut groups: Vec<usize> = (0..k).collect();
        let representative: HashMap<usize, u16> =
            graph.alphabet.iter().map(|&a| (component[&a], a)).collect();
        groups.sort_by(|&g1, &g2| {
            let (a, b) = (representative[&g1], representative[&g2]);
            if reach[&(a, b)] {
                std::cmp::Ordering::Less
            } else if reach[&(b, a)] {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        });
        let position: HashMap<usize, usize> =
            groups.iter().enumerate().map(|(i, &g)| (g, i)).collect();

        let mut parts: Vec<Log> = vec![Log::new(); k];
        for (sequence, &count) in log {
            let mut segments: Vec<Vec<u16>> = vec![Vec::new(); k];
            for &a in sequence {
                segments[position[&component[&a]]].push(a);
            }
            for (i, segment) in segments.into_iter().enumerate() {
                *parts[i].entry(segment).or_insert(0) += count;
            }
        }
        Some(ProcessTree::Sequence {
            children: parts.iter().map(|p| self.mine(p)).collect(),
        })
    }

    fn parallel_cut(&self, log: &Log, graph: &Graph) -> Option<ProcessTree> {
        // linked when NOT both-directions (cannot be separated into parallel groups)
        let component = graph.components(|a, b| !(graph.has(a, b) && graph.has(b, a)));
        let k = count_groups(&component);
        if k < 2 {
            return None;
        }
        let mut groups: Vec<Vec<u16>> = vec![Vec::new(); k];
        for &a in &graph.alphabet {
            groups[component[&a]].push(a);
        }
        // a group without a start or an end activity cannot stand alone;
        // merge it into a neighbor instead of giving the cut up (all
        // cross-component pairs are mutual, so the partition stays valid)
        groups.sort_by_key(Vec::len);
        let mut i = 0;
        while i < groups.len() && groups.len() > 1 {
            let has_start = groups[i].iter().any(|a| graph.starts.contains(a));
            let has_end = groups[i].iter().any(|a| graph.ends.contains(a));
            if has_start && has_end {
                i += 1;
                continue;
            }
            let group = groups.remove(i);
            let target = i.saturating_sub(1);
            groups[target].extend(group);
        }
        if groups.len() < 2 {
            return None;
        }

        let index: HashMap<u16, usize> = groups
            .iter()
            .enumerate()
            .flat_map(|(i, group)| group.iter().map(move |&a| (a, i)))
            .collect();
        let mut parts: Vec<Log> = vec![Log::new(); groups.len()];
        for (sequence, &count) in log {
            let mut projections: Vec<Vec<u16>> = vec![Vec::new(); groups.len()];
            for &a in sequence {
                projections[index[&a]].push(a);
            }
            for (i, projection) in projections.into_iter().enumerate() {
                *parts[i].entry(projection).or_insert(0) += count;
            }
        }
        Some(ProcessTree::Parallel {
            children: parts.iter().map(|p| self.mine(p)).collect(),
        })
    }

    fn loop_cut(&self, log: &Log, graph: &Graph) -> Option<ProcessTree> {
        // connectivity without the potential loop-back edges: everything
        // leaving an end activity or entering a start activity is cut away
        let component = graph.components(|a, b| {
            graph.has(a, b) && !graph.ends.contains(&a) && !graph.starts.contains(&b)
        });
        // the body is every component holding a start or end
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
        // boundary condition: body -> redo only from ends, redo -> body only to starts
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
            // a trace must end in the body; if it ended in a redo part, record
            // an implicit empty body completion
            if current_part != 0 {
                *parts[0].entry(Vec::new()).or_insert(0) += count;
            }
        }
        Some(ProcessTree::Loop {
            children: parts.iter().map(|p| self.mine(p)).collect(),
        })
    }
}

/// Pairwise reachability over the directly-follows relation.
pub(crate) fn reachability(graph: &Graph) -> HashMap<(u16, u16), bool> {
    let n = graph.alphabet.len();
    let index: HashMap<u16, usize> = graph
        .alphabet
        .iter()
        .enumerate()
        .map(|(i, &a)| (a, i))
        .collect();
    let mut reach = vec![vec![false; n]; n];
    for &(a, b) in graph.follows.keys() {
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
    let mut out = HashMap::new();
    for (&a, &i) in &index {
        for (&b, &j) in &index {
            out.insert((a, b), reach[i][j]);
        }
    }
    out
}

/// Discover a process tree for `object_type` with the inductive miner.
///
/// `noise_threshold` of `0.0` is the exact basic miner; higher values (`IMf`
/// territory is around `0.2`) ignore infrequent directly-follows edges so the
/// mainstream structure survives noisy logs. Objects without events are
/// ignored (they are not part of the process).
#[must_use]
pub fn inductive(log: &Ocel, object_type: &str, noise_threshold: f64) -> ProcessTree {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::log_from_sequences;

    fn activity(label: &str) -> ProcessTree {
        ProcessTree::Activity {
            label: label.into(),
        }
    }

    #[test]
    fn sequence_of_three() {
        let log = log_from_sequences(&[&["a", "b", "c"], &["a", "b", "c"]]);
        let tree = inductive(&log, "case", 0.0);
        assert_eq!(
            tree,
            ProcessTree::Sequence {
                children: vec![activity("a"), activity("b"), activity("c")]
            }
        );
    }

    #[test]
    fn exclusive_choice() {
        let log = log_from_sequences(&[&["a", "b"], &["c", "d"]]);
        let tree = inductive(&log, "case", 0.0);
        let ProcessTree::Exclusive { children } = tree else {
            panic!("expected exclusive root");
        };
        assert_eq!(children.len(), 2);
    }

    #[test]
    fn parallel_pair() {
        let log = log_from_sequences(&[&["a", "b"], &["b", "a"]]);
        let tree = inductive(&log, "case", 0.0);
        assert_eq!(
            tree,
            ProcessTree::Parallel {
                children: vec![activity("a"), activity("b")]
            }
        );
    }

    #[test]
    fn loop_with_redo() {
        let log = log_from_sequences(&[&["a"], &["a", "b", "a"], &["a", "b", "a", "b", "a"]]);
        let tree = inductive(&log, "case", 0.0);
        assert_eq!(
            tree,
            ProcessTree::Loop {
                children: vec![activity("a"), activity("b")]
            }
        );
    }

    #[test]
    fn optional_tail_becomes_xor_tau() {
        let log = log_from_sequences(&[&["a", "b"], &["a"]]);
        let tree = inductive(&log, "case", 0.0);
        assert_eq!(
            tree,
            ProcessTree::Sequence {
                children: vec![
                    activity("a"),
                    ProcessTree::Exclusive {
                        children: vec![ProcessTree::Tau, activity("b")]
                    }
                ]
            }
        );
    }

    #[test]
    fn once_per_trace_beats_the_flower() {
        // no cut applies, but b happens exactly once per trace
        let log = log_from_sequences(&[&["a", "b", "a"], &["a", "a", "b"]]);
        let tree = inductive(&log, "case", 0.0);
        assert_eq!(
            tree,
            ProcessTree::Parallel {
                children: vec![
                    activity("b"),
                    ProcessTree::Loop {
                        children: vec![activity("a"), ProcessTree::Tau]
                    },
                ]
            }
        );
    }

    #[test]
    fn noise_threshold_ignores_rare_swap() {
        // ten a,b,c and one b,a,c: the swap makes a and b look concurrent
        let mut sequences: Vec<&[&str]> = vec![&["a", "b", "c"]; 10];
        sequences.push(&["b", "a", "c"]);
        let log = log_from_sequences(&sequences);

        let noisy = inductive(&log, "case", 0.0);
        assert_eq!(
            noisy,
            ProcessTree::Sequence {
                children: vec![
                    ProcessTree::Parallel {
                        children: vec![activity("a"), activity("b")]
                    },
                    activity("c"),
                ]
            }
        );
        // with filtering, the rare b->a edge and the rare start b are ignored
        let filtered = inductive(&log, "case", 0.2);
        assert_eq!(
            filtered,
            ProcessTree::Sequence {
                children: vec![activity("a"), activity("b"), activity("c")]
            }
        );
    }

    #[test]
    fn textbook_l1_structure() {
        let log = log_from_sequences(&[
            &["a", "b", "c", "d"],
            &["a", "b", "c", "d"],
            &["a", "b", "c", "d"],
            &["a", "c", "b", "d"],
            &["a", "c", "b", "d"],
            &["a", "e", "d"],
        ]);
        let tree = inductive(&log, "case", 0.0);
        assert_eq!(
            tree,
            ProcessTree::Sequence {
                children: vec![
                    activity("a"),
                    ProcessTree::Exclusive {
                        children: vec![
                            ProcessTree::Parallel {
                                children: vec![activity("b"), activity("c")]
                            },
                            activity("e"),
                        ]
                    },
                    activity("d"),
                ]
            }
        );
    }
}
