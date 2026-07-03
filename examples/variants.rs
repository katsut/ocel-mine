//! Print trace variants for one object type of an OCEL 2.0 log, with timing.
//!
//! Usage: `cargo run --release --example variants -- <log> <object_type> [top]`

use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(path), Some(object_type)) = (args.next(), args.next()) else {
        return Err("usage: variants <log> <object_type> [top]".into());
    };
    let top: usize = args.next().map_or(Ok(10), |s| s.parse())?;

    let read_start = Instant::now();
    let log = ocel::io::read_path(&path)?;
    let read_elapsed = read_start.elapsed();

    let start = Instant::now();
    let report = ocel_mine::variants(&log, &object_type);
    let elapsed = start.elapsed();

    println!(
        "{}: {} objects, {} with events, {} variants",
        report.object_type,
        report.objects,
        report.with_events,
        report.variants.len()
    );
    for variant in report.variants.iter().take(top) {
        println!(
            "  {:>6}  {}",
            variant.count,
            variant.activities.join(" -> ")
        );
    }
    eprintln!("read: {read_elapsed:?}  variants: {elapsed:?}");
    Ok(())
}
