//! Replay a discovered model against its own log and print the fitness.
//!
//! Usage: `cargo run --release --example replay -- <log> <object_type> <inductive|alpha> [noise_threshold]`
//!
//! For `inductive` the tree is also printed in `PM4Py` notation so the result
//! can be cross-checked with `pm4py.parse_process_tree` + token replay.

use std::time::Instant;

use ocel_mine::ProcessTree;

fn pm4py_notation(tree: &ProcessTree) -> String {
    match tree {
        ProcessTree::Activity { label } => format!("'{label}'"),
        ProcessTree::Tau => "tau".to_owned(),
        ProcessTree::Sequence { children } => join("->", children),
        ProcessTree::Exclusive { children } => join("X", children),
        ProcessTree::Parallel { children } => join("+", children),
        ProcessTree::Loop { children } => {
            // pm4py loops are binary (body, redo); several redo parts fold
            // into an exclusive choice — language-equivalent
            let body = pm4py_notation(&children[0]);
            let redo = if children.len() == 2 {
                pm4py_notation(&children[1])
            } else {
                join("X", &children[1..])
            };
            format!("*( {body}, {redo} )")
        }
    }
}

fn join(op: &str, children: &[ProcessTree]) -> String {
    let parts: Vec<String> = children.iter().map(pm4py_notation).collect();
    format!("{op}( {} )", parts.join(", "))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(path), Some(object_type), Some(algo)) = (args.next(), args.next(), args.next())
    else {
        return Err("usage: replay <log> <object_type> <inductive|alpha> [noise_threshold]".into());
    };
    let noise: f64 = args.next().map_or(Ok(0.0), |s| s.parse())?;

    let log = ocel::io::read_path(&path)?;
    let start = Instant::now();
    let report = match algo.as_str() {
        "inductive" => {
            let tree = ocel_mine::inductive(&log, &object_type, noise);
            println!("tree: {}", pm4py_notation(&tree));
            ocel_mine::tree_replay(&log, &object_type, &tree)
        }
        "alpha" => {
            let net = ocel_mine::alpha(&log, &object_type);
            for warning in &net.warnings {
                println!("warning: {warning}");
            }
            ocel_mine::net_replay(&log, &object_type, &net)
        }
        other => return Err(format!("unknown algo: {other}").into()),
    };
    let elapsed = start.elapsed();

    // trace counts are far below 2^53, so the f64 percentage is exact enough
    #[allow(clippy::cast_precision_loss)]
    let pct = report.fitting as f64 / report.traces.max(1) as f64 * 100.0;
    println!(
        "{}: {} / {} traces fit ({pct:.2}%), {} / {} variants",
        report.object_type, report.fitting, report.traces, report.fitting_variants, report.variants
    );
    for misfit in report.misfits.iter().take(10) {
        println!(
            "  {:>5}x  {}  (e.g. {})",
            misfit.count,
            misfit.activities.join(" -> "),
            misfit.example
        );
    }
    eprintln!("replay: {elapsed:?}");
    Ok(())
}
