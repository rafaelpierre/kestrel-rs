//! Offline release scaling benchmark; cloning/setup excluded, destruction included.
use kestrelsearch::{
    Engine, SearchResult, SourceOccurrence,
    ranking::{RankingPolicy, filter_fetch_candidates, rank_with_policy},
};
use std::{hint::black_box, time::Instant};

fn main() {
    let queries = vec![
        (0..10)
            .map(|i| format!("term{i}"))
            .collect::<Vec<_>>()
            .join(" "),
    ];
    for tokens in [200, 2000] {
        for count in [15, 100, 200, 400] {
            let input: Vec<_> = (0..count)
                .map(|i| {
                    let text = (0..tokens)
                        .map(|j| format!("term{}", (i + j) % 32))
                        .collect::<Vec<_>>()
                        .join(" ");
                    let mut hit = SearchResult {
                        title: format!("candidate {i}"),
                        url: format!("https://example.com/{i}"),
                        display_url: String::new(),
                        snippet: text.clone(),
                        content: None,
                        bm25_score: None,
                        engine: None,
                        query: None,
                        engine_rank: None,
                        sources: Vec::new(),
                    };
                    hit.content = Some(text);
                    hit.query = Some(queries[0].clone());
                    hit.sources = vec![SourceOccurrence {
                        engine: Engine::Bing,
                        query: queries[0].clone(),
                        rank: i + 1,
                    }];
                    hit
                })
                .collect();
            for policy in ["snippet", "hybrid", "rrf", "filter"] {
                let mut samples = Vec::new();
                for iteration in 0..13 {
                    let mut candidates = input.clone();
                    let start = Instant::now();
                    match policy {
                        "filter" => {
                            black_box(
                                filter_fetch_candidates(&mut candidates, &queries, 0.1).unwrap(),
                            );
                            drop(black_box(candidates));
                        }
                        _ => {
                            let rank = match policy {
                                "snippet" => RankingPolicy::Snippet,
                                "hybrid" => RankingPolicy::Hybrid,
                                _ => RankingPolicy::Rrf,
                            };
                            drop(black_box(rank_with_policy(candidates, &queries, rank)));
                        }
                    }
                    let ms = start.elapsed().as_secs_f64() * 1000.0;
                    if iteration >= 2 {
                        samples.push(ms);
                    }
                }
                samples.sort_by(f64::total_cmp);
                println!(
                    "{}",
                    serde_json::json!({"policy":policy,"candidates":count,"tokens_per_field":tokens,"samples_ms":samples,"p50_ms":samples[5],"p95_ms":samples[10]})
                );
            }
        }
    }
}
