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
    pub candidates: &'a [SearchResult],
    pub candidate_counts: Option<&'a BTreeMap<String, usize>>,
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
        diagnostics.candidates,
        diagnostics.candidate_counts,
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
    candidates: &[SearchResult],
    candidate_counts: Option<&BTreeMap<String, usize>>,
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
                "content_quality": result.content_quality(),
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
        "candidates": candidates,
        "candidate_counts": candidate_counts,
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
            "candidate_content_quality": candidates.iter().map(SearchResult::content_quality).collect::<Vec<_>>(),
            "fetch": fetch_diagnostics,
        },
    });
    fs::create_dir_all(directory)?;
    let target = directory.join(format!("{run_id}-{}.json", uuid::Uuid::new_v4().simple()));
    fs::write(&target, serde_json::to_vec_pretty(&artifact)?)?;
    Ok(target)
}

#[cfg(test)]
tokio::task_local! {
    pub(crate) static TEST_TRACE_DIRECTORY: Option<PathBuf>;
}

fn trace_directory() -> Option<PathBuf> {
    #[cfg(test)]
    if let Ok(directory) = TEST_TRACE_DIRECTORY.try_with(Clone::clone) {
        return directory;
    }
    std::env::var_os("KESTRELSEARCH_PROVIDER_TRACE_DIR").map(PathBuf::from)
}

/// Opt-in raw provider capture; normal output never includes response HTML.
#[allow(clippy::too_many_arguments)]
pub(crate) fn capture_provider(
    engine: Engine,
    query: &str,
    url: &str,
    status: u16,
    http_version: &str,
    attempt: usize,
    html: &str,
) {
    let Some(directory) = trace_directory() else {
        return;
    };
    let id = format!("{}-{}", engine, uuid::Uuid::new_v4().simple());
    crate::diagnostic_sink::submit(|record| {
        record.file(directory.join(format!("{id}.html")), false, |writer| {
            writer.write_all(html.as_bytes())
        })?;
        record.file(directory.join(format!("{id}.json")), false, |writer| {
            serde_json::to_writer_pretty(writer, &json!({
                "engine": engine, "query": query, "final_url": url, "http_status": status, "http_version": http_version,
                "attempt": attempt, "html_file": format!("{id}.html"),
                "captured_at": chrono::Utc::now().to_rfc3339(),
                "correlation": crate::search::current_correlation(),
            })).map_err(io::Error::other)
        })
    });
}

/// Record only generated browser headers, never credentials or response cookies.
pub(crate) fn capture_headers(client: &str, headers: &reqwest::header::HeaderMap) {
    let Some(directory) = trace_directory() else {
        return;
    };
    let values: BTreeMap<_, _> = headers
        .iter()
        .filter_map(|(name, value)| value.to_str().ok().map(|value| (name.as_str(), value)))
        .collect();
    crate::diagnostic_sink::submit(|record| {
        record.file(
            directory.join(format!(
                "headers-{client}-{}.json",
                uuid::Uuid::new_v4().simple()
            )),
            false,
            |writer| {
                serde_json::to_writer_pretty(writer, &json!({"client": client, "headers": values}))
                    .map_err(io::Error::other)
            },
        )
    });
}

/// Preserve diagnostics even when all providers fail and no SearchReport is returned.
pub(crate) fn capture_provider_lifecycle(
    diagnostic: &ProviderSearchDiagnostic,
    lifecycle: Option<&crate::provider_diagnostics::Lifecycle>,
) {
    let Some(directory) = trace_directory() else {
        return;
    };
    crate::diagnostic_sink::submit(|record| {
        record.file(
            directory.join(format!(
                "outcome-{}-{}.json",
                diagnostic.engine,
                uuid::Uuid::new_v4().simple()
            )),
            false,
            |writer| {
                let mut value = serde_json::to_value(diagnostic)?;
                if let Some(lifecycle) = lifecycle {
                    value["lifecycle"] = serde_json::to_value(lifecycle)?;
                }
                serde_json::to_writer_pretty(writer, &value).map_err(io::Error::other)
            },
        )
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_compact_result_metadata() {
        let _telemetry = crate::telemetry::test_export_guard();
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
            std::slice::from_ref(&result),
            &BTreeMap::from([("search".into(), 12)]),
            &["example query".into()],
            &[],
            SearchMode::Fanout,
            &[],
            0,
            None,
            &[
                SearchResult::parsed(
                    "Not fetched".into(),
                    "https://example.test/missing".into(),
                    String::new(),
                    String::new(),
                ),
                result.clone(),
            ],
            None,
        )
        .unwrap();
        let artifact: Value = serde_json::from_slice(&fs::read(target).unwrap()).unwrap();
        assert_eq!(
            artifact["results"][0]["content_quality"],
            json!({
                "version": 1, "state": "unflagged", "reasons": ["no_known_shell"]
            })
        );
        assert_eq!(
            artifact["diagnostics"]["candidate_content_quality"],
            json!([
                {"version": 1, "state": "unknown", "reasons": ["no_text"]},
                {"version": 1, "state": "unflagged", "reasons": ["no_known_shell"]}
            ])
        );
        assert_eq!(artifact["mode"], "fanout");
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
