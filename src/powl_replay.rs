//! Exact replay and precision for POWL models.
//!
//! Same foundations as the process-tree replay: cuts partition the alphabet,
//! so each symbol routes to exactly one child. A partial-order node accepts
//! a word iff every child accepts its projection (children without events
//! must be nullable) and every ordered pair is fully sequential — child `i`'s
//! last event before child `j`'s first. No token-game approximation.

use std::collections::HashMap;

use ocel::Ocel;

use crate::powl::Powl;
use crate::precision::{score, sequences_of, visit_states, PrecisionReport};
use crate::replay::{collect_variants, report, ReplayReport};
use crate::trace;

struct Compiled {
    kinds: Vec<Kind>,
    nullable: Vec<bool>,
    /// Per node: symbol -> index of the child owning it (empty for leaves).
    owner: Vec<HashMap<u16, usize>>,
}

enum Kind {
    Activity(u16),
    Tau,
    Exclusive(Vec<usize>),
    Loop(Vec<usize>),
    /// Children plus the transitive closure of the order: `before[i][j]`.
    Po(Vec<usize>, Vec<Vec<bool>>),
}

fn compile(
    model: &Powl,
    intern: &mut HashMap<String, u16>,
    out: &mut Compiled,
) -> (usize, Vec<u16>) {
    let (kind, nullable, alphabet, owner) = match model {
        Powl::Activity { label } => {
            let next = u16::try_from(intern.len()).expect("more than u16::MAX activities");
            let id = *intern.entry(label.clone()).or_insert(next);
            (Kind::Activity(id), false, vec![id], HashMap::new())
        }
        Powl::Tau => (Kind::Tau, true, Vec::new(), HashMap::new()),
        Powl::Exclusive { children } | Powl::Loop { children } => {
            let (ids, nullables, alphabet, owner) = compile_children(children, intern, out);
            let (kind, nullable) = if matches!(model, Powl::Exclusive { .. }) {
                (Kind::Exclusive(ids), nullables.iter().any(|&n| n))
            } else {
                let body_nullable = nullables[0];
                (Kind::Loop(ids), body_nullable)
            };
            (kind, nullable, alphabet, owner)
        }
        Powl::PartialOrder { children, order } => {
            let (ids, nullables, alphabet, owner) = compile_children(children, intern, out);
            let n = ids.len();
            let mut before = vec![vec![false; n]; n];
            for &(i, j) in order {
                before[i][j] = true;
            }
            // transitive closure
            for k in 0..n {
                let via = before[k].clone();
                for row in &mut before {
                    if row[k] {
                        for (j, &reachable) in via.iter().enumerate() {
                            if reachable {
                                row[j] = true;
                            }
                        }
                    }
                }
            }
            let nullable = nullables.iter().all(|&n| n);
            (Kind::Po(ids, before), nullable, alphabet, owner)
        }
    };
    out.kinds.push(kind);
    out.nullable.push(nullable);
    out.owner.push(owner);
    (out.kinds.len() - 1, alphabet)
}

type Children = (Vec<usize>, Vec<bool>, Vec<u16>, HashMap<u16, usize>);

fn compile_children(
    children: &[Powl],
    intern: &mut HashMap<String, u16>,
    out: &mut Compiled,
) -> Children {
    let mut ids = Vec::with_capacity(children.len());
    let mut nullables = Vec::with_capacity(children.len());
    let mut alphabet: Vec<u16> = Vec::new();
    let mut owner: HashMap<u16, usize> = HashMap::new();
    for (index, child) in children.iter().enumerate() {
        let (id, child_alphabet) = compile(child, intern, out);
        nullables.push(out.nullable[id]);
        ids.push(id);
        for symbol in child_alphabet {
            debug_assert!(!owner.contains_key(&symbol), "overlapping child alphabets");
            owner.insert(symbol, index);
            alphabet.push(symbol);
        }
    }
    (ids, nullables, alphabet, owner)
}

/// Per-child projection and first/last positions within `w`.
struct Projections {
    parts: Vec<Vec<u16>>,
    first: Vec<Option<usize>>,
    last: Vec<Option<usize>>,
}

impl Compiled {
    fn project(&self, node: usize, children: usize, w: &[u16]) -> Option<Projections> {
        let owner = &self.owner[node];
        let mut parts: Vec<Vec<u16>> = vec![Vec::new(); children];
        let mut first: Vec<Option<usize>> = vec![None; children];
        let mut last: Vec<Option<usize>> = vec![None; children];
        for (position, &s) in w.iter().enumerate() {
            let &child = owner.get(&s)?;
            parts[child].push(s);
            first[child].get_or_insert(position);
            last[child] = Some(position);
        }
        Some(Projections { parts, first, last })
    }

    /// Order check: every closed pair with events on both sides must be
    /// fully sequential.
    fn order_respected(before: &[Vec<bool>], p: &Projections) -> bool {
        for (i, row) in before.iter().enumerate() {
            for (j, &ordered) in row.iter().enumerate() {
                if !ordered {
                    continue;
                }
                if let (Some(last_i), Some(first_j)) = (p.last[i], p.first[j]) {
                    if last_i > first_j {
                        return false;
                    }
                }
            }
        }
        true
    }

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
            Kind::Loop(children) => self.loop_reach(node, children, w)[w.len()],
            Kind::Po(children, before) => {
                let Some(p) = self.project(node, children.len(), w) else {
                    return false;
                };
                if !Self::order_respected(before, &p) {
                    return false;
                }
                children
                    .iter()
                    .zip(&p.parts)
                    .all(|(&c, part)| self.accepts(c, part))
            }
        }
    }

    /// Loop = body (redo body)*; identical construction to the tree replay.
    fn loop_reach(&self, node: usize, children: &[usize], w: &[u16]) -> Vec<bool> {
        let owner = &self.owner[node];
        let body = children[0];
        let n = w.len();
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
                continue;
            }
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
        reach
    }

    /// Is `w` a prefix of some accepted word?
    fn prefix_ok(&self, node: usize, w: &[u16]) -> bool {
        if w.is_empty() {
            return true;
        }
        match &self.kinds[node] {
            Kind::Activity(id) => w.len() == 1 && w[0] == *id,
            Kind::Tau => false,
            Kind::Exclusive(children) => {
                let owner = &self.owner[node];
                let Some(&child) = owner.get(&w[0]) else {
                    return false;
                };
                if w.iter().any(|s| owner.get(s) != Some(&child)) {
                    return false;
                }
                self.prefix_ok(children[child], w)
            }
            Kind::Loop(children) => self.prefix_loop(node, children, w),
            Kind::Po(children, before) => {
                let Some(p) = self.project(node, children.len(), w) else {
                    return false;
                };
                if !Self::order_respected(before, &p) {
                    return false;
                }
                let n = children.len();
                for i in 0..n {
                    let successor_started = (0..n).any(|j| before[i][j] && p.first[j].is_some());
                    if p.first[i].is_none() {
                        // a started successor seals child i: it can never
                        // run any more, so it must be skippable
                        if successor_started && !self.nullable[children[i]] {
                            return false;
                        }
                        continue;
                    }
                    // a child with a started successor must already be complete
                    if successor_started {
                        if !self.accepts(children[i], &p.parts[i]) {
                            return false;
                        }
                    } else if !self.prefix_ok(children[i], &p.parts[i]) {
                        return false;
                    }
                }
                true
            }
        }
    }

    fn prefix_loop(&self, node: usize, children: &[usize], w: &[u16]) -> bool {
        let body = children[0];
        if self.prefix_ok(body, w) {
            return true;
        }
        let owner = &self.owner[node];
        let n = w.len();
        let redo_nullable = children[1..].iter().any(|&c| self.nullable[c]);
        let reach = self.loop_reach(node, children, w);
        if reach[n] {
            return true;
        }
        for j in (0..n).filter(|&j| reach[j]) {
            if redo_nullable && self.prefix_ok(body, &w[j..]) {
                return true;
            }
            let Some(&child) = owner.get(&w[j]) else {
                continue;
            };
            if child == 0 {
                continue;
            }
            if self.prefix_ok(children[child], &w[j..]) {
                return true;
            }
            let mut end = j;
            while end < n && owner.get(&w[end]) == Some(&child) {
                end += 1;
            }
            for k in j + 1..=end {
                if self.accepts(children[child], &w[j..k]) && self.prefix_ok(body, &w[k..]) {
                    return true;
                }
            }
        }
        false
    }
}

fn compile_model(model: &Powl, traces: &trace::Traces<'_>) -> (Compiled, usize, usize, usize) {
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
    let (root, _) = compile(model, &mut intern, &mut compiled);
    let logged = traces.activity_names.len();
    let symbols = intern.len();
    (compiled, root, logged, symbols)
}

/// Exact replay of one object type's traces on a POWL model.
#[must_use]
pub fn powl_replay(log: &Ocel, object_type: &str, model: &Powl) -> ReplayReport {
    let traces = trace::build(log, object_type);
    let (compiled, root, _, _) = compile_model(model, &traces);
    let variants = collect_variants(&traces);
    report(object_type, &traces.activity_names, variants, |w| {
        compiled.accepts(root, w)
    })
}

/// Escaping-edges precision of a POWL model (same measure as
/// [`crate::tree_precision`]).
#[must_use]
pub fn powl_precision(log: &Ocel, object_type: &str, model: &Powl) -> PrecisionReport {
    let traces = trace::build(log, object_type);
    let (compiled, root, logged, symbols) = compile_model(model, &traces);
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
