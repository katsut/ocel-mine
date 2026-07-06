//! Print per-type statistics for a log — the numbers behind "which type is
//! the case-like default".
//!
//! Usage: `cargo run --release --example stats -- <log>`

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(path) = std::env::args().nth(1) else {
        return Err("usage: stats <log>".into());
    };
    let log = ocel::io::read_path(&path)?;
    let log_span_secs = log
        .events
        .iter()
        .map(|e| e.time)
        .max()
        .zip(log.events.iter().map(|e| e.time).min())
        .map_or(0, |(max, min)| (max - min).num_seconds());
    let log_activities = log.event_types.len().max(1);
    // second counts are far below 2^53
    #[allow(clippy::cast_precision_loss)]
    let span_days = log_span_secs as f64 / 86_400.0;
    println!("log span: {span_days:.1} days");
    println!(
        "| type | objects | withEvents | median len | median active span | span/log | activities |"
    );
    println!("|---|---|---|---|---|---|---|");
    for stats in ocel_mine::type_stats(&log) {
        let days = stats.median_active_span_secs / 86_400.0;
        #[allow(clippy::cast_precision_loss)]
        let ratio = if log_span_secs > 0 {
            stats.median_active_span_secs / log_span_secs as f64
        } else {
            0.0
        };
        #[allow(clippy::cast_precision_loss)]
        let alphabet = stats.activity_types as f64 / log_activities as f64;
        println!(
            "| {} | {} | {} | {} | {days:.2}d | {ratio:.3} | {}/{log_activities} ({alphabet:.2}) |",
            stats.object_type,
            stats.objects,
            stats.with_events,
            stats.median_trace_len,
            stats.activity_types
        );
    }
    Ok(())
}
