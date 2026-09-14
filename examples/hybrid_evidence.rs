//! Issue #78 benchmark: fixed metadata selection, optional live fetch, full replay.
//! This is an experimental harness, not a production CLI interface.
use kestrelsearch::{
    FetchOptions, KestrelClient, SearchResult,
    ranking::{RankingPolicy, pre_rank_candidates, rank_with_policy},
};
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let start = Instant::now();
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3
        || !["metadata", "fetch", "replay"].contains(&args[1].as_str())
        || !["provider", "pre-rank"].contains(&args[2].as_str())
    {
        return Err(
            "usage: hybrid_evidence POOL.json metadata|fetch|replay provider|pre-rank".into(),
        );
    }
    let input: serde_json::Value = serde_json::from_slice(&std::fs::read(&args[0])?)?;
    let mut candidates: Vec<SearchResult> = serde_json::from_value(input["candidates"].clone())?;
    let queries: Vec<String> = serde_json::from_value(input["queries"].clone())?;
    if queries.is_empty() {
        return Err("at least one query is required".into());
    }
    if args[1] != "replay"
        && candidates
            .iter()
            .any(|r| r.content.is_some() || r.bm25_score.is_some())
    {
        return Err("live arms require unfetched, unranked metadata".into());
    }
    let available = candidates.len();
    if args[2] == "pre-rank" {
        candidates = pre_rank_candidates(candidates, &queries);
    }
    if args[1] != "replay" {
        candidates.truncate(15);
    }
    let selected_urls: Vec<_> = candidates.iter().map(|r| r.url.clone()).collect();
    let mut fetch_report = None;
    let mut fetch_seconds = 0.0;
    if args[1] == "fetch" {
        let fetchable: Vec<_> = candidates
            .iter()
            .enumerate()
            .filter(|(_, r)| !r.url.to_ascii_lowercase().contains(".pdf"))
            .map(|(i, _)| i)
            .collect();
        let urls: Vec<_> = fetchable
            .iter()
            .map(|&i| candidates[i].url.clone())
            .collect();
        let client = KestrelClient::new()?;
        let options = FetchOptions {
            timeout: Duration::from_secs(10),
            content_limit: 2000,
            max_concurrency: 10,
            parse_concurrency: 10,
            max_response_bytes: 1_000_000,
        };
        let clock = Instant::now();
        let report = client
            .fetch_all_detailed(&urls, &options, Some(Duration::from_secs(2)))
            .await?;
        fetch_seconds = clock.elapsed().as_secs_f64();
        for (&i, body) in fetchable.iter().zip(&report.contents) {
            candidates[i].content = body
                .as_ref()
                .map(|b| format!("Source: {}\n\n{b}", candidates[i].url));
        }
        fetch_report = Some(report);
    }
    let orderings: Vec<_> = [
        RankingPolicy::Provider,
        RankingPolicy::Snippet,
        RankingPolicy::Body,
        RankingPolicy::Hybrid,
    ]
    .into_iter()
    .map(|p| {
        serde_json::json!({
            "policy": format!("{p:?}"), "results": rank_with_policy(candidates.clone(), &queries, p)
        })
    })
    .collect();
    println!(
        "{}",
        serde_json::json!({"queries": queries, "available": available,
        "selected_urls": selected_urls, "candidates": candidates, "orderings": orderings,
        "fetch_report": fetch_report, "fetch_seconds": fetch_seconds,
        "elapsed_seconds": start.elapsed().as_secs_f64()})
    );
    Ok(())
}
