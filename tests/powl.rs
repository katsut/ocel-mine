use ocel_mine::{inductive, powl, powl_precision, powl_replay, tree_precision, tree_replay, Powl};

mod util {
    use chrono::{DateTime, Utc};
    use ocel::{AttributeDefinition, Event, EventType, Object, ObjectType, Ocel, Relationship};

    fn ts(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    /// One object per sequence; events every second.
    pub fn log_from_sequences(sequences: &[&[&str]]) -> Ocel {
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
            for (position, &activity) in sequence.iter().enumerate() {
                if !event_types.iter().any(|t| t == activity) {
                    event_types.push(activity.to_owned());
                }
                events.push(Event {
                    id: format!("e{index}-{position}"),
                    event_type: activity.to_owned(),
                    time: ts(i64::try_from(index * 1000 + position).expect("small")),
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

fn activity(label: &str) -> Powl {
    Powl::Activity {
        label: label.into(),
    }
}

/// All five linear extensions of the N-shaped order a≺c, b≺c, b≺d
/// (a ∥ b, a ∥ d, c ∥ d).
const N_POSET: &[&[&str]] = &[
    &["a", "b", "c", "d"],
    &["a", "b", "d", "c"],
    &["b", "a", "c", "d"],
    &["b", "a", "d", "c"],
    &["b", "d", "a", "c"],
];

#[test]
fn n_poset_becomes_one_partial_order_node() {
    let log = log_from_sequences(N_POSET);
    let model = powl(&log, "case", 0.0);
    let Powl::PartialOrder { children, order } = &model else {
        panic!("expected a partial-order root, got {model:?}");
    };
    assert_eq!(children.len(), 4);
    // labels are in alphabet order a,b,c,d; expected reduced order:
    // a≺c, b≺c, b≺d
    assert_eq!(children[0], activity("a"));
    let mut sorted = order.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, vec![(0, 2), (1, 2), (1, 3)]);
}

#[test]
fn n_poset_powl_is_perfectly_fit_and_precise_where_the_tree_is_not() {
    let log = log_from_sequences(N_POSET);

    let model = powl(&log, "case", 0.0);
    let replay = powl_replay(&log, "case", &model);
    assert_eq!(replay.fitting, replay.traces);
    let precision = powl_precision(&log, "case", &model);
    assert!(
        (precision.precision - 1.0).abs() < 1e-9,
        "POWL should capture exactly the 5 linear extensions, got {}",
        precision.precision
    );

    // the tree miner cannot express the N shape: still 100% fit, but it
    // over-generalizes and loses precision
    let tree = inductive(&log, "case", 0.0);
    let tree_fit = tree_replay(&log, "case", &tree);
    assert_eq!(tree_fit.fitting, tree_fit.traces);
    let tree_prec = tree_precision(&log, "case", &tree);
    assert!(
        tree_prec.precision < 0.95,
        "expected the tree to be less precise, got {}",
        tree_prec.precision
    );
}

#[test]
fn order_violations_do_not_replay() {
    let log = log_from_sequences(N_POSET);
    let model = powl(&log, "case", 0.0);
    // c before a violates a≺c; d before b violates b≺d
    let bad = log_from_sequences(&[&["c", "a", "b", "d"], &["d", "b", "a", "c"]]);
    let replay = powl_replay(&bad, "case", &model);
    assert_eq!(replay.fitting, 0);
}

#[test]
fn sequence_is_a_total_order() {
    let log = log_from_sequences(&[&["a", "b", "c"], &["a", "b", "c"]]);
    let model = powl(&log, "case", 0.0);
    assert_eq!(
        model,
        Powl::PartialOrder {
            children: vec![activity("a"), activity("b"), activity("c")],
            order: vec![(0, 1), (1, 2)], // transitively reduced
        }
    );
    assert_eq!(powl_replay(&log, "case", &model).fitting, 2);
}

#[test]
fn parallel_is_an_empty_order() {
    let log = log_from_sequences(&[&["a", "b"], &["b", "a"]]);
    let model = powl(&log, "case", 0.0);
    assert_eq!(
        model,
        Powl::PartialOrder {
            children: vec![activity("a"), activity("b")],
            order: vec![],
        }
    );
}

#[test]
fn exclusive_and_loop_still_work() {
    let log = log_from_sequences(&[&["a", "b"], &["c", "d"]]);
    let Powl::Exclusive { children } = powl(&log, "case", 0.0) else {
        panic!("expected exclusive root");
    };
    assert_eq!(children.len(), 2);

    let log = log_from_sequences(&[&["a"], &["a", "b", "a"], &["a", "b", "a", "b", "a"]]);
    let model = powl(&log, "case", 0.0);
    assert_eq!(
        model,
        Powl::Loop {
            children: vec![activity("a"), activity("b")]
        }
    );
    let replay = powl_replay(&log, "case", &model);
    assert_eq!(replay.fitting, replay.traces);
}

#[test]
fn alternatives_inside_an_ordered_group_become_exclusive() {
    // a and b are alternatives (never co-occur), both before c
    let log = log_from_sequences(&[&["a", "c"], &["b", "c"]]);
    let model = powl(&log, "case", 0.0);
    let Powl::PartialOrder { children, order } = &model else {
        panic!("expected partial order, got {model:?}");
    };
    assert_eq!(order, &vec![(0, 1)]);
    let Powl::Exclusive { .. } = &children[0] else {
        panic!("expected exclusive first group, got {:?}", children[0]);
    };
    let replay = powl_replay(&log, "case", &model);
    assert_eq!(replay.fitting, 2);
}

#[test]
fn optional_group_stays_replayable() {
    // b sometimes skipped: child must accept ε through xor(τ, b)
    let log = log_from_sequences(&[&["a", "b", "c"], &["a", "c"]]);
    let model = powl(&log, "case", 0.0);
    let replay = powl_replay(&log, "case", &model);
    assert_eq!(replay.fitting, replay.traces);
}

#[test]
fn noise_threshold_ignores_rare_swap() {
    let mut sequences: Vec<&[&str]> = vec![&["a", "b", "c"]; 10];
    sequences.push(&["b", "a", "c"]);
    let log = log_from_sequences(&sequences);
    let filtered = powl(&log, "case", 0.2);
    assert_eq!(
        filtered,
        Powl::PartialOrder {
            children: vec![activity("a"), activity("b"), activity("c")],
            order: vec![(0, 1), (1, 2)],
        }
    );
}

#[test]
fn powl_round_trips_through_json() {
    let model = Powl::PartialOrder {
        children: vec![
            activity("a"),
            Powl::Exclusive {
                children: vec![Powl::Tau, activity("b")],
            },
            Powl::Loop {
                children: vec![activity("c"), Powl::Tau],
            },
        ],
        order: vec![(0, 1), (1, 2)],
    };
    let json = serde_json::to_string(&model).expect("serialize");
    let back: Powl = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, model);
}
