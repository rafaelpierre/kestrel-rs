//! Benchmark live fetching on prefixes of a frozen, discovered metadata pool.
//! Excludes discovery latency; uses production fetching and hybrid ranking.
use kestrelsearch::{
    FetchOptions, KestrelClient, SearchResult,
    ranking::{RankingPolicy, rank_with_policy},
};
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: fetch_cap_replay POOL.json CAP (5, 10, or 15)".into());
    }
    let cap: usize = args[1].parse()?;
    if ![5, 10, 15].contains(&cap) {
        return Err("cap must be 5, 10, or 15".into());
    }
    let pool: serde_json::Value = serde_json::from_slice(&std::fs::read(&args[0])?)?;
    let mut candidates: Vec<SearchResult> = serde_json::from_value(pool["candidates"].clone())?;
    let queries: Vec<String> = serde_json::from_value(pool["queries"].clone())?;
    let unique: std::collections::HashSet<_> = candidates.iter().map(|r| &r.url).collect();
    if candidates.len() < 15 || unique.len() != candidates.len() || queries.len() != 1 {
        return Err("pool needs at least 15 unique URLs and exactly one query".into());
    }
    if candidates
        .iter()
        .any(|r| r.content.is_some() || r.bm25_score.is_some())
    {
        return Err("pool must contain unranked, unfetched metadata".into());
    }
    let available = candidates.len();
    candidates.truncate(cap);
    let fetchable: Vec<usize> = candidates
        .iter()
        .enumerate()
        .filter(|(_, r)| !r.url.to_ascii_lowercase().contains(".pdf"))
        .map(|(i, _)| i)
        .collect();
    let urls: Vec<String> = fetchable
        .iter()
        .map(|&i| candidates[i].url.clone())
        .collect();
    let options = FetchOptions {
        timeout: Duration::from_secs(10),
        content_limit: 2000,
        max_concurrency: 10,
        parse_concurrency: 10,
        max_response_bytes: 1_000_000,
    };
    let client = KestrelClient::new()?;
    let fetch_started = Instant::now();
    let report = client
        .fetch_all_detailed(&urls, &options, Some(Duration::from_secs(5)))
        .await?;
    let fetch_seconds = fetch_started.elapsed().as_secs_f64();
    for (&index, content) in fetchable.iter().zip(&report.contents) {
        let result = &mut candidates[index];
        result.content = content
            .as_ref()
            .map(|body| format!("Source: {}\n\n{body}", result.url));
    }
    let rank_started = Instant::now();
    let ranked = rank_with_policy(candidates.clone(), &queries, RankingPolicy::Hybrid);
    let rank_seconds = rank_started.elapsed().as_secs_f64();
    println!(
        "{}",
        serde_json::json!({
            "available": available, "cap": cap, "selected_urls": urls,
            "candidates": candidates, "fetch_report": report,
            "results": ranked.into_iter().take(5).collect::<Vec<_>>(),
            "fetch_seconds": fetch_seconds, "rank_seconds": rank_seconds,
            "elapsed_seconds": started.elapsed().as_secs_f64(),
        })
    );
    Ok(())
}
