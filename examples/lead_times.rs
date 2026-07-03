//! Print lead-time metrics for one object type.
//!
//! Usage: `cargo run --release --example lead_times -- <log> <object_type>`

use std::time::Instant;

fn days(secs: f64) -> f64 {
    secs / 86400.0
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(path), Some(object_type)) = (args.next(), args.next()) else {
        return Err("usage: lead_times <log> <object_type>".into());
    };

    let log = ocel::io::read_path(&path)?;
    let start = Instant::now();
    let report = ocel_mine::lead_times(&log, &object_type);
    let elapsed = start.elapsed();

    println!(
        "{}: {} traces, lead median {:.1}d mean {:.1}d p90 {:.1}d",
        report.object_type,
        report.measured,
        days(report.median_secs),
        days(report.mean_secs),
        days(report.p90_secs),
    );
    if let Some(top) = report.variants.first() {
        println!(
            "happy path ({} traces): median {:.1}d  |  rest ({} traces): median {:.1}d",
            top.count,
            days(top.median_secs),
            report.rest_count,
            days(report.rest_median_secs),
        );
    }
    for variant in report.variants.iter().take(5) {
        println!(
            "  {:>5} × median {:>6.1}d  {}",
            variant.count,
            days(variant.median_secs),
            variant.activities.join(" -> ")
        );
    }
    for rework in report.rework.iter().take(5) {
        println!(
            "  rework: {} — {} traces, {} extra occurrences",
            rework.activity, rework.traces, rework.extra_occurrences
        );
    }
    eprintln!("lead_times: {elapsed:?}");
    Ok(())
}
