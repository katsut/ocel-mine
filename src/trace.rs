//! Shared per-type trace construction.
//!
//! One global `(time, event index)` sort replaces per-trace sorts; activity
//! names are interned to `u16` ids; an event linked to the same object through
//! several qualifiers contributes one step. Borrowed everywhere — no strings
//! are cloned here.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use ocel::Ocel;

pub(crate) struct Traces<'a> {
    /// Object ids of the requested type, slot-indexed.
    pub object_ids: Vec<&'a str>,
    /// Interned activity names.
    pub activity_names: Vec<&'a str>,
    /// Per slot: `(interned activity, event time)` in trace order.
    pub steps: Vec<Vec<(u16, DateTime<Utc>)>>,
}

pub(crate) fn build<'a>(log: &'a Ocel, object_type: &str) -> Traces<'a> {
    let mut slot_of: HashMap<&str, usize> = HashMap::new();
    let mut object_ids: Vec<&str> = Vec::new();
    for object in &log.objects {
        if object.object_type == object_type {
            slot_of.entry(object.id.as_str()).or_insert_with(|| {
                object_ids.push(object.id.as_str());
                object_ids.len() - 1
            });
        }
    }

    let mut order: Vec<usize> = (0..log.events.len()).collect();
    order.sort_unstable_by_key(|&i| (log.events[i].time, i));

    let mut activity_ids: HashMap<&str, u16> = HashMap::new();
    let mut activity_names: Vec<&str> = Vec::new();

    let mut steps: Vec<Vec<(u16, DateTime<Utc>)>> = vec![Vec::new(); object_ids.len()];
    let mut last_event: Vec<usize> = vec![usize::MAX; object_ids.len()];
    for &event_index in &order {
        let event = &log.events[event_index];
        let mut interned: Option<u16> = None;
        for relation in &event.relationships {
            let Some(&slot) = slot_of.get(relation.object_id.as_str()) else {
                continue;
            };
            if last_event[slot] == event_index {
                continue; // second qualifier of the same event
            }
            last_event[slot] = event_index;
            let activity = if let Some(id) = interned {
                id
            } else {
                let id = *activity_ids
                    .entry(event.event_type.as_str())
                    .or_insert_with(|| {
                        activity_names.push(event.event_type.as_str());
                        u16::try_from(activity_names.len() - 1)
                            .expect("more than u16::MAX event types")
                    });
                interned = Some(id);
                id
            };
            steps[slot].push((activity, event.time));
        }
    }

    Traces {
        object_ids,
        activity_names,
        steps,
    }
}
