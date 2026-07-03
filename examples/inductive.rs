//! Discover a process tree with the inductive miner and print it.
//!
//! Usage: `cargo run --release --example inductive -- <log> <object_type> [noise_threshold]`

use std::time::Instant;

use ocel_mine::ProcessTree;

fn print_tree(tree: &ProcessTree, indent: usize) {
    let pad = "  ".repeat(indent);
    match tree {
        ProcessTree::Activity { label } => println!("{pad}{label}"),
        ProcessTree::Tau => println!("{pad}tau"),
        ProcessTree::Sequence { children } => {
            print_children(&format!("{pad}seq"), children, indent);
        }
        ProcessTree::Exclusive { children } => {
            print_children(&format!("{pad}xor"), children, indent);
        }
        ProcessTree::Parallel { children } => {
            print_children(&format!("{pad}and"), children, indent);
        }
        ProcessTree::Loop { children } => print_children(&format!("{pad}loop"), children, indent),
    }
}

fn print_children(head: &str, children: &[ProcessTree], indent: usize) {
    println!("{head}");
    for child in children {
        print_tree(child, indent + 1);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(path), Some(object_type)) = (args.next(), args.next()) else {
        return Err("usage: inductive <log> <object_type> [noise_threshold]".into());
    };
    let noise_threshold: f64 = args.next().map_or(Ok(0.0), |s| s.parse())?;

    let log = ocel::io::read_path(&path)?;
    let start = Instant::now();
    let tree = ocel_mine::inductive(&log, &object_type, noise_threshold);
    let elapsed = start.elapsed();

    print_tree(&tree, 0);
    eprintln!("inductive: {elapsed:?}");
    Ok(())
}
