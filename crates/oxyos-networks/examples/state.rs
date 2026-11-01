//! Prints the network as the session reads it from the system's NetworkManager (or the bus at the
//! address given), as JSON: what the shell and Settings would be told. Reads only.
//!
//! cargo run -p oxyos-networks --example state [ADDRESS]

fn main() -> std::io::Result<()> {
    let address = std::env::args().nth(1);
    let manager = oxyos_networks::NetworkManager::connect(address.as_deref())?;
    let state = manager.state()?;
    println!(
        "{}",
        serde_json::to_string_pretty(&state).expect("serializable")
    );
    Ok(())
}
