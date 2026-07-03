//! Shared test helpers.

use chrono::{Duration, TimeZone, Utc};
use ocel::{Event, EventType, Object, ObjectType, Ocel, Relationship};

/// Build a single-type log ("case") where each sequence becomes one object's
/// trace, events spaced one minute apart.
pub(crate) fn log_from_sequences(sequences: &[&[&str]]) -> Ocel {
    let mut builder = Ocel::builder();
    let mut declared = std::collections::BTreeSet::new();
    for sequence in sequences {
        for activity in *sequence {
            declared.insert(*activity);
        }
    }
    for name in &declared {
        builder.add_event_type(EventType {
            name: (*name).into(),
            attributes: vec![],
        });
    }
    builder.add_object_type(ObjectType {
        name: "case".into(),
        attributes: vec![],
    });
    let base = Utc.with_ymd_and_hms(2026, 1, 1, 9, 0, 0).unwrap();
    let mut event_id = 0usize;
    for (i, sequence) in sequences.iter().enumerate() {
        let object_id = format!("o{i}");
        builder.add_object(Object {
            id: object_id.clone(),
            object_type: "case".into(),
            attributes: vec![],
            relationships: vec![],
        });
        for (j, activity) in sequence.iter().enumerate() {
            event_id += 1;
            let offset = i64::try_from(i * 1000 + j).expect("small test data");
            builder.add_event(Event {
                id: format!("e{event_id}"),
                event_type: (*activity).into(),
                time: base + Duration::minutes(offset),
                attributes: vec![],
                relationships: vec![Relationship {
                    object_id: object_id.clone(),
                    qualifier: "q".into(),
                }],
            });
        }
    }
    builder.build().expect("valid log")
}
