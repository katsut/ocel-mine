//! Noise-robustness harness: discover on a noisy log, evaluate on the clean one.
//!
//! Usage: `cargo run --release --example noise -- <log> <object_type> [seed]`
//!
//! For each noise kind (swap / drop / duplicate) and rate, the log's traces
//! for `object_type` are perturbed with [`ocel_mine::inject_noise`], a model
//! is discovered on the noisy log (inductive and POWL across their noise
//! thresholds, alpha as an unfiltered baseline), and replay fitness + ETC
//! precision are computed **against the original clean log**. High numbers
//! mean the miner recovered the true structure despite the noise.

use ocel::Ocel;

const RATES: [f64; 3] = [0.05, 0.1, 0.2];
const THRESHOLDS: [f64; 4] = [0.0, 0.1, 0.2, 0.4];

/// fitness%/precision% of a model mined on `mined_on`, judged against `clean`.
fn cell(clean: &Ocel, mined_on: &Ocel, object_type: &str, algo: &str, threshold: f64) -> String {
    let (replay, precision) = match algo {
        "inductive" => {
            let tree = ocel_mine::inductive(mined_on, object_type, threshold);
            (
                ocel_mine::tree_replay(clean, object_type, &tree),
                ocel_mine::tree_precision(clean, object_type, &tree),
            )
        }
        "powl" => {
            let model = ocel_mine::powl(mined_on, object_type, threshold);
            (
                ocel_mine::powl_replay(clean, object_type, &model),
                ocel_mine::powl_precision(clean, object_type, &model),
            )
        }
        "alpha" => {
            let net = ocel_mine::alpha(mined_on, object_type);
            (
                ocel_mine::net_replay(clean, object_type, &net),
                ocel_mine::net_precision(clean, object_type, &net),
            )
        }
        other => unreachable!("unknown algo {other}"),
    };
    // trace counts are far below 2^53, so the f64 percentage is exact enough
    #[allow(clippy::cast_precision_loss)]
    let fit = replay.fitting as f64 / replay.traces.max(1) as f64 * 100.0;
    format!("{fit:.1}/{:.1}", precision.precision * 100.0)
}

fn row(clean: &Ocel, mined_on: &Ocel, object_type: &str, label: &str) {
    for algo in ["inductive", "powl"] {
        let cells: Vec<String> = THRESHOLDS
            .iter()
            .map(|&threshold| cell(clean, mined_on, object_type, algo, threshold))
            .collect();
        println!("| {label} | {algo} | {} |", cells.join(" | "));
    }
    let alpha = cell(clean, mined_on, object_type, "alpha", 0.0);
    println!("| {label} | alpha | {alpha} | — | — | — |");
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(path), Some(object_type)) = (args.next(), args.next()) else {
        return Err("usage: noise <log> <object_type> [seed]".into());
    };
    let seed: u64 = args.next().map_or(Ok(7), |s| s.parse())?;

    let clean = ocel::io::read_path(&path)?;
    println!("cells are fitness%/precision% against the clean log; columns are");
    println!("miner noise thresholds {THRESHOLDS:?} (alpha has none)\n");
    println!("| noise | algo | t=0 | t=0.1 | t=0.2 | t=0.4 |");
    println!("|---|---|---|---|---|---|");
    row(&clean, &clean, &object_type, "none");

    for kind in ["swap", "drop", "duplicate"] {
        for rate in RATES {
            let spec = ocel_mine::NoiseSpec {
                swap: if kind == "swap" { rate } else { 0.0 },
                drop: if kind == "drop" { rate } else { 0.0 },
                duplicate: if kind == "duplicate" { rate } else { 0.0 },
                seed,
            };
            let noisy = ocel_mine::inject_noise(&clean, &object_type, &spec);
            row(&clean, &noisy, &object_type, &format!("{kind} {rate}"));
        }
    }
    Ok(())
}
