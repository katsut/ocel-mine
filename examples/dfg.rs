//! Print the directly-follows graph for one object type, with timing.
//!
//! Usage: `cargo run --release --example dfg -- <log> <object_type> [top]`

use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(path), Some(object_type)) = (args.next(), args.next()) else {
        return Err("usage: dfg <log> <object_type> [top]".into());
    };
    let top: usize = args.next().map_or(Ok(15), |s| s.parse())?;

    let log = ocel::io::read_path(&path)?;
    let start = Instant::now();
    let graph = ocel_mine::dfg(&log, &object_type);
    let elapsed = start.elapsed();

    println!(
        "{}: {} objects ({} with events), {} activities, {} edges",
        graph.object_type,
        graph.objects,
        graph.with_events,
        graph.nodes.len(),
        graph.edges.len()
    );
    for edge in graph.edges.iter().take(top) {
        println!(
            "  {:>6}  {} -> {}  (objects {}, median {:.0}s)",
            edge.frequency, edge.from, edge.to, edge.objects, edge.median_secs
        );
    }
    eprintln!("dfg: {elapsed:?}");
    Ok(())
}
