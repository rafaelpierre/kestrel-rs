//! Benchmark-only retained-client counterpart; arguments are controlled by run.py.
use kestrelsearch::{Engine, FetchOptions, KestrelClient, SearchOptions, budget_probe as probe};
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    probe::mark("main_entry");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    probe::mark("runtime_ready");
    runtime.block_on(run())?;
    probe::mark("handler_return");
    drop(runtime);
    probe::mark("runtime_dropped");
    probe::emit();
    Ok(())
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let rounds: usize = args[1].parse()?;
    let budget: f64 = args[2].parse()?;
    let fetch = args[3] == "fetch";
    let query = args[4].clone();
    probe::mark("client_begin");
    let client = KestrelClient::new()?;
    probe::mark("client_end");
    let options = SearchOptions {
        engines: vec![Engine::Bing, Engine::Yahoo],
        max_concurrency: 1,
        search_budget: Some(Duration::from_secs_f64(budget)),
        ..Default::default()
    };
    for round in 0..rounds {
        probe::mark("call_begin");
        let started = Instant::now();
        let result = client
            .search_many_detailed(std::slice::from_ref(&query), &options)
            .await;
        probe::mark("search_return");
        let search_seconds = started.elapsed().as_secs_f64();
        let mut content_count = 0;
        let mut fetch_seconds = 0.0;
        if fetch && let Ok(report) = &result {
            let urls: Vec<_> = report.results.iter().map(|r| r.url.clone()).collect();
            let started = Instant::now();
            probe::mark("fetch_begin");
            content_count = client
                .fetch_all(&urls, &FetchOptions::default())
                .await?
                .iter()
                .filter(|content| content.is_some())
                .count();
            probe::mark("fetch_end");
            fetch_seconds = started.elapsed().as_secs_f64();
        }
        probe::mark("call_end");
        println!(
            "{}",
            serde_json::json!({
                "round": round, "search_seconds": search_seconds, "fetch_seconds": fetch_seconds,
                "content_count": content_count,
                "results": result.as_ref().ok().map(|r| r.results.len()),
                "providers": result.as_ref().ok().map(|r| &r.providers),
                "error": result.as_ref().err().map(ToString::to_string),
            })
        );
    }
    probe::mark("client_drop_begin");
    drop(client);
    probe::mark("client_drop_end");
    Ok(())
}
