//! Print the heuristics dependency graph for one object type, with timing.
//!
//! Usage: `cargo run --release --example heuristics -- <log> <object_type> [dependency_threshold]`

use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(path), Some(object_type)) = (args.next(), args.next()) else {
        return Err("usage: heuristics <log> <object_type> [dependency_threshold]".into());
    };
    let mut params = ocel_mine::HeuristicsParams::default();
    if let Some(threshold) = args.next() {
        params.dependency_threshold = threshold.parse()?;
    }

    let log = ocel::io::read_path(&path)?;
    let start = Instant::now();
    let net = ocel_mine::heuristics(&log, &object_type, &params);
    let elapsed = start.elapsed();

    println!(
        "{}: {} objects ({} with events), {} activities, {} edges (dependency >= {})",
        net.object_type,
        net.objects,
        net.with_events,
        net.activities.len(),
        net.edges.len(),
        params.dependency_threshold
    );
    for edge in &net.edges {
        println!(
            "  {:>6}  {} -> {}  (dependency {:.6})",
            edge.frequency, edge.from, edge.to, edge.dependency
        );
    }
    eprintln!("heuristics: {elapsed:?}");
    Ok(())
}
