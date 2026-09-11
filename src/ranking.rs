//! Python-compatible BM25 ranking and fair multi-query interleaving.

use std::collections::{HashMap, HashSet};

use once_cell::sync::Lazy;
use regex::Regex;

use crate::model::{Engine, SearchResult};

static WORD_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?u)\b\w+\b").expect("valid tokenizer"));

/// Lowercase Unicode word tokenizer matching the Python implementation.
pub fn tokenize(text: &str) -> Vec<String> {
    WORD_RE
        .find_iter(&text.to_lowercase())
        .map(|matched| matched.as_str().to_owned())
        .collect()
}

static EVIDENCE_TOKEN_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?u)\b(?:\d+(?:\.\d+)+|\w+(?:-\w+)*)\b").expect("valid evidence tokenizer")
});

fn evidence_tokens(text: &str) -> Vec<String> {
    EVIDENCE_TOKEN_RE
        .find_iter(&text.to_lowercase())
        .map(|m| m.as_str().to_owned())
        .collect()
}

/// Experimental policies; legacy content-only BM25 remains the default.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum RankingPolicy {
    Provider,
    Snippet,
    Body,
    Hybrid,
    Rrf,
}

/// Rank without discarding candidates in snippet/hybrid modes. Scores stay internal;
/// the public bm25_score field retains its content-only meaning.
pub fn rank_with_policy(
    results: Vec<SearchResult>,
    queries: &[String],
    policy: RankingPolicy,
) -> Vec<SearchResult> {
    if matches!(policy, RankingPolicy::Provider) {
        return results;
    }
    if matches!(policy, RankingPolicy::Body) {
        return rank_results_by_query(results, queries);
    }
    let mut groups: Vec<Vec<SearchResult>> = vec![Vec::new(); queries.len() + 1];
    for result in results {
        let index = queries
            .iter()
            .position(|q| Some(q.as_str()) == result.query.as_deref())
            .unwrap_or(queries.len());
        groups[index].push(result);
    }
    for (index, group) in groups.iter_mut().enumerate() {
        let query = queries
            .get(index)
            .cloned()
            .unwrap_or_else(|| queries.join(" "));
        let terms = evidence_tokens(
            &query
                .split_whitespace()
                .filter(|term| !term.contains(':'))
                .collect::<Vec<_>>()
                .join(" "),
        );
        let documents: Vec<_> = group
            .iter()
            .map(|r| {
                let body = if matches!(policy, RankingPolicy::Hybrid) {
                    let body = r.content.as_deref().unwrap_or_default();
                    body.strip_prefix("Source: ")
                        .and_then(|_| body.split_once("\n\n").map(|(_, text)| text))
                        .unwrap_or(body)
                } else {
                    ""
                };
                evidence_tokens(&format!("{} {} {} {}", r.title, r.title, r.snippet, body))
            })
            .collect();
        let scores = positive_bm25_scores(&documents, &terms);
        let mut scored: Vec<_> = group.drain(..).zip(scores).collect();
        if matches!(policy, RankingPolicy::Rrf) {
            for (result, score) in &mut scored {
                let mut seen = HashSet::new();
                *score = result
                    .sources
                    .iter()
                    .filter(|source| source.query == query && seen.insert(source.engine))
                    .map(|source| 1.0 / (60.0 + source.rank as f64))
                    .sum();
            }
        }
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        *group = scored.into_iter().map(|(result, _)| result).collect();
    }
    interleave(groups)
}

fn positive_bm25_scores(corpus: &[Vec<String>], query: &[String]) -> Vec<f64> {
    if corpus.is_empty() {
        return Vec::new();
    }
    let average =
        (corpus.iter().map(Vec::len).sum::<usize>() as f64 / corpus.len() as f64).max(1.0);
    corpus
        .iter()
        .map(|document| {
            query
                .iter()
                .map(|term| {
                    let tf = document.iter().filter(|t| *t == term).count() as f64;
                    let df = corpus.iter().filter(|d| d.contains(term)).count() as f64;
                    let idf = (1.0 + (corpus.len() as f64 - df + 0.5) / (df + 0.5)).ln();
                    idf * tf * 2.5 / (tf + 1.5 * (0.25 + 0.75 * document.len() as f64 / average))
                })
                .sum()
        })
        .collect()
}

/// Rank fetched results with rank_bm25's BM25Okapi defaults.
pub fn rank_results(mut results: Vec<SearchResult>, query: &str) -> Vec<SearchResult> {
    if results.is_empty() {
        return results;
    }
    score_results(&mut results, query);
    retain_ranked(results)
}

fn score_results(results: &mut [SearchResult], query: &str) {
    let corpus: Vec<Vec<String>> = results
        .iter()
        .map(|result| tokenize(result.content.as_deref().unwrap_or_default()))
        .collect();
    let scores = bm25_okapi_scores(&corpus, &tokenize(query));
    for (result, score) in results.iter_mut().zip(scores) {
        result.bm25_score = Some(score);
    }
}

fn retain_ranked(mut results: Vec<SearchResult>) -> Vec<SearchResult> {
    results.retain(|result| result.bm25_score.is_some_and(|score| score > 0.0));
    results.sort_by(|left, right| {
        right
            .bm25_score
            .unwrap_or_default()
            .total_cmp(&left.bm25_score.unwrap_or_default())
    });
    results
}

/// Rank each originating-query group and interleave the groups fairly.
pub fn rank_results_by_query(results: Vec<SearchResult>, queries: &[String]) -> Vec<SearchResult> {
    let mut seen = HashSet::new();
    let query_order: Vec<&str> = queries
        .iter()
        .map(String::as_str)
        .filter(|query| seen.insert(*query))
        .collect();
    let mut groups: Vec<Vec<SearchResult>> = vec![Vec::new(); query_order.len()];
    let indexes: HashMap<&str, usize> = query_order
        .iter()
        .enumerate()
        .map(|(index, query)| (*query, index))
        .collect();
    let mut unassigned = Vec::new();
    for result in results {
        if let Some(index) = result
            .query
            .as_deref()
            .and_then(|query| indexes.get(query).copied())
        {
            groups[index].push(result);
        } else {
            unassigned.push(result);
        }
    }

    let mut buckets = Vec::new();
    for (query, group) in query_order.iter().zip(groups) {
        if !group.is_empty() {
            buckets.push(rank_or_retain(group, query));
        }
    }
    if !unassigned.is_empty() {
        buckets.push(rank_or_retain(unassigned, &query_order.join(" ")));
    }
    interleave(buckets)
}

/// Order search candidates by title/snippet relevance while interleaving source buckets.
///
/// This does not set the public BM25 score, which remains reserved for final
/// extracted-content ranking.
pub fn pre_rank_candidates(results: Vec<SearchResult>, queries: &[String]) -> Vec<SearchResult> {
    if results.len() < 2 {
        return results;
    }
    let mut query_order: Vec<&str> = Vec::new();
    let mut seen_queries = HashSet::new();
    for query in queries {
        if seen_queries.insert(query.as_str()) {
            query_order.push(query);
        }
    }
    let fallback_query = query_order.join(" ");
    let mut scores = vec![0.0; results.len()];
    for query in &query_order {
        score_snippet_group(&results, &mut scores, query, |result| {
            result.query.as_deref() == Some(*query)
        });
    }
    score_snippet_group(&results, &mut scores, &fallback_query, |result| {
        result
            .query
            .as_deref()
            .is_none_or(|query| !seen_queries.contains(query))
    });

    let mut bucket_indexes: HashMap<(String, Option<Engine>), usize> = HashMap::new();
    let mut buckets: Vec<Vec<(usize, f64, SearchResult)>> = Vec::new();
    for (index, (result, score)) in results.into_iter().zip(scores).enumerate() {
        let key = (result.query.clone().unwrap_or_default(), result.engine);
        let next_index = bucket_indexes.len();
        let bucket = *bucket_indexes.entry(key).or_insert_with(|| {
            buckets.push(Vec::new());
            next_index
        });
        buckets[bucket].push((index, score, result));
    }
    for bucket in &mut buckets {
        bucket.sort_by(|left, right| {
            right
                .1
                .total_cmp(&left.1)
                .then_with(|| left.0.cmp(&right.0))
        });
    }
    let ranked_buckets = buckets
        .into_iter()
        .map(|bucket| bucket.into_iter().map(|(_, _, result)| result).collect())
        .collect();
    interleave(ranked_buckets)
}

fn score_snippet_group(
    results: &[SearchResult],
    scores: &mut [f64],
    query: &str,
    includes: impl Fn(&SearchResult) -> bool,
) {
    let indexes: Vec<usize> = results
        .iter()
        .enumerate()
        .filter(|(_, result)| includes(result))
        .map(|(index, _)| index)
        .collect();
    if indexes.is_empty() {
        return;
    }
    let corpus: Vec<Vec<String>> = indexes
        .iter()
        .map(|index| {
            let result = &results[*index];
            tokenize(&format!(
                "{} {} {}",
                result.title, result.title, result.snippet
            ))
        })
        .collect();
    for (index, score) in indexes
        .into_iter()
        .zip(bm25_okapi_scores(&corpus, &tokenize(query)))
    {
        scores[index] = score;
    }
}

fn rank_or_retain(results: Vec<SearchResult>, query: &str) -> Vec<SearchResult> {
    let mut scored = results;
    score_results(&mut scored, query);
    if scored
        .iter()
        .any(|result| result.bm25_score.is_some_and(|score| score > 0.0))
    {
        retain_ranked(scored)
    } else {
        scored
    }
}

fn interleave(buckets: Vec<Vec<SearchResult>>) -> Vec<SearchResult> {
    let max_len = buckets.iter().map(Vec::len).max().unwrap_or_default();
    let mut iterators: Vec<_> = buckets.into_iter().map(Vec::into_iter).collect();
    let mut ranked = Vec::new();
    for _ in 0..max_len {
        for iterator in &mut iterators {
            if let Some(result) = iterator.next() {
                ranked.push(result);
            }
        }
    }
    ranked
}

fn bm25_okapi_scores(corpus: &[Vec<String>], query: &[String]) -> Vec<f64> {
    const K1: f64 = 1.5;
    const B: f64 = 0.75;
    const EPSILON: f64 = 0.25;

    let corpus_size = corpus.len();
    if corpus_size == 0 {
        return Vec::new();
    }
    let doc_lengths: Vec<usize> = corpus.iter().map(Vec::len).collect();
    let average_length = doc_lengths.iter().sum::<usize>() as f64 / corpus_size as f64;
    let frequencies: Vec<HashMap<&str, usize>> = corpus
        .iter()
        .map(|document| {
            let mut counts = HashMap::new();
            for token in document {
                *counts.entry(token.as_str()).or_default() += 1;
            }
            counts
        })
        .collect();
    let mut document_frequency: HashMap<&str, usize> = HashMap::new();
    for counts in &frequencies {
        for token in counts.keys() {
            *document_frequency.entry(token).or_default() += 1;
        }
    }
    if document_frequency.is_empty() {
        return vec![0.0; corpus_size];
    }

    let mut idf: HashMap<&str, f64> = document_frequency
        .iter()
        .map(|(token, frequency)| {
            let value =
                ((corpus_size as f64 - *frequency as f64 + 0.5) / (*frequency as f64 + 0.5)).ln();
            (*token, value)
        })
        .collect();
    let average_idf = idf.values().sum::<f64>() / idf.len() as f64;
    let floor = EPSILON * average_idf;
    for value in idf.values_mut() {
        if *value < 0.0 {
            *value = floor;
        }
    }

    frequencies
        .iter()
        .zip(doc_lengths)
        .map(|(counts, document_length)| {
            query
                .iter()
                .filter_map(|term| idf.get(term.as_str()).map(|idf| (term, idf)))
                .map(|(term, idf)| {
                    let frequency = counts.get(term.as_str()).copied().unwrap_or_default() as f64;
                    if frequency == 0.0 {
                        return 0.0;
                    }
                    let normalized_length = if average_length == 0.0 {
                        0.0
                    } else {
                        document_length as f64 / average_length
                    };
                    idf * (frequency * (K1 + 1.0)
                        / (frequency + K1 * (1.0 - B + B * normalized_length)))
                })
                .sum()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(title: &str, query: Option<&str>, content: Option<&str>) -> SearchResult {
        let mut result =
            SearchResult::parsed(title.into(), String::new(), String::new(), String::new());
        result.query = query.map(str::to_owned);
        result.content = content.map(str::to_owned);
        result
    }

    #[test]
    fn tokenizer_normalizes_words() {
        assert_eq!(tokenize("Hello, WORLD! 123"), ["hello", "world", "123"]);
    }

    #[test]
    fn relevant_result_wins_and_zero_scores_are_removed() {
        let ranked = rank_results(
            vec![
                result("Low", None, Some("python")),
                result("High", None, Some("python python dataclasses")),
                result("None", None, None),
                result("Other", None, Some("unrelated text")),
            ],
            "python dataclasses",
        );
        assert_eq!(
            ranked
                .iter()
                .map(|item| item.title.as_str())
                .collect::<Vec<_>>(),
            ["High"]
        );
    }

    #[test]
    fn query_groups_are_interleaved() {
        let ranked = rank_results_by_query(
            vec![
                result("Alpha best", Some("alpha"), Some("alpha alpha exact")),
                result("Alpha low", Some("alpha"), Some("alpha")),
                result("Beta best", Some("beta"), Some("beta beta exact")),
                result("Beta low", Some("beta"), Some("beta")),
            ],
            &["alpha".into(), "beta".into()],
        );
        assert_eq!(ranked[0].query.as_deref(), Some("alpha"));
        assert_eq!(ranked[1].query.as_deref(), Some("beta"));
        assert_eq!(ranked.len(), 4);
    }

    #[test]
    fn all_zero_fallback_retains_assigned_scores() {
        let ranked = rank_results_by_query(
            vec![result("Only", Some("rust"), Some("rust language"))],
            &["rust".into()],
        );
        assert_eq!(ranked.len(), 1);
        assert!(ranked[0].bm25_score.is_some());
    }

    #[test]
    fn snippet_pre_rank_reorders_within_sources_and_keeps_source_interleaving() {
        let mut alpha_low = result("Alpha low", Some("rust async"), None);
        alpha_low.engine = Some(Engine::Bing);
        alpha_low.snippet = "programming language".into();
        let mut beta = result("Async Rust", Some("rust async"), None);
        beta.engine = Some(Engine::Duckduckgo);
        beta.snippet = "async rust guide".into();
        let mut alpha_high = result("Rust async book", Some("rust async"), None);
        alpha_high.engine = Some(Engine::Bing);
        alpha_high.snippet = "rust async runtimes and futures".into();

        let ranked = pre_rank_candidates(vec![alpha_low, beta, alpha_high], &["rust async".into()]);
        assert_eq!(ranked[0].title, "Rust async book");
        assert_eq!(ranked[1].engine, Some(Engine::Duckduckgo));
        assert_eq!(ranked[2].title, "Alpha low");
        assert!(ranked.iter().all(|result| result.bm25_score.is_none()));
    }
    #[test]
    fn hybrid_retains_failed_fetches_and_ignores_source_prefix() {
        let mut relevant = result("Rust E0382 moved value", Some("E0382"), None);
        relevant.snippet = "How to fix ownership errors".into();
        let irrelevant = result(
            "Radio",
            Some("E0382"),
            Some("Source: https://example.org/E0382\n\nMusic and radio"),
        );
        let ranked = rank_with_policy(
            vec![irrelevant, relevant],
            &["E0382".into()],
            RankingPolicy::Hybrid,
        );
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].title, "Rust E0382 moved value");
        assert!(ranked.iter().all(|r| r.bm25_score.is_none()));
    }

    #[test]
    fn hybrid_single_document_and_ties_are_retained() {
        let input = vec![
            result("one", Some("absent"), None),
            result("two", Some("absent"), None),
        ];
        let ranked = rank_with_policy(input, &["absent".into()], RankingPolicy::Hybrid);
        assert_eq!(
            ranked.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(),
            ["one", "two"]
        );
        assert_eq!(
            rank_with_policy(
                vec![result("rust", Some("rust"), Some("rust"))],
                &["rust".into()],
                RankingPolicy::Hybrid
            )
            .len(),
            1
        );
    }
    #[test]
    fn experimental_tokens_preserve_versions_and_identifiers() {
        assert_eq!(
            evidence_tokens("Tempo 3.0 TRAPPIST-1 E0382 foo_bar"),
            ["tempo", "3.0", "trappist-1", "e0382", "foo_bar"]
        );
        let input = vec![
            result("Tempo 2.0 release notes", Some("Tempo 3.0"), None),
            result("Tempo 3.0 release notes", Some("Tempo 3.0"), None),
        ];
        let ranked = rank_with_policy(input, &["Tempo 3.0".into()], RankingPolicy::Snippet);
        assert_eq!(ranked[0].title, "Tempo 3.0 release notes");
    }
}
