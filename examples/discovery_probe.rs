//! Benchmark-only JSON-lines interface retaining one client across discovery calls.
//! Blocking stdin and trace flushing occur outside the async executor.
use kestrelsearch::{Engine, KestrelClient, SearchOptions};
use serde::Deserialize;
use std::io::{self, BufRead, Write};
use std::time::{Duration, Instant};

#[derive(Deserialize)]
struct Request {
    queries: Vec<String>,
    engines: Vec<Engine>,
    budget: f64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let runtime = tokio::runtime::Runtime::new()?;
    let client = KestrelClient::new()?;
    println!(
        "{}",
        serde_json::json!({"initialize_seconds": started.elapsed().as_secs_f64()})
    );
    io::stdout().flush()?;
    for line in io::stdin().lock().lines() {
        let request: Request = serde_json::from_str(&line?)?;
        if !request.budget.is_finite() || request.budget <= 0.0 || request.budget > 5.0 {
            return Err("probe budget must be finite and in (0, 5]".into());
        }
        let options = SearchOptions {
            engines: request.engines,
            min_results: Some(1),
            search_budget: Some(Duration::from_secs_f64(request.budget)),
            ..Default::default()
        };
        let start = Instant::now();
        let result = runtime.block_on(client.search_many_detailed(&request.queries, &options));
        let seconds = start.elapsed().as_secs_f64();
        let flushed = runtime.block_on(kestrelsearch::diagnostic_sink::flush(
            Duration::from_millis(200),
        ));
        println!(
            "{}",
            serde_json::json!({
                "seconds": seconds, "report": result.as_ref().ok(),
                "error": result.as_ref().err().map(ToString::to_string), "flushed": flushed,
            })
        );
        io::stdout().flush()?;
    }
    Ok(())
}
