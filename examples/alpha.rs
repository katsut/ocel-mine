//! Discover a Petri net with the alpha algorithm and print it.
//!
//! Usage: `cargo run --release --example alpha -- <log> <object_type>`

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(path), Some(object_type)) = (args.next(), args.next()) else {
        return Err("usage: alpha <log> <object_type>".into());
    };

    let log = ocel::io::read_path(&path)?;
    let net = ocel_mine::alpha(&log, &object_type);

    println!(
        "{}: {} transitions, {} places",
        net.object_type,
        net.transitions.len(),
        net.places.len()
    );
    for place in &net.places {
        println!("  {}: {:?} -> {:?}", place.id, place.inputs, place.outputs);
    }
    for warning in &net.warnings {
        eprintln!("warning: {warning}");
    }
    Ok(())
}
