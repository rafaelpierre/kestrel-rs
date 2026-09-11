//! Compare first/subsequent calls through one pooled client (not CLI startup time).
use kestrelsearch::{Engine, KestrelClient, SearchOptions};
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: reuse_benchmark ENGINE 'QUERY'".into());
    }
    let engine: Engine = serde_json::from_value(serde_json::Value::String(args[0].clone()))?;
    let client = KestrelClient::new()?;
    let options = SearchOptions {
        engines: vec![engine],
        search_budget: Some(Duration::from_secs(5)),
        ..Default::default()
    };
    for round in 1..=3 {
        let started = Instant::now();
        let report = client
            .search_many_detailed(&[args[1].clone()], &options)
            .await;
        println!(
            "{}",
            serde_json::json!({"round": round, "seconds": started.elapsed().as_secs_f64(),
            "results": report.as_ref().ok().map(|r| &r.results), "error": report.as_ref().err().map(ToString::to_string)})
        );
    }
    Ok(())
}
