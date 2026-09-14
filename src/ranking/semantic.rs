//! Backend-independent, opt-in building blocks; no production backend or CLI mode.
use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::model::SearchResult;

/// Immutable identity. Revisions must identify exact model weights and preprocessing.
#[derive(Clone, Debug, Serialize)]
pub struct ScorerIdentity {
    pub backend: String,
    pub model_revision: String,
    /// Includes text selection, truncation, chunking and score interpretation.
    pub configuration: String,
}

/// A caller-assigned unique batch ID, independent of backend output order.
#[derive(Clone, Debug, Serialize)]
pub struct ScoringInput<'a> {
    pub id: usize,
    pub query: &'a str,
    pub text: &'a str,
}

/// None means unavailable evidence, not a zero similarity. Higher is better.
#[derive(Clone, Debug)]
pub struct ScoringOutput {
    pub id: usize,
    pub score: Option<f64>,
}

/// Explicit byte limits apply before any backend invocation; text is never silently cut.
#[derive(Clone, Copy, Debug)]
pub struct ScoringLimits {
    pub max_pairs: usize,
    pub max_query_bytes: usize,
    pub max_text_bytes: usize,
    pub max_total_bytes: usize,
    pub timeout: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ScoringError {
    #[error("semantic backend unavailable: {0}")]
    Unavailable(String),
    #[error("semantic input exceeds limits or contains duplicate IDs")]
    InvalidInput,
    #[error("semantic output has missing, duplicate, unknown IDs or nonfinite scores")]
    InvalidOutput,
    #[error("semantic scoring deadline elapsed")]
    Timeout,
    #[error("semantic backend failed: {0}")]
    Backend(String),
}

/// Implementations must yield, avoid blocking the executor, and cancel owned work
/// when their future is dropped. Detached inference is not permitted by this contract.
#[async_trait]
pub trait SemanticScorer: Send + Sync {
    fn identity(&self) -> ScorerIdentity;
    async fn availability(&self) -> Result<(), ScoringError>;
    async fn score(&self, inputs: &[ScoringInput<'_>]) -> Result<Vec<ScoringOutput>, ScoringError>;
}

/// Hash exact query/text bytes plus backend/model/configuration; excludes transient IDs.
/// Callers must not cache failures or share entries between different identities.
pub fn cache_key(identity: &ScorerIdentity, input: &ScoringInput<'_>) -> String {
    // All serialized fields are strings; serialization to a byte vector is infallible.
    let bytes = serde_json::to_vec(&(1u8, identity, input.query, input.text))
        .expect("string-only cache identity serializes");
    format!("{:x}", Sha256::digest(bytes))
}

/// Validate and score one bounded batch. Returns scores in input order, regardless
/// of backend order. Explicit None is required for unscored pairs. A malformed
/// batch fails atomically; fallback policy belongs to the caller. Dropping this
/// future cancels availability/inference; the deadline covers both.
pub async fn score_batch(
    scorer: &dyn SemanticScorer,
    inputs: &[ScoringInput<'_>],
    limits: ScoringLimits,
) -> Result<Vec<ScoringOutput>, ScoringError> {
    let mut ids = HashSet::new();
    let mut total = 0usize;
    if limits.timeout.is_zero() || inputs.len() > limits.max_pairs {
        return Err(ScoringError::InvalidInput);
    }
    for input in inputs {
        total = total
            .checked_add(input.query.len())
            .and_then(|v| v.checked_add(input.text.len()))
            .ok_or(ScoringError::InvalidInput)?;
        if !ids.insert(input.id)
            || input.query.len() > limits.max_query_bytes
            || input.text.len() > limits.max_text_bytes
            || total > limits.max_total_bytes
        {
            return Err(ScoringError::InvalidInput);
        }
    }
    if inputs.is_empty() {
        return Ok(Vec::new());
    }
    tokio::time::timeout(limits.timeout, async {
        scorer.availability().await?;
        let output = scorer.score(inputs).await?;
        let mut mapped = BTreeMap::new();
        for row in output {
            if !ids.contains(&row.id)
                || row.score.is_some_and(|v| !v.is_finite())
                || mapped.insert(row.id, row.score).is_some()
            {
                return Err(ScoringError::InvalidOutput);
            }
        }
        if mapped.len() != inputs.len() {
            return Err(ScoringError::InvalidOutput);
        }
        Ok(inputs
            .iter()
            .map(|input| ScoringOutput {
                id: input.id,
                score: mapped[&input.id],
            })
            .collect())
    })
    .await
    .map_err(|_| ScoringError::Timeout)?
}

/// Scores for a single query/candidate pair. Query indices refer to the caller's
/// ordered query list (including repeated queries). Scores never cross queries.
#[derive(Clone, Debug)]
pub struct RelevanceEvidence {
    pub query: usize,
    pub candidate: usize,
    pub lexical: Option<f64>,
    pub semantic: Option<f64>,
}

/// Separate diagnostics: the combined score is equal-weight RRF, not BM25.
#[derive(Clone, Debug)]
pub struct CombinedRank {
    pub query: usize,
    pub url: String,
    pub lexical_rank: Option<usize>,
    pub semantic_rank: Option<usize>,
    pub score: f64,
}

#[derive(Clone, Debug)]
pub struct Combination {
    pub results: Vec<SearchResult>,
    pub ranks: Vec<CombinedRank>,
}

/// Combine already-selected canonical candidates per query with sum(1/(60+rank)).
/// Finite scores (including zero/negative) participate; missing/nonfinite scores
/// contribute nothing. Duplicate URL/pair evidence uses the best component score.
/// Equal component scores use canonical URL order and ordinal one-based ranks.
/// Final ties use canonical URL; query lists are fairly interleaved, emitting each
/// URL once. All candidates, including those without evidence, survive. The first
/// URL representative retains metadata/BM25; all distinct provenance is merged.
pub fn combine(
    candidates: Vec<SearchResult>,
    query_count: usize,
    evidence: &[RelevanceEvidence],
) -> Result<Combination, ScoringError> {
    if evidence
        .iter()
        .any(|e| e.query >= query_count || e.candidate >= candidates.len())
    {
        return Err(ScoringError::InvalidInput);
    }
    let urls: Vec<_> = candidates
        .iter()
        .map(|r| crate::search::canonical_url(&r.url))
        .collect();
    if urls.iter().any(String::is_empty) {
        return Err(ScoringError::InvalidInput);
    }
    let mut unique: BTreeMap<String, SearchResult> = BTreeMap::new();
    for (result, url) in candidates.into_iter().zip(&urls) {
        if let Some(existing) = unique.get_mut(url) {
            for source in result.sources {
                if !existing.sources.contains(&source) {
                    existing.sources.push(source);
                }
            }
        } else {
            unique.insert(url.clone(), result);
        }
    }
    let mut groups = Vec::new();
    let mut ranks = Vec::new();
    for query in 0..query_count {
        let mut scores: BTreeMap<String, [Option<f64>; 2]> = BTreeMap::new();
        for e in evidence.iter().filter(|e| e.query == query) {
            let row = scores
                .entry(urls[e.candidate].clone())
                .or_insert([None, None]);
            for (slot, value) in row.iter_mut().zip([e.lexical, e.semantic]) {
                if let Some(value) = value.filter(|v| v.is_finite()) {
                    *slot = Some(slot.map_or(value, |old| old.max(value)));
                }
            }
        }
        let mut component_ranks: BTreeMap<String, [Option<usize>; 2]> = scores
            .keys()
            .map(|url| (url.clone(), [None, None]))
            .collect();
        for component in 0..2 {
            let mut ordered: Vec<_> = scores
                .iter()
                .filter_map(|(url, s)| s[component].map(|s| (url, s)))
                .collect();
            ordered.sort_by(|a, b| {
                b.1.partial_cmp(&a.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.0.cmp(b.0))
            });
            for (index, (url, _)) in ordered.into_iter().enumerate() {
                if let Some(row) = component_ranks.get_mut(url) {
                    row[component] = Some(index + 1);
                }
            }
        }
        let mut group: Vec<_> = component_ranks
            .into_iter()
            .map(|(url, r)| CombinedRank {
                query,
                url,
                lexical_rank: r[0],
                semantic_rank: r[1],
                score: r
                    .into_iter()
                    .flatten()
                    .map(|r| 1.0 / (60.0 + r as f64))
                    .sum(),
            })
            .collect();
        group.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.url.cmp(&b.url)));
        groups.push(group.iter().map(|r| r.url.clone()).collect::<Vec<_>>());
        ranks.extend(group);
    }
    let mut results = Vec::new();
    for index in 0..groups.iter().map(Vec::len).max().unwrap_or(0) {
        for group in &groups {
            if let Some(url) = group.get(index)
                && let Some(result) = unique.remove(url)
            {
                results.push(result);
            }
        }
    }
    results.extend(unique.into_values());
    Ok(Combination { results, ranks })
}

#[cfg(test)]
mod tests;
