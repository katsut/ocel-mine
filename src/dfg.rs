//! Per-type directly-follows graphs and the object-centric overlay.
//!
//! Edges are computed **within one object type only** (ADR 0001): a naive
//! flattened DFG double-counts shared events and manufactures orderings across
//! unrelated lifecycles. The OC-DFG is the overlay of per-type DFGs — every
//! edge keeps its object type, and activity nodes carry per-type annotations
//! plus an honest total (an event touching several included types counts once).

use std::collections::{HashMap, HashSet};

use ocel::Ocel;
use serde::Serialize;

use crate::trace;

/// An activity node of one type's DFG.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DfgNode {
    pub activity: String,
    /// Trace steps with this activity.
    pub events: usize,
    /// Distinct objects touching the activity.
    pub objects: usize,
    /// Traces starting here.
    pub starts: usize,
    /// Traces ending here.
    pub ends: usize,
}

/// A directly-follows edge of one type's DFG.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DfgEdge {
    pub from: String,
    pub to: String,
    /// Directly-follows occurrences.
    pub frequency: usize,
    /// Distinct objects exhibiting the edge.
    pub objects: usize,
    /// Median gap between the two events, in seconds.
    pub median_secs: f64,
    /// Mean gap between the two events, in seconds.
    pub mean_secs: f64,
}

/// The directly-follows graph of one object type.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dfg {
    pub object_type: String,
    pub objects: usize,
    pub with_events: usize,
    /// Nodes sorted by descending event count, ties by activity name.
    pub nodes: Vec<DfgNode>,
    /// Edges sorted by descending frequency, ties by (from, to).
    pub edges: Vec<DfgEdge>,
}

struct NodeAgg {
    events: usize,
    objects: usize,
    last_slot: usize,
    starts: usize,
    ends: usize,
}

struct EdgeAgg {
    frequency: usize,
    objects: usize,
    last_slot: usize,
    gaps: Vec<i64>,
}

// Gap seconds of realistic logs are far below 2^53, so f64 stats are exact
// enough; the lossy usize/i64 → f64 casts here are intentional.
#[allow(clippy::cast_precision_loss)]
fn gap_stats(gaps: &mut [i64]) -> (f64, f64) {
    gaps.sort_unstable();
    let n = gaps.len();
    let median = if n % 2 == 1 {
        gaps[n / 2] as f64
    } else {
        (gaps[n / 2 - 1] + gaps[n / 2]) as f64 / 2.0
    };
    let mean = gaps.iter().sum::<i64>() as f64 / n as f64;
    (median, mean)
}

/// Compute the directly-follows graph for `object_type`.
#[must_use]
pub fn dfg(log: &Ocel, object_type: &str) -> Dfg {
    let traces = trace::build(log, object_type);

    let mut nodes: Vec<NodeAgg> = (0..traces.activity_names.len())
        .map(|_| NodeAgg {
            events: 0,
            objects: 0,
            last_slot: usize::MAX,
            starts: 0,
            ends: 0,
        })
        .collect();
    let mut edges: HashMap<u32, EdgeAgg> = HashMap::new();

    let mut with_events = 0usize;
    for (slot, steps) in traces.steps.iter().enumerate() {
        let (Some(&(first, _)), Some(&(last, _))) = (steps.first(), steps.last()) else {
            continue;
        };
        with_events += 1;
        nodes[first as usize].starts += 1;
        nodes[last as usize].ends += 1;
        for &(activity, _) in steps {
            let node = &mut nodes[activity as usize];
            node.events += 1;
            if node.last_slot != slot {
                node.last_slot = slot;
                node.objects += 1;
            }
        }
        for pair in steps.windows(2) {
            let (from, from_time) = pair[0];
            let (to, to_time) = pair[1];
            let key = (u32::from(from) << 16) | u32::from(to);
            let agg = edges.entry(key).or_insert_with(|| EdgeAgg {
                frequency: 0,
                objects: 0,
                last_slot: usize::MAX,
                gaps: Vec::new(),
            });
            agg.frequency += 1;
            if agg.last_slot != slot {
                agg.last_slot = slot;
                agg.objects += 1;
            }
            agg.gaps.push((to_time - from_time).num_seconds());
        }
    }

    let mut nodes: Vec<DfgNode> = nodes
        .into_iter()
        .enumerate()
        .filter(|(_, agg)| agg.events > 0)
        .map(|(id, agg)| DfgNode {
            activity: traces.activity_names[id].to_owned(),
            events: agg.events,
            objects: agg.objects,
            starts: agg.starts,
            ends: agg.ends,
        })
        .collect();
    nodes.sort_unstable_by(|a, b| {
        b.events
            .cmp(&a.events)
            .then_with(|| a.activity.cmp(&b.activity))
    });

    let mut edges: Vec<DfgEdge> = edges
        .into_iter()
        .map(|(key, mut agg)| {
            let (median_secs, mean_secs) = gap_stats(&mut agg.gaps);
            DfgEdge {
                from: traces.activity_names[(key >> 16) as usize].to_owned(),
                to: traces.activity_names[(key & 0xffff) as usize].to_owned(),
                frequency: agg.frequency,
                objects: agg.objects,
                median_secs,
                mean_secs,
            }
        })
        .collect();
    edges.sort_unstable_by(|a, b| {
        b.frequency
            .cmp(&a.frequency)
            .then_with(|| (a.from.as_str(), a.to.as_str()).cmp(&(b.from.as_str(), b.to.as_str())))
    });

    Dfg {
        object_type: object_type.to_owned(),
        objects: traces.object_ids.len(),
        with_events,
        nodes,
        edges,
    }
}

/// Per-type annotation of one activity in the OC-DFG.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcTypeCount {
    pub object_type: String,
    pub events: usize,
    pub objects: usize,
    pub starts: usize,
    pub ends: usize,
}

/// One activity of the OC-DFG with its per-type annotations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcActivity {
    pub activity: String,
    /// Events with this activity touching at least one included type,
    /// counted once even when they touch several types (convergence-honest).
    pub events: usize,
    pub per_type: Vec<OcTypeCount>,
}

/// A per-type edge of the OC-DFG overlay.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcDfgEdge {
    pub object_type: String,
    #[serde(flatten)]
    pub edge: DfgEdge,
}

/// The object-centric DFG: per-type DFGs overlaid, nothing flattened.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcDfg {
    pub object_types: Vec<String>,
    /// Activities sorted by descending total event count, ties by name.
    pub activities: Vec<OcActivity>,
    /// All per-type edges, sorted by descending frequency.
    pub edges: Vec<OcDfgEdge>,
}

/// Compute the OC-DFG overlay for `object_types`.
#[must_use]
pub fn oc_dfg(log: &Ocel, object_types: &[&str]) -> OcDfg {
    let included: HashSet<&str> = log
        .objects
        .iter()
        .filter(|o| object_types.contains(&o.object_type.as_str()))
        .map(|o| o.id.as_str())
        .collect();
    let mut event_totals: HashMap<&str, usize> = HashMap::new();
    for event in &log.events {
        if event
            .relationships
            .iter()
            .any(|r| included.contains(r.object_id.as_str()))
        {
            *event_totals.entry(event.event_type.as_str()).or_insert(0) += 1;
        }
    }

    let mut activities: HashMap<String, Vec<OcTypeCount>> = HashMap::new();
    let mut edges: Vec<OcDfgEdge> = Vec::new();
    for &object_type in object_types {
        let graph = dfg(log, object_type);
        for node in graph.nodes {
            activities
                .entry(node.activity)
                .or_default()
                .push(OcTypeCount {
                    object_type: object_type.to_owned(),
                    events: node.events,
                    objects: node.objects,
                    starts: node.starts,
                    ends: node.ends,
                });
        }
        edges.extend(graph.edges.into_iter().map(|edge| OcDfgEdge {
            object_type: object_type.to_owned(),
            edge,
        }));
    }

    let mut activities: Vec<OcActivity> = activities
        .into_iter()
        .map(|(activity, per_type)| OcActivity {
            events: event_totals.get(activity.as_str()).copied().unwrap_or(0),
            activity,
            per_type,
        })
        .collect();
    activities.sort_unstable_by(|a, b| {
        b.events
            .cmp(&a.events)
            .then_with(|| a.activity.cmp(&b.activity))
    });
    edges.sort_unstable_by(|a, b| b.edge.frequency.cmp(&a.edge.frequency));

    OcDfg {
        object_types: object_types.iter().map(|&t| t.to_owned()).collect(),
        activities,
        edges,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use ocel::{Event, EventType, Object, ObjectType, Relationship};

    fn rel(object_id: &str) -> Relationship {
        Relationship {
            object_id: object_id.into(),
            qualifier: "q".into(),
        }
    }

    fn event(id: &str, event_type: &str, minute: u32, objects: &[&str]) -> Event {
        Event {
            id: id.into(),
            event_type: event_type.into(),
            time: Utc.with_ymd_and_hms(2026, 1, 1, 9, minute, 0).unwrap(),
            attributes: vec![],
            relationships: objects.iter().map(|o| rel(o)).collect(),
        }
    }

    fn log(events: Vec<Event>, objects: &[(&str, &str)]) -> Ocel {
        let mut builder = Ocel::builder();
        for name in ["created", "changed", "closed"] {
            builder.add_event_type(EventType {
                name: name.into(),
                attributes: vec![],
            });
        }
        for type_name in ["task", "user"] {
            builder.add_object_type(ObjectType {
                name: type_name.into(),
                attributes: vec![],
            });
        }
        for (id, type_name) in objects {
            builder.add_object(Object {
                id: (*id).into(),
                object_type: (*type_name).into(),
                attributes: vec![],
                relationships: vec![],
            });
        }
        for e in events {
            builder.add_event(e);
        }
        builder.build().expect("valid log")
    }

    #[test]
    // whole-second gaps are exactly representable in f64
    #[allow(clippy::float_cmp)]
    fn aggregates_edges_with_durations() {
        let log = log(
            vec![
                event("e1", "created", 0, &["a"]),
                event("e2", "changed", 1, &["a"]),
                event("e3", "closed", 3, &["a"]),
                event("e4", "created", 10, &["b"]),
                event("e5", "changed", 13, &["b"]),
                event("e6", "closed", 14, &["b"]),
                event("e7", "created", 20, &["c"]),
                event("e8", "closed", 21, &["c"]),
            ],
            &[("a", "task"), ("b", "task"), ("c", "task")],
        );
        let graph = dfg(&log, "task");
        assert_eq!(graph.objects, 3);
        assert_eq!(graph.with_events, 3);

        let created = graph
            .nodes
            .iter()
            .find(|n| n.activity == "created")
            .unwrap();
        assert_eq!((created.events, created.objects), (3, 3));
        assert_eq!((created.starts, created.ends), (3, 0));
        let closed = graph.nodes.iter().find(|n| n.activity == "closed").unwrap();
        assert_eq!((closed.starts, closed.ends), (0, 3));

        let created_changed = graph
            .edges
            .iter()
            .find(|e| e.from == "created" && e.to == "changed")
            .unwrap();
        assert_eq!(created_changed.frequency, 2);
        assert_eq!(created_changed.objects, 2);
        // gaps: 60s and 180s
        assert_eq!(created_changed.median_secs, 120.0);
        assert_eq!(created_changed.mean_secs, 120.0);

        let created_closed = graph
            .edges
            .iter()
            .find(|e| e.from == "created" && e.to == "closed")
            .unwrap();
        assert_eq!(created_closed.frequency, 1);
        assert_eq!(created_closed.median_secs, 60.0);
    }

    #[test]
    fn oc_dfg_counts_shared_events_once() {
        let log = log(
            vec![
                event("e1", "created", 0, &["a", "u"]),
                event("e2", "closed", 1, &["a", "u"]),
            ],
            &[("a", "task"), ("u", "user")],
        );
        let graph = oc_dfg(&log, &["task", "user"]);
        let created = graph
            .activities
            .iter()
            .find(|a| a.activity == "created")
            .unwrap();
        // one event touching both types counts once
        assert_eq!(created.events, 1);
        assert_eq!(created.per_type.len(), 2);
        // one created->closed edge per type
        assert_eq!(graph.edges.len(), 2);
        assert!(graph
            .edges
            .iter()
            .all(|e| e.edge.from == "created" && e.edge.to == "closed"));
    }

    #[test]
    fn empty_type_yields_empty_graph() {
        let log = log(vec![], &[]);
        let graph = dfg(&log, "task");
        assert_eq!(graph.objects, 0);
        assert!(graph.nodes.is_empty());
        assert!(graph.edges.is_empty());
    }
}
