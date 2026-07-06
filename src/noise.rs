//! Synthetic noise injection for robustness benchmarking (PDC-style).
//!
//! Take a log you trust, inject a **known** rate of order swaps, event drops,
//! or duplicate events into one object type's traces, then discover on the
//! noisy log and evaluate fitness/precision **against the original clean log**
//! (see `examples/noise.rs`). The numbers then answer "did the miner recover
//! the true structure despite the noise" instead of "did it memorize the
//! noise".
//!
//! Injection is deterministic per [`NoiseSpec::seed`]. The perturbed order is
//! encoded in both the timestamps and the position in the output `events`
//! vec, so trace order (which breaks timestamp ties by event index) reflects
//! every swap even between same-second events. Events shared by several
//! objects of the target type move in all their traces at once, so effective
//! rates are approximate for convergent types; case-like types are exact.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use ocel::Ocel;

/// Independent probabilities for each noise kind, applied in the order
/// drop → swap → duplicate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoiseSpec {
    /// Probability that each adjacent pair in a (surviving) trace is swapped.
    /// Boundaries are walked left to right over the current order, so a rate
    /// of 1.0 turns `a,b,c` into `b,c,a`.
    pub swap: f64,
    /// Probability that each event related to the target type is removed.
    pub drop: f64,
    /// Probability that each surviving event is followed by a copy of itself
    /// (new id `<id>~dup<n>`, same time, same relationships).
    pub duplicate: f64,
    /// PRNG seed; the same log, type, spec, and seed give the same output.
    pub seed: u64,
}

/// xorshift64* — tiny, deterministic, no dependency.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1) // xorshift state must be non-zero
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn chance(&mut self, p: f64) -> bool {
        if p <= 0.0 {
            return false;
        }
        if p >= 1.0 {
            return true;
        }
        // 53 high bits → exact f64 in [0, 1)
        #[allow(clippy::cast_precision_loss)]
        let unit = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        unit < p
    }
}

/// Inject noise into the traces of `object_type` and return the noisy log.
///
/// The original log is untouched; evaluate discovered models against it.
#[must_use]
pub fn inject_noise(log: &Ocel, object_type: &str, spec: &NoiseSpec) -> Ocel {
    let mut rng = Rng::new(spec.seed);

    let members: HashSet<&str> = log
        .objects
        .iter()
        .filter(|object| object.object_type == object_type)
        .map(|object| object.id.as_str())
        .collect();
    let is_target = |index: usize| {
        log.events[index]
            .relationships
            .iter()
            .any(|relation| members.contains(relation.object_id.as_str()))
    };

    // Global order = (time, index), the same tie-breaking the miners use.
    let mut global: Vec<usize> = (0..log.events.len()).collect();
    global.sort_unstable_by_key(|&i| (log.events[i].time, i));

    // 1. drop — one decision per target event, in global order.
    let mut dropped: HashSet<usize> = HashSet::new();
    for &index in &global {
        if is_target(index) && rng.chance(spec.drop) {
            dropped.insert(index);
        }
    }

    // 2. swap — walk each surviving trace's boundaries left to right. A swap
    //    exchanges the two events' effective times AND their slots in the
    //    global order, so ties keep the intended order in the output vec.
    let mut times: Vec<DateTime<Utc>> = log.events.iter().map(|event| event.time).collect();
    let mut position: Vec<usize> = vec![0; log.events.len()];
    for (slot, &index) in global.iter().enumerate() {
        position[index] = slot;
    }

    let mut traces: Vec<Vec<usize>> = Vec::new();
    {
        let mut slot_of: HashMap<&str, usize> = HashMap::new();
        for object in &log.objects {
            if object.object_type == object_type && !slot_of.contains_key(object.id.as_str()) {
                slot_of.insert(object.id.as_str(), traces.len());
                traces.push(Vec::new());
            }
        }
        let mut last_event: Vec<usize> = vec![usize::MAX; traces.len()];
        for &index in &global {
            if dropped.contains(&index) {
                continue;
            }
            for relation in &log.events[index].relationships {
                let Some(&slot) = slot_of.get(relation.object_id.as_str()) else {
                    continue;
                };
                if last_event[slot] != index {
                    last_event[slot] = index;
                    traces[slot].push(index);
                }
            }
        }
    }

    for trace in &mut traces {
        for boundary in 0..trace.len().saturating_sub(1) {
            if !rng.chance(spec.swap) {
                continue;
            }
            let (a, b) = (trace[boundary], trace[boundary + 1]);
            times.swap(a, b);
            global.swap(position[a], position[b]);
            position.swap(a, b);
            trace.swap(boundary, boundary + 1);
        }
    }

    // 3. duplicate — one decision per surviving target event; the copy sits
    //    immediately after the original (same time, next vec position).
    let mut events = Vec::with_capacity(log.events.len());
    let mut dup_count = 0usize;
    for &index in &global {
        if dropped.contains(&index) {
            continue;
        }
        let mut event = log.events[index].clone();
        event.time = times[index];
        events.push(event);
        if is_target(index) && rng.chance(spec.duplicate) {
            dup_count += 1;
            let mut copy = events[events.len() - 1].clone();
            copy.id = format!("{}~dup{dup_count}", copy.id);
            events.push(copy);
        }
    }

    Ocel {
        event_types: log.event_types.clone(),
        object_types: log.object_types.clone(),
        events,
        objects: log.objects.clone(),
    }
}
