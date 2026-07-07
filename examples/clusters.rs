//! Cluster trace variants of one object type into behavioral families.
//!
//! Usage: `cargo run --release --example clusters -- <log> <object_type> [max]`

use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(path), Some(object_type)) = (args.next(), args.next()) else {
        return Err("usage: clusters <log> <object_type> [max]".into());
    };
    let max_clusters: usize = args.next().map_or(Ok(10), |s| s.parse())?;

    let read_start = Instant::now();
    let log = ocel::io::read_path(&path)?;
    let read_elapsed = read_start.elapsed();

    let start = Instant::now();
    let report = ocel_mine::variant_clusters(&log, &object_type, max_clusters);
    let elapsed = start.elapsed();

    println!(
        "{}: {} variants, {} clusters (max {max_clusters})",
        report.object_type,
        report.variants,
        report.clusters.len()
    );
    for cluster in &report.clusters {
        println!(
            "  {:>6} traces  {:>5} variants  {}",
            cluster.traces,
            cluster.variants,
            cluster.representative.join(" -> ")
        );
        println!("          top: {}", cluster.top_activities.join(", "));
    }
    eprintln!("read: {read_elapsed:?}  clusters: {elapsed:?}");
    Ok(())
}
