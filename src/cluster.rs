//! Variant clustering: group one type's trace variants into behavioral
//! families.
//!
//! Deterministic agglomerative clustering — no randomness, no ML, no learned
//! weights. Each variant becomes a sparse count vector of its activities and
//! adjacent activity bigrams; similarity is the cosine between those vectors.
//! Clustering starts with one cluster per variant and repeatedly merges the
//! most similar pair, stopping at `max_clusters` or when the best pair falls
//! below [`MIN_SIMILARITY`].
//!
//! Linkage is **centroid**, not true average linkage: a cluster is the sum of
//! its members' feature vectors (cosine ignores scale, so the sum is the
//! centroid), which makes a merge one vector addition plus one similarity row
//! update. The full run computes O(n²) cosines for n variants; the cached
//! similarity matrix is only scanned, never recomputed, between merges.

use std::cmp::Ordering;
use std::collections::HashMap;

use ocel::Ocel;
use serde::Serialize;

use crate::variants;

/// Merging stops when the most similar pair of clusters falls below this
/// cosine similarity, so a small `max_clusters` may honestly return more
/// clusters than asked for instead of gluing unrelated behavior together.
pub const MIN_SIMILARITY: f64 = 0.3;

/// At most this many activities are reported per cluster.
const TOP_ACTIVITIES: usize = 8;

/// One family of trace variants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cluster {
    /// Objects (traces) across all member variants.
    pub traces: usize,
    /// Member variants.
    pub variants: usize,
    /// The highest-count member variant's activity sequence.
    pub representative: Vec<String>,
    /// Most frequent activities across the cluster's traces, capped at 8;
    /// ties break lexicographically.
    pub top_activities: Vec<String>,
}

/// Variant clusters of one object type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterReport {
    pub object_type: String,
    /// Distinct variants that were clustered.
    pub variants: usize,
    /// Clusters sorted by descending trace count, ties by representative.
    pub clusters: Vec<Cluster>,
}

/// A cluster under construction: member variant indices into the sorted
/// variants report plus the summed feature vector (the centroid, up to scale).
struct Node {
    traces: usize,
    members: Vec<usize>,
    vector: Vec<(u64, f64)>,
    norm: f64,
}

/// Sparse dot product over feature-id-sorted vectors.
fn dot(a: &[(u64, f64)], b: &[(u64, f64)]) -> f64 {
    let mut sum = 0.0;
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].0.cmp(&b[j].0) {
            Ordering::Less => i += 1,
            Ordering::Greater => j += 1,
            Ordering::Equal => {
                sum += a[i].1 * b[j].1;
                i += 1;
                j += 1;
            }
        }
    }
    sum
}

fn sum_vectors(a: &[(u64, f64)], b: &[(u64, f64)]) -> Vec<(u64, f64)> {
    let mut out = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].0.cmp(&b[j].0) {
            Ordering::Less => {
                out.push(a[i]);
                i += 1;
            }
            Ordering::Greater => {
                out.push(b[j]);
                j += 1;
            }
            Ordering::Equal => {
                out.push((a[i].0, a[i].1 + b[j].1));
                i += 1;
                j += 1;
            }
        }
    }
    out.extend_from_slice(&a[i..]);
    out.extend_from_slice(&b[j..]);
    out
}

fn norm(v: &[(u64, f64)]) -> f64 {
    v.iter().map(|&(_, x)| x * x).sum::<f64>().sqrt()
}

/// Non-empty variants always contain at least one activity, so norms are
/// never zero.
fn cosine(a: &Node, b: &Node) -> f64 {
    dot(&a.vector, &b.vector) / (a.norm * b.norm)
}

/// Bag of activities plus bag of adjacent bigrams, as a sorted sparse vector.
/// Feature ids: activity `a` is `a`; bigram `(a, b)` is
/// `activity_count + a * activity_count + b`.
// counts are far below 2^53, so the usize → f64 casts are exact
#[allow(clippy::cast_precision_loss)]
fn feature_vector(sequence: &[u64], activity_count: u64) -> Vec<(u64, f64)> {
    let mut counts: HashMap<u64, usize> = HashMap::new();
    for &a in sequence {
        *counts.entry(a).or_insert(0) += 1;
    }
    for pair in sequence.windows(2) {
        *counts
            .entry(activity_count + pair[0] * activity_count + pair[1])
            .or_insert(0) += 1;
    }
    let mut vector: Vec<(u64, f64)> = counts
        .into_iter()
        .map(|(feature, count)| (feature, count as f64))
        .collect();
    vector.sort_unstable_by_key(|&(feature, _)| feature);
    vector
}

/// The most similar live pair, or `None` when fewer than two clusters remain.
/// Ties prefer more combined traces, then the lexicographically smaller slot
/// pair — and a slot is its cluster's representative index in the variants
/// report (count-descending, then lexicographic), so ties resolve toward the
/// heavier, lexicographically earlier representatives.
fn best_pair(live: &[usize], nodes: &[Node], sim: &[f64], n: usize) -> Option<(usize, usize, f64)> {
    let mut best: Option<(f64, usize, usize, usize)> = None;
    for (position, &i) in live.iter().enumerate() {
        for &j in &live[position + 1..] {
            let s = sim[i * n + j];
            let combined = nodes[i].traces + nodes[j].traces;
            let better = match best {
                None => true,
                Some((best_sim, best_combined, best_i, best_j)) => match s.total_cmp(&best_sim) {
                    Ordering::Greater => true,
                    Ordering::Less => false,
                    Ordering::Equal => match combined.cmp(&best_combined) {
                        Ordering::Greater => true,
                        Ordering::Less => false,
                        Ordering::Equal => (i, j) < (best_i, best_j),
                    },
                },
            };
            if better {
                best = Some((s, combined, i, j));
            }
        }
    }
    best.map(|(s, _, i, j)| (i, j, s))
}

/// The report entry for one finished cluster.
fn assemble(node: &Node, all_variants: &[variants::Variant]) -> Cluster {
    // variants are sorted count-descending then lexicographic, so the
    // smallest member index is the highest-count representative
    let representative = node.members.iter().min().expect("non-empty cluster");
    let mut frequency: HashMap<&str, usize> = HashMap::new();
    for &member in &node.members {
        let variant = &all_variants[member];
        for activity in &variant.activities {
            *frequency.entry(activity.as_str()).or_insert(0) += variant.count;
        }
    }
    let mut top: Vec<(&str, usize)> = frequency.into_iter().collect();
    top.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    top.truncate(TOP_ACTIVITIES);
    Cluster {
        traces: node.traces,
        variants: node.members.len(),
        representative: all_variants[*representative].activities.clone(),
        top_activities: top.into_iter().map(|(name, _)| name.to_owned()).collect(),
    }
}

/// Cluster the trace variants of `object_type` into behavioral families.
///
/// Deterministic: variants come from [`variants::variants`] (count-descending,
/// ties lexicographic), similarities are cosines over activity + adjacent
/// bigram count vectors, and every tie-break is total. Merging is centroid
/// linkage (see the module doc) and stops at `max_clusters` **or** when the
/// best pair's similarity drops below [`MIN_SIMILARITY`] — so
/// `max_clusters = 1` may still return several honest clusters rather than
/// force unrelated behavior together. `max_clusters = 0` is treated as 1.
#[must_use]
pub fn variant_clusters(log: &Ocel, object_type: &str, max_clusters: usize) -> ClusterReport {
    let report = variants::variants(log, object_type);
    let n = report.variants.len();
    if n == 0 {
        return ClusterReport {
            object_type: object_type.to_owned(),
            variants: 0,
            clusters: Vec::new(),
        };
    }

    // Intern activity names in report order (deterministic).
    let mut activity_ids: HashMap<&str, u64> = HashMap::new();
    let mut next_id: u64 = 0;
    for variant in &report.variants {
        for activity in &variant.activities {
            activity_ids.entry(activity.as_str()).or_insert_with(|| {
                let id = next_id;
                next_id += 1;
                id
            });
        }
    }

    let mut nodes: Vec<Node> = report
        .variants
        .iter()
        .enumerate()
        .map(|(index, variant)| {
            let sequence: Vec<u64> = variant
                .activities
                .iter()
                .map(|activity| activity_ids[activity.as_str()])
                .collect();
            let vector = feature_vector(&sequence, next_id);
            let vector_norm = norm(&vector);
            Node {
                traces: variant.count,
                members: vec![index],
                vector,
                norm: vector_norm,
            }
        })
        .collect();

    // Cache all pairwise similarities once; merges only refresh one row.
    let mut sim = vec![0.0f64; n * n];
    for i in 0..n {
        for j in (i + 1)..n {
            sim[i * n + j] = cosine(&nodes[i], &nodes[j]);
        }
    }

    let mut live: Vec<usize> = (0..n).collect();
    let target = max_clusters.max(1);
    while live.len() > target {
        let Some((i, j, best_sim)) = best_pair(&live, &nodes, &sim, n) else {
            break;
        };
        if best_sim < MIN_SIMILARITY {
            break;
        }
        // Merge j into i (i < j, so the merged slot stays the smaller
        // representative index).
        let members = std::mem::take(&mut nodes[j].members);
        let vector = std::mem::take(&mut nodes[j].vector);
        nodes[i].traces += nodes[j].traces;
        nodes[i].members.extend(members);
        nodes[i].vector = sum_vectors(&nodes[i].vector, &vector);
        nodes[i].norm = norm(&nodes[i].vector);
        live.retain(|&slot| slot != j);
        for &k in &live {
            if k == i {
                continue;
            }
            let (lo, hi) = if k < i { (k, i) } else { (i, k) };
            sim[lo * n + hi] = cosine(&nodes[i], &nodes[k]);
        }
    }

    let mut clusters: Vec<Cluster> = live
        .iter()
        .map(|&slot| assemble(&nodes[slot], &report.variants))
        .collect();
    clusters.sort_unstable_by(|a, b| {
        b.traces
            .cmp(&a.traces)
            .then_with(|| a.representative.cmp(&b.representative))
    });

    ClusterReport {
        object_type: object_type.to_owned(),
        variants: n,
        clusters,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::log_from_sequences;

    #[test]
    fn two_families_cluster_apart() {
        let log = log_from_sequences(&[
            &["a", "b", "c"],
            &["a", "b", "c"],
            &["a", "b", "c"],
            &["a", "b", "c"],
            &["a", "b", "b", "c"],
            &["x", "y", "z"],
            &["x", "y", "z"],
            &["x", "y", "z"],
            &["x", "z", "y"],
        ]);
        let report = variant_clusters(&log, "case", 2);
        assert_eq!(report.variants, 4);
        assert_eq!(report.clusters.len(), 2);
        assert_eq!(report.clusters[0].traces, 5);
        assert_eq!(report.clusters[0].variants, 2);
        assert_eq!(report.clusters[0].representative, ["a", "b", "c"]);
        // frequencies: b 4+2=6, a 5, c 5 — tie between a and c breaks lex
        assert_eq!(report.clusters[0].top_activities, ["b", "a", "c"]);
        assert_eq!(report.clusters[1].traces, 4);
        assert_eq!(report.clusters[1].variants, 2);
        assert_eq!(report.clusters[1].representative, ["x", "y", "z"]);
    }

    #[test]
    fn two_runs_are_identical() {
        let log = log_from_sequences(&[
            &["a", "b", "c"],
            &["a", "b", "c"],
            &["a", "b", "b", "c"],
            &["a", "c", "b"],
            &["x", "y", "z"],
            &["x", "z", "y"],
            &["q"],
        ]);
        let first = variant_clusters(&log, "case", 3);
        let second = variant_clusters(&log, "case", 3);
        assert_eq!(first, second);
    }

    #[test]
    fn max_clusters_is_honored_when_similarity_allows() {
        // one family: every pairwise similarity stays above the floor
        let log = log_from_sequences(&[
            &["a", "b", "c"],
            &["a", "b", "c", "d"],
            &["a", "b", "c", "e"],
            &["a", "b", "c", "d", "e"],
        ]);
        let report = variant_clusters(&log, "case", 2);
        assert_eq!(report.variants, 4);
        assert_eq!(report.clusters.len(), 2);
        let total: usize = report.clusters.iter().map(|c| c.traces).sum();
        assert_eq!(total, 4);
    }

    #[test]
    fn dissimilar_singletons_stay_apart() {
        // disjoint alphabets: every similarity is 0, so even max_clusters = 1
        // honestly returns one cluster per variant
        let log = log_from_sequences(&[&["a"], &["b"], &["c"], &["d"]]);
        let report = variant_clusters(&log, "case", 1);
        assert_eq!(report.clusters.len(), 4);
        for cluster in &report.clusters {
            assert_eq!(cluster.traces, 1);
            assert_eq!(cluster.variants, 1);
        }
    }

    #[test]
    fn unknown_type_is_empty() {
        let log = log_from_sequences(&[&["a"]]);
        let report = variant_clusters(&log, "nope", 3);
        assert_eq!(report.variants, 0);
        assert!(report.clusters.is_empty());
    }
}
