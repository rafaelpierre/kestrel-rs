//! Optional retrieval-artifact capture used by the benchmark harness.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::model::{
    Engine, FetchOutcome, FetchReport, ProviderSearchDiagnostic, SearchMode, SearchResult,
};

/// Optional fine-grained measurements attached to a benchmark artifact.
#[derive(Clone, Copy, Debug, Default)]
pub struct ArtifactDiagnostics<'a> {
    pub providers: &'a [ProviderSearchDiagnostic],
    pub provider_cancellations: usize,
    pub fetch: Option<&'a FetchReport>,
}

/// Write an artifact only when both benchmark environment variables are present.
pub fn write_artifact(
    query: &str,
    results: &[SearchResult],
    timings_ms: &BTreeMap<String, u64>,
    queries: &[String],
    engines: &[Engine],
    mode: SearchMode,
    diagnostics: ArtifactDiagnostics<'_>,
) -> io::Result<Option<PathBuf>> {
    let (Ok(directory), Ok(run_id)) = (
        std::env::var("KESTRELSEARCH_BENCHMARK_ARTIFACT_DIR"),
        std::env::var("KESTRELSEARCH_BENCHMARK_RUN_ID"),
    ) else {
        return Ok(None);
    };
    write_artifact_to(
        Path::new(&directory),
        &run_id,
        query,
        results,
        timings_ms,
        queries,
        engines,
        mode,
        diagnostics.providers,
        diagnostics.provider_cancellations,
        diagnostics.fetch,
    )
    .map(Some)
}

#[allow(clippy::too_many_arguments)]
fn write_artifact_to(
    directory: &Path,
    run_id: &str,
    query: &str,
    results: &[SearchResult],
    timings_ms: &BTreeMap<String, u64>,
    queries: &[String],
    engines: &[Engine],
    mode: SearchMode,
    providers: &[ProviderSearchDiagnostic],
    provider_cancellations: usize,
    fetch: Option<&FetchReport>,
) -> io::Result<PathBuf> {
    let rendered_results: Vec<Value> = results
        .iter()
        .enumerate()
        .map(|(index, result)| {
            let content = result.content.as_deref().unwrap_or_default();
            json!({
                "rank": index + 1,
                "url": result.url,
                "title": result.title,
                "snippet": result.snippet,
                "bm25_score": result.bm25_score,
                "content": result.content,
                "content_chars": content.chars().count(),
                "content_sha256": format!("{:x}", Sha256::digest(content.as_bytes())),
                "engine": result.engine,
                "query": result.query,
                "engine_rank": result.engine_rank,
                "sources": result.sources,
            })
        })
        .collect();
    let fetch_diagnostics = fetch.map(|report| {
        let count = |outcome| {
            report
                .pages
                .iter()
                .filter(|page| page.outcome == outcome)
                .count()
        };
        json!({
            "budget_exhausted": report.budget_exhausted,
            "cancelled": report.cancelled,
            "cache_hits": report.cache_hits,
            "cache_misses": report.cache_misses,
            "response_bytes": report.pages.iter().map(|page| page.response_bytes).sum::<usize>(),
            "outcomes": {
                "success": count(FetchOutcome::Success),
                "no_content": count(FetchOutcome::NoContent),
                "unsupported_content_type": count(FetchOutcome::UnsupportedContentType),
                "response_too_large": count(FetchOutcome::ResponseTooLarge),
                "request_failed": count(FetchOutcome::RequestFailed),
                "cache_hit": count(FetchOutcome::CacheHit),
            },
            "pages": report.pages,
        })
    });
    let artifact = json!({
        "run_id": run_id,
        "query": query,
        "queries": queries,
        "engines": engines,
        "mode": mode,
        "results": rendered_results,
        "returned_chars": results.iter().map(|result| result.content.as_deref().unwrap_or_default().chars().count()).sum::<usize>(),
        "timings_ms": timings_ms,
        "diagnostics": {
            "providers": providers,
            "provider_cancellations": provider_cancellations,
            "fetch": fetch_diagnostics,
        },
    });
    fs::create_dir_all(directory)?;
    let target = directory.join(format!("{run_id}-{}.json", uuid::Uuid::new_v4().simple()));
    fs::write(&target, serde_json::to_vec_pretty(&artifact)?)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_compact_result_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let mut result = SearchResult::parsed(
            "Example".into(),
            "https://example.test".into(),
            String::new(),
            "Example snippet".into(),
        );
        result.content = Some("Some page text".into());
        result.bm25_score = Some(1.5);
        let target = write_artifact_to(
            directory.path(),
            "run-123",
            "example query",
            &[result],
            &BTreeMap::from([("search".into(), 12)]),
            &["example query".into()],
            &[],
            SearchMode::Fallback,
            &[],
            0,
            None,
        )
        .unwrap();
        let artifact: Value = serde_json::from_slice(&fs::read(target).unwrap()).unwrap();
        assert_eq!(artifact["returned_chars"], "Some page text".chars().count());
        assert_eq!(artifact["results"][0]["content"], "Some page text");
        assert!(
            artifact["results"][0]["content_sha256"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
        assert_eq!(artifact["diagnostics"]["providers"], json!([]));
        assert_eq!(artifact["diagnostics"]["provider_cancellations"], 0);
    }
}
