//! Per-type trace variants.
//!
//! A trace is one object's E2O-linked events ordered by `(time, event index)`;
//! a variant is the activity sequence shared by objects of one type. Computed
//! without flattening: only objects of the requested type contribute, so
//! divergence/convergence across types cannot distort counts.
//!
//! Hot-path design: activity names are interned to `u16` ids (cheap sequence
//! hashing/comparison), events are sorted once globally instead of per trace,
//! and no strings are cloned until the final report is assembled.

use std::collections::HashMap;

use ocel::Ocel;
use serde::Serialize;

use crate::trace;

/// One activity sequence and how many objects of the type follow it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Variant {
    /// Activity (event type) names in trace order.
    pub activities: Vec<String>,
    /// Objects whose trace is exactly this sequence.
    pub count: usize,
    /// One object id exhibiting this variant.
    pub example: String,
}

/// Trace variants of every object of one type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VariantsReport {
    pub object_type: String,
    /// Objects of the type in the log.
    pub objects: usize,
    /// Objects with at least one linked event (only these form variants).
    pub with_events: usize,
    /// Variants sorted by descending count, ties by activity sequence.
    pub variants: Vec<Variant>,
}

/// Compute trace variants for `object_type`.
///
/// An event linked to the same object through several qualifiers counts once.
/// Same-timestamp events are ordered by their index in `log.events`, making the
/// result deterministic.
#[must_use]
pub fn variants(log: &Ocel, object_type: &str) -> VariantsReport {
    let traces = trace::build(log, object_type);
    let sequences: Vec<Vec<u16>> = traces
        .steps
        .iter()
        .map(|steps| steps.iter().map(|&(activity, _)| activity).collect())
        .collect();

    // Group identical sequences; keys borrow the sequence buffers.
    let mut groups: HashMap<&[u16], (usize, usize)> = HashMap::new();
    let mut with_events = 0usize;
    for (slot, sequence) in sequences.iter().enumerate() {
        if sequence.is_empty() {
            continue;
        }
        with_events += 1;
        let entry = groups.entry(sequence.as_slice()).or_insert((0, slot));
        entry.0 += 1;
    }

    let mut variants: Vec<Variant> = groups
        .into_iter()
        .map(|(sequence, (count, example_slot))| Variant {
            activities: sequence
                .iter()
                .map(|&id| traces.activity_names[id as usize].to_owned())
                .collect(),
            count,
            example: traces.object_ids[example_slot].to_owned(),
        })
        .collect();
    variants.sort_unstable_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.activities.cmp(&b.activities))
    });

    VariantsReport {
        object_type: object_type.to_owned(),
        objects: traces.object_ids.len(),
        with_events,
        variants,
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

    fn log(events: Vec<Event>, objects: &[&str]) -> Ocel {
        let mut builder = Ocel::builder();
        for name in ["created", "closed", "changed"] {
            builder.add_event_type(EventType {
                name: name.into(),
                attributes: vec![],
            });
        }
        builder.add_object_type(ObjectType {
            name: "task".into(),
            attributes: vec![],
        });
        for id in objects {
            builder.add_object(Object {
                id: (*id).into(),
                object_type: "task".into(),
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
    fn groups_identical_sequences() {
        let log = log(
            vec![
                event("e1", "created", 0, &["a"]),
                event("e2", "closed", 1, &["a"]),
                event("e3", "created", 2, &["b"]),
                event("e4", "closed", 3, &["b"]),
                event("e5", "created", 4, &["c"]),
            ],
            &["a", "b", "c"],
        );
        let report = variants(&log, "task");
        assert_eq!(report.objects, 3);
        assert_eq!(report.with_events, 3);
        assert_eq!(report.variants.len(), 2);
        assert_eq!(report.variants[0].activities, ["created", "closed"]);
        assert_eq!(report.variants[0].count, 2);
        assert_eq!(report.variants[1].activities, ["created"]);
        assert_eq!(report.variants[1].count, 1);
        assert_eq!(report.variants[1].example, "c");
    }

    #[test]
    fn duplicate_links_from_one_event_count_once() {
        let mut e = event("e1", "created", 0, &["a", "a"]);
        e.relationships[1].qualifier = "other".into();
        let log = log(vec![e], &["a"]);
        let report = variants(&log, "task");
        assert_eq!(report.variants[0].activities, ["created"]);
    }

    #[test]
    fn same_timestamp_orders_by_event_index() {
        let log = log(
            vec![
                event("e2", "closed", 5, &["a"]),
                event("e1", "created", 5, &["a"]),
            ],
            &["a"],
        );
        let report = variants(&log, "task");
        // e2 comes first in log.events, so index order puts "closed" first.
        assert_eq!(report.variants[0].activities, ["closed", "created"]);
    }

    #[test]
    fn objects_without_events_form_no_variant() {
        let log = log(vec![event("e1", "created", 0, &["a"])], &["a", "b"]);
        let report = variants(&log, "task");
        assert_eq!(report.objects, 2);
        assert_eq!(report.with_events, 1);
        assert_eq!(report.variants.len(), 1);
    }

    #[test]
    fn unknown_type_is_empty() {
        let log = log(vec![], &[]);
        let report = variants(&log, "nope");
        assert_eq!(report.objects, 0);
        assert!(report.variants.is_empty());
    }
}
