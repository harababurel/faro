//! Native reference run on a paused tokio clock (same as the mock's own tests).

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let n = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(300);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(std::env::args().nth(2).as_deref() != Some("--real"))
        .build()
        .expect("runtime");
    let lines = rt.block_on(mock_wasm_spike::first_events(n));
    println!("{}", lines.join("\n"));
    let round_trip = rt.block_on(mock_wasm_spike::send_round_trip());
    eprintln!("{}", round_trip.join("\n"));
}

#[cfg(target_arch = "wasm32")]
fn main() {}
