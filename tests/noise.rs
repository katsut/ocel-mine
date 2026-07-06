use ocel_mine::{inject_noise, variants, NoiseSpec};

mod util {
    use chrono::{DateTime, Utc};
    use ocel::{AttributeDefinition, Event, EventType, Object, ObjectType, Ocel, Relationship};

    fn ts(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    /// One object per sequence; events every second unless `same_time`.
    pub fn log_from_sequences(sequences: &[&[&str]], same_time: bool) -> Ocel {
        let mut event_types: Vec<String> = Vec::new();
        let mut events: Vec<Event> = Vec::new();
        let mut objects: Vec<Object> = Vec::new();
        for (index, sequence) in sequences.iter().enumerate() {
            let object_id = format!("o{index}");
            objects.push(Object {
                id: object_id.clone(),
                object_type: "case".into(),
                attributes: vec![],
                relationships: vec![],
            });
            for (offset, &activity) in sequence.iter().enumerate() {
                if !event_types.iter().any(|t| t == activity) {
                    event_types.push(activity.to_owned());
                }
                let secs = if same_time {
                    i64::try_from(index * 1000).expect("small")
                } else {
                    i64::try_from(index * 1000 + offset).expect("small")
                };
                events.push(Event {
                    id: format!("e{index}-{offset}"),
                    event_type: activity.to_owned(),
                    time: ts(secs),
                    attributes: vec![],
                    relationships: vec![Relationship {
                        object_id: object_id.clone(),
                        qualifier: "case".into(),
                    }],
                });
            }
        }
        Ocel {
            event_types: event_types
                .into_iter()
                .map(|name| EventType {
                    name,
                    attributes: Vec::<AttributeDefinition>::new(),
                })
                .collect(),
            object_types: vec![ObjectType {
                name: "case".into(),
                attributes: vec![],
            }],
            events,
            objects,
        }
    }
}

use util::log_from_sequences;

fn spec(swap: f64, drop: f64, duplicate: f64, seed: u64) -> NoiseSpec {
    NoiseSpec {
        swap,
        drop,
        duplicate,
        seed,
    }
}

fn first_variant(log: &ocel::Ocel) -> Vec<String> {
    let report = variants(log, "case");
    assert_eq!(report.variants.len(), 1, "expected a single variant");
    report.variants[0].activities.clone()
}

#[test]
fn zero_rates_are_identity() {
    let log = log_from_sequences(&[&["a", "b", "c"], &["a", "c"]], false);
    let noisy = inject_noise(&log, "case", &spec(0.0, 0.0, 0.0, 42));
    let ids: Vec<&str> = noisy.events.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, vec!["e0-0", "e0-1", "e0-2", "e1-0", "e1-1"]);
    let times: Vec<_> = noisy.events.iter().map(|e| e.time).collect();
    let original: Vec<_> = log.events.iter().map(|e| e.time).collect();
    assert_eq!(times, original);
}

fn fingerprint(log: &ocel::Ocel) -> Vec<(String, chrono::DateTime<chrono::Utc>)> {
    log.events.iter().map(|e| (e.id.clone(), e.time)).collect()
}

#[test]
fn same_seed_is_deterministic_and_seeds_differ() {
    let sequences: Vec<&[&str]> = vec![&["a", "b", "c", "d"]; 50];
    let log = log_from_sequences(&sequences, false);
    let once = inject_noise(&log, "case", &spec(0.3, 0.3, 0.3, 7));
    let twice = inject_noise(&log, "case", &spec(0.3, 0.3, 0.3, 7));
    assert_eq!(fingerprint(&once), fingerprint(&twice));
    let other = inject_noise(&log, "case", &spec(0.3, 0.3, 0.3, 8));
    assert_ne!(fingerprint(&once), fingerprint(&other));
}

#[test]
fn swap_walks_boundaries_left_to_right() {
    // rate 1.0 cascades: a,b,c -> b,a,c -> b,c,a
    let log = log_from_sequences(&[&["a", "b", "c"]], false);
    let noisy = inject_noise(&log, "case", &spec(1.0, 0.0, 0.0, 1));
    assert_eq!(first_variant(&noisy), vec!["b", "c", "a"]);
}

#[test]
fn swap_works_on_timestamp_ties() {
    // both events share one timestamp; order must still flip via vec order
    let log = log_from_sequences(&[&["a", "b"]], true);
    let noisy = inject_noise(&log, "case", &spec(1.0, 0.0, 0.0, 1));
    assert_eq!(first_variant(&noisy), vec!["b", "a"]);
}

#[test]
fn drop_removes_target_events_only() {
    let mut log = log_from_sequences(&[&["a", "b", "c"]], false);
    // second type whose events must survive
    log.object_types.push(ocel::ObjectType {
        name: "other".into(),
        attributes: vec![],
    });
    log.objects.push(ocel::Object {
        id: "x".into(),
        object_type: "other".into(),
        attributes: vec![],
        relationships: vec![],
    });
    log.events.push(ocel::Event {
        id: "ex".into(),
        event_type: "a".into(),
        time: log.events[0].time,
        attributes: vec![],
        relationships: vec![ocel::Relationship {
            object_id: "x".into(),
            qualifier: "case".into(),
        }],
    });
    let noisy = inject_noise(&log, "case", &spec(0.0, 1.0, 0.0, 3));
    let ids: Vec<&str> = noisy.events.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, vec!["ex"]);
}

#[test]
fn drop_rate_is_roughly_honored() {
    let sequences: Vec<&[&str]> = vec![&["a", "b", "c", "d"]; 250];
    let log = log_from_sequences(&sequences, false);
    let noisy = inject_noise(&log, "case", &spec(0.0, 0.3, 0.0, 11));
    let kept = noisy.events.len();
    // 1000 events at drop 0.3 → ~700 kept; allow a generous band
    assert!((620..=780).contains(&kept), "kept {kept}");
}

#[test]
fn duplicate_doubles_each_trace_and_stays_valid() {
    let log = log_from_sequences(&[&["a", "b"]], false);
    let noisy = inject_noise(&log, "case", &spec(0.0, 0.0, 1.0, 5));
    assert_eq!(first_variant(&noisy), vec!["a", "a", "b", "b"]);
    assert!(noisy.validate().is_ok());
}

#[test]
fn combined_noise_keeps_the_log_valid() {
    let sequences: Vec<&[&str]> = vec![&["a", "b", "c", "d", "e"]; 40];
    let log = log_from_sequences(&sequences, false);
    let noisy = inject_noise(&log, "case", &spec(0.2, 0.2, 0.2, 13));
    assert!(noisy.validate().is_ok());
}
