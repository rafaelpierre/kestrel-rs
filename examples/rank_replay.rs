//! Replay all rank policies over identical captured candidates, without networking.
use kestrelsearch::{
    SearchResult,
    ranking::{RankingPolicy, rank_with_policy},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: rank_replay ARTIFACT.json")?;
    let artifact: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let candidates: Vec<SearchResult> = serde_json::from_value(artifact["candidates"].clone())?;
    let queries: Vec<String> = serde_json::from_value(artifact["queries"].clone())?;
    for policy in [
        RankingPolicy::Provider,
        RankingPolicy::Snippet,
        RankingPolicy::Body,
        RankingPolicy::Hybrid,
        RankingPolicy::Rrf,
    ] {
        let ranked = rank_with_policy(candidates.clone(), &queries, policy);
        println!(
            "{}",
            serde_json::json!({"policy": format!("{policy:?}"), "candidate_count": candidates.len(),
            "retained_count": ranked.len(), "results": ranked.into_iter().take(5).collect::<Vec<_>>()})
        );
    }
    Ok(())
}
