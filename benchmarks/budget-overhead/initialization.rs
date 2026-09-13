//! Isolate cached native-root loading without changing trust or client policy.
use kestrelsearch::KestrelClient;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let preload = std::env::args().nth(1).as_deref() == Some("preload");
    let started = Instant::now();
    if preload {
        let _ = primp::tls::default_root_store_arc();
    }
    let roots_seconds = started.elapsed().as_secs_f64();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let _guard = runtime.enter();
    for round in 0..10 {
        let started = Instant::now();
        let client = KestrelClient::new()?;
        let build_seconds = started.elapsed().as_secs_f64();
        let started = Instant::now();
        drop(client);
        println!(
            "{}",
            serde_json::json!({"preload": preload, "round": round,
            "roots_seconds": roots_seconds, "build_seconds": build_seconds,
            "drop_seconds": started.elapsed().as_secs_f64()})
        );
    }
    Ok(())
}
