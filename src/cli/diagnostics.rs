//! Bounded caller-facing projections of existing reports. Never serialize raw errors,
//! queries, URLs, bodies or transport records into the diagnostic extension.
use std::collections::{BTreeMap, HashMap};

use kestrelsearch::{FetchOutcome, FetchReport, PageFetchDiagnostic, SearchReport, SearchResult};
use serde_json::{Value, json};

const QUERY_LIMIT: usize = 128;
const PROVIDER_LIMIT: usize = 128;
const PAGE_LIMIT: usize = 256;

pub(super) struct SearchDiagnostics {
    search: Value,
    // Only the bounded detail rows retain identities, never page content.
    pages: Vec<(String, Value)>,
    unique: usize,
}

fn retained(outcome: &str) -> bool {
    matches!(
        outcome,
        "results"
            | "empty"
            | "filtered_empty"
            | "deadline"
            | "cancelled_min_results"
            | "cancelled_quorum"
    )
}

fn outcome(value: &str) -> &str {
    match value {
        "results"
        | "empty"
        | "filtered_empty"
        | "deadline"
        | "cancelled_min_results"
        | "cancelled_quorum"
        | "cancelled_caller"
        | "response_too_large"
        | "challenge"
        | "unrecognized"
        | "request_error" => value,
        _ => "unknown",
    }
}

impl SearchDiagnostics {
    pub(super) fn new(report: &SearchReport, queries: &[String], minimum: usize) -> Self {
        let queries = kestrelsearch::search::normalize_queries(queries);
        let indices: HashMap<_, _> = queries
            .iter()
            .enumerate()
            .map(|(i, q)| (q.as_str(), i))
            .collect();
        let mut unique = vec![0usize; queries.len()];
        for result in &report.results {
            let mut contributed: Vec<_> = result
                .sources
                .iter()
                .map(|s| s.query.as_str())
                .chain(result.query.as_deref())
                .filter_map(|q| indices.get(q).copied())
                .collect();
            contributed.sort_unstable();
            contributed.dedup();
            for index in contributed {
                unique[index] += 1;
            }
        }
        let mut occurrences = HashMap::new();
        for source in report.results.iter().flat_map(|r| &r.sources) {
            *occurrences
                .entry((source.engine, source.query.as_str()))
                .or_insert(0usize) += 1;
        }
        let mut raw = 0;
        let mut rejected = 0;
        let mut accepted_snapshot = 0;
        let mut rejected_response_snapshot = 0;
        let mut provider_rows = Vec::new();
        let mut outcome_counts = BTreeMap::<&str, usize>::new();
        let mut per_query = vec![Vec::new(); queries.len()];
        for provider in &report.providers {
            *outcome_counts
                .entry(outcome(&provider.outcome))
                .or_default() += 1;
            raw += provider.raw_result_count;
            rejected += provider.filtered_count;
            if retained(&provider.outcome) {
                accepted_snapshot += provider.result_count;
            } else {
                rejected_response_snapshot += provider.result_count;
            }
            let index = indices.get(provider.query.as_str()).copied();
            if let Some(index) = index {
                per_query[index].push(provider);
            }
            if provider_rows.len() < PROVIDER_LIMIT {
                let censored = match provider.outcome.as_str() {
                    "deadline"
                    | "cancelled_min_results"
                    | "cancelled_quorum"
                    | "cancelled_caller" => Some(true),
                    "results" | "empty" | "filtered_empty" => Some(false),
                    _ => None,
                };
                provider_rows.push(json!({
                    "query_index": index, "engine": provider.engine,
                    "outcome": outcome(&provider.outcome), "response_completed_successfully": provider.success,
                    "raw": provider.raw_result_count, "rejected": provider.filtered_count,
                    "accepted_snapshot": provider.result_count,
                    "retained_occurrences": occurrences.get(&(provider.engine, provider.query.as_str())).copied().unwrap_or(0),
                    "retries": provider.retries, "elapsed_ms": provider.elapsed_ms,
                    "timing_censored": censored,
                }));
            }
        }
        let mut query_rows = Vec::new();
        let mut minimum_count = 0;
        let mut deadline_count = 0;
        let mut exhausted_count = 0;
        let mut failed_count = 0;
        for (index, providers) in per_query.iter().enumerate() {
            let minimum_reached = unique[index] >= minimum;
            let deadline = providers.iter().any(|p| p.outcome == "deadline");
            let exhausted = !providers.is_empty()
                && providers.iter().all(|p| {
                    !matches!(
                        p.outcome.as_str(),
                        "deadline"
                            | "cancelled_min_results"
                            | "cancelled_quorum"
                            | "cancelled_caller"
                    )
                });
            let all_failed =
                unique[index] == 0 && !providers.is_empty() && providers.iter().all(|p| !p.success);
            minimum_count += usize::from(minimum_reached);
            deadline_count += usize::from(deadline);
            exhausted_count += usize::from(exhausted);
            failed_count += usize::from(all_failed);
            if query_rows.len() < QUERY_LIMIT {
                query_rows.push(
                    json!({"query_index": index, "unique_accepted": unique[index],
                    "minimum_reached": minimum_reached, "deadline": deadline,
                    "providers_exhausted": exhausted, "all_failed": all_failed}),
                );
            }
        }
        Self {
            unique: report.results.len(),
            pages: report
                .results
                .iter()
                .take(PAGE_LIMIT)
                .enumerate()
                .map(|(i, r)| (r.url.clone(), json!({"candidate_index": i})))
                .collect(),
            search: json!({
                "minimum_per_query": minimum, "query_count": queries.len(),
                "minimum_reached_queries": minimum_count, "deadline_queries": deadline_count,
                "providers_exhausted_queries": exhausted_count, "all_failed_queries": failed_count,
                "all_minimum_reached": minimum_count == queries.len(),
                "budget_exhausted": deadline_count > 0,
                "raw": raw, "rejected": rejected, "accepted_snapshot": accepted_snapshot + rejected_response_snapshot,
                "retained_occurrences": report.results.iter().map(|r| r.sources.len()).sum::<usize>(),
                "rejected_response_snapshot": rejected_response_snapshot, "unique_accepted": report.results.len(),
                "cancelled_minimum_or_quorum": report.cancelled,
                "queries": query_rows, "queries_omitted": queries.len().saturating_sub(QUERY_LIMIT),
                "provider_outcomes": outcome_counts, "providers": provider_rows, "providers_omitted": report.providers.len().saturating_sub(PROVIDER_LIMIT),
            }),
        }
    }

    pub(super) fn prepare(
        mut self,
        candidates: &[SearchResult],
        fetch: Option<&FetchReport>,
        no_fetch: bool,
        counts: &BTreeMap<String, usize>,
        byte_cap: usize,
    ) -> PreparedDiagnostics {
        let selected: HashMap<_, _> = candidates.iter().map(|r| (r.url.as_str(), r)).collect();
        let fetched: HashMap<_, _> = fetch
            .into_iter()
            .flat_map(|r| &r.pages)
            .map(|p| (p.url.as_str(), p))
            .collect();
        let mut states = BTreeMap::<String, usize>::new();
        let mut quality = BTreeMap::<String, usize>::new();
        let mut extracted = 0;
        for candidate in candidates {
            let state = evidence_state(
                no_fetch,
                Some(candidate),
                fetched.get(candidate.url.as_str()).copied(),
                fetch,
            );
            *states.entry(state.into()).or_default() += 1;
            let assessment = candidate.content_quality();
            let key = match assessment.state {
                kestrelsearch::ContentQualityState::Unknown => "unknown",
                kestrelsearch::ContentQualityState::Unflagged => "unflagged",
                kestrelsearch::ContentQualityState::BoilerplateOnly => "boilerplate_only",
            };
            *quality.entry(key.into()).or_default() += 1;
            extracted += usize::from(candidate.content.is_some());
        }
        let not_selected = self.unique.saturating_sub(candidates.len());
        if not_selected > 0 {
            states.insert("not_selected".into(), not_selected);
        }
        for (url, row) in &mut self.pages {
            let candidate = selected.get(url.as_str()).copied();
            let page = fetched.get(url.as_str()).copied();
            row["returned_index"] = Value::Null;
            row["state"] = json!(evidence_state(no_fetch, candidate, page, fetch));
            row["quality"] = json!(
                candidate
                    .map(|r| r.content_quality())
                    .unwrap_or_else(|| kestrelsearch::assess_content_quality(None))
            );
            row["timing"] = page_timing(page);
            row["byte_cap_reached"] = json!(page.map(|p| p.response_bytes >= byte_cap));
        }
        let value = json!({
            "schema_version": 1, "search": self.search,
            "candidates": {
                "unique_accepted": self.unique,
                "fetch_score_rejected": counts.get("fetch_score_rejected").copied().unwrap_or(0),
                "after_fetch_score": counts.get("after_fetch_score").copied().unwrap_or(self.unique),
                "after_selection": candidates.len(), "not_selected": not_selected,
                "after_ranking": counts.get("after_ranking").copied().unwrap_or(0), "returned": 0,
            },
            "evidence": {
                "enabled": !no_fetch, "selected": if no_fetch { 0 } else { candidates.len() },
                "scheduled": if no_fetch { 0 } else { candidates.iter().filter(|r| !r.url.to_ascii_lowercase().contains(".pdf")).count() },
                "completed": fetch.map_or(0, |r| r.pages.len()), "extracted": extracted,
                "usable": Value::Null, "usefulness_unknown": extracted,
                "quality": quality, "states": states,
                "budget_exhausted": fetch.is_some_and(|r| r.budget_exhausted),
                "cancelled": fetch.map_or(0, |r| r.cancelled), "cache_hits": fetch.map_or(0, |r| r.cache_hits),
            },
            "pages": self.pages.iter_mut().map(|(_, row)| row.take()).collect::<Vec<_>>(),
            "pages_omitted": self.unique.saturating_sub(PAGE_LIMIT),
        });
        PreparedDiagnostics {
            value,
            urls: self.pages.into_iter().map(|(url, _)| url).collect(),
        }
    }

    #[cfg(test)]
    pub(super) fn finish(
        self,
        candidates: &[SearchResult],
        results: &[SearchResult],
        fetch: Option<&FetchReport>,
        no_fetch: bool,
        counts: &BTreeMap<String, usize>,
        byte_cap: usize,
    ) -> Value {
        self.prepare(candidates, fetch, no_fetch, counts, byte_cap)
            .finish(results, counts)
    }
}

/// Body-dependent evidence is computed before ranking. Only bounded page URLs
/// survive to connect diagnostic rows to the final result positions.
pub(super) struct PreparedDiagnostics {
    value: Value,
    urls: Vec<String>,
}

impl PreparedDiagnostics {
    pub(super) fn finish(
        mut self,
        results: &[SearchResult],
        counts: &BTreeMap<String, usize>,
    ) -> Value {
        self.value["candidates"]["after_ranking"] =
            json!(counts.get("after_ranking").copied().unwrap_or(0));
        self.value["candidates"]["returned"] = json!(results.len());
        let returned: HashMap<_, _> = results
            .iter()
            .enumerate()
            .map(|(index, result)| (result.url.as_str(), index))
            .collect();
        for (index, url) in self.urls.iter().enumerate() {
            self.value["pages"][index]["returned_index"] = json!(returned.get(url.as_str()));
        }
        self.value
    }
}

fn evidence_state(
    no_fetch: bool,
    candidate: Option<&SearchResult>,
    page: Option<&PageFetchDiagnostic>,
    fetch: Option<&FetchReport>,
) -> &'static str {
    if no_fetch {
        return "no_fetch";
    }
    let Some(candidate) = candidate else {
        return "not_selected";
    };
    if candidate.url.to_ascii_lowercase().contains(".pdf") {
        return "skipped_pdf";
    }
    if let Some(page) = page {
        return fetch_state(page.outcome);
    }
    if fetch.is_some_and(|r| r.budget_exhausted) {
        "fetch_deadline"
    } else {
        "unknown"
    }
}

fn fetch_state(outcome: FetchOutcome) -> &'static str {
    match outcome {
        FetchOutcome::Success => "extracted",
        FetchOutcome::CacheHit => "cache_hit",
        FetchOutcome::NoContent => "empty_extraction",
        FetchOutcome::UnsupportedContentType => "unsupported_content_type",
        FetchOutcome::ResponseTooLarge => "response_too_large",
        FetchOutcome::RequestFailed => "request_failed",
    }
}

fn page_timing(page: Option<&PageFetchDiagnostic>) -> Value {
    page.map_or(Value::Null, |p| {
        json!({"total_ms": p.total_ms,
        // Existing fetch reports cannot identify a timeout's interrupted phase.
        "censored": if p.outcome == FetchOutcome::RequestFailed { None } else { Some(false) }})
    })
}

pub(super) fn direct_fetch(report: &FetchReport, byte_cap: usize) -> Value {
    let page = report.pages.first();
    json!({"schema_version": 1,
        "state": page.map(|p| fetch_state(p.outcome)).unwrap_or("unknown"),
        "quality": report.content_quality(0), "usable": Value::Null,
        "timing": page_timing(page),
        "byte_cap_reached": page.map(|p| p.response_bytes >= byte_cap),
        "budget_exhausted": report.budget_exhausted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kestrelsearch::{Engine, ProviderSearchDiagnostic};

    fn candidate(url: &str, queries: &[&str]) -> SearchResult {
        serde_json::from_value(json!({"title": "private title", "url": url,
            "display_url": url, "snippet": "private snippet", "content": null,
            "sources": queries.iter().map(|q| json!({"engine":"bing", "query":q, "rank":1})).collect::<Vec<_>>()
        })).unwrap()
    }

    fn provider(
        query: &str,
        outcome: &str,
        raw: usize,
        rejected: usize,
    ) -> ProviderSearchDiagnostic {
        ProviderSearchDiagnostic {
            engine: Engine::Bing,
            query: query.into(),
            outcome: outcome.into(),
            success: matches!(outcome, "results" | "empty" | "filtered_empty"),
            raw_result_count: raw,
            filtered_count: rejected,
            result_count: raw - rejected,
            elapsed_ms: 20,
            retries: 0,
            error: Some("private transport credential".into()),
        }
    }

    fn report(
        results: Vec<SearchResult>,
        providers: Vec<ProviderSearchDiagnostic>,
    ) -> SearchReport {
        SearchReport {
            results,
            providers,
            cancelled: 0,
        }
    }

    #[test]
    fn prepared_evidence_survives_body_drop_and_result_reordering() {
        let mut original = report(
            vec![
                candidate("https://a.test", &["q"]),
                candidate("https://b.test", &["q"]),
            ],
            vec![],
        );
        original.results[0].content = Some("Useful original page evidence.".repeat(1000));
        original.results[1].content = Some("Second original page evidence.".into());
        let counts = BTreeMap::from([("after_ranking".into(), 2)]);
        let prepared = SearchDiagnostics::new(&original, &["q".into()], 1).prepare(
            &original.results,
            None,
            false,
            &counts,
            1_000_000,
        );
        let mut returned = original.results.pop().unwrap();
        drop(original);
        returned.content = None;
        let value = prepared.finish(&[returned], &counts);
        assert_eq!(value["evidence"]["extracted"], 2);
        assert_eq!(value["candidates"]["after_ranking"], 2);
        assert_eq!(value["candidates"]["returned"], 1);
        assert_eq!(value["pages"][0]["returned_index"], Value::Null);
        assert_eq!(value["pages"][1]["returned_index"], 0);
        assert!(!value.to_string().contains("original page"));
    }

    #[test]
    fn structured_completion_preserves_empty_filtered_failed_and_simultaneous_conditions() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        let queries: Vec<_> = [
            " first ", "first", "", "filtered", "empty", "failed", "deadline",
        ]
        .map(String::from)
        .to_vec();
        let report = report(
            vec![candidate(
                "https://private.test",
                &["first", "first", "deadline"],
            )],
            vec![
                provider("first", "results", 2, 0),
                provider("filtered", "filtered_empty", 3, 3),
                provider("empty", "empty", 0, 0),
                provider("failed", "request_error", 2, 0),
                provider("deadline", "deadline", 1, 0),
            ],
        );
        let value = SearchDiagnostics::new(&report, &queries, 1).finish(
            &report.results,
            &report.results,
            None,
            true,
            &BTreeMap::from([("after_ranking".into(), 1)]),
            100,
        );
        let search = &value["search"];
        assert_eq!(search["query_count"], 5);
        assert_eq!(search["unique_accepted"], 1);
        assert_eq!(search["minimum_reached_queries"], 2);
        assert_eq!(search["providers_exhausted_queries"], 4);
        assert_eq!(search["all_failed_queries"], 1);
        assert_eq!(search["raw"], 8);
        assert_eq!(search["rejected"], 3);
        assert_eq!(search["accepted_snapshot"], 5);
        assert_eq!(search["rejected_response_snapshot"], 2);
        assert_eq!(search["retained_occurrences"], 3);
        assert_eq!(search["queries"][0]["unique_accepted"], 1);
        assert_eq!(search["queries"][0]["minimum_reached"], true);
        assert_eq!(search["queries"][0]["providers_exhausted"], true);
        assert_eq!(search["queries"][1]["all_failed"], false);
        assert_eq!(search["queries"][2]["all_failed"], false);
        assert_eq!(search["queries"][4]["minimum_reached"], true);
        assert_eq!(search["queries"][4]["deadline"], true);
        assert_eq!(search["providers"][4]["retained_occurrences"], 1);
        assert_eq!(search["providers"][4]["timing_censored"], true);
        assert!(search["providers"][3]["timing_censored"].is_null());
        assert_eq!(value["evidence"]["states"]["no_fetch"], 1);
        assert_eq!(value["evidence"]["selected"], 0);
        assert_eq!(value["pages"][0]["returned_index"], 0);
        assert!(value["pages"][0]["timing"].is_null());
        let serialized = value.to_string();
        for private in ["private", "first", "filtered\"", "https://"] {
            assert!(!serialized.contains(private), "{private}");
        }
    }

    #[test]
    fn structured_cancelled_snapshot_is_not_a_failed_candidate_or_a_completed_latency() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        for reason in ["cancelled_min_results", "deadline"] {
            let report = report(
                vec![candidate("https://example.test", &["q"])],
                vec![provider("q", reason, 3, 1)],
            );
            let value = SearchDiagnostics::new(&report, &["q".into()], 1).search;
            assert_eq!(value["queries"][0]["all_failed"], false);
            assert_eq!(value["queries"][0]["providers_exhausted"], false);
            assert_eq!(
                value["providers"][0]["response_completed_successfully"],
                false
            );
            assert_eq!(value["providers"][0]["accepted_snapshot"], 2);
            assert_eq!(value["providers"][0]["retained_occurrences"], 1);
            assert_eq!(value["providers"][0]["outcome"], reason);
        }
    }

    #[test]
    fn structured_empty_and_all_gated_out_stages_reconcile() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        for count in [0, 3] {
            let results = (0..count)
                .map(|i| candidate(&format!("https://example.test/{i}"), &["q"]))
                .collect();
            let report = report(
                results,
                vec![provider(
                    "q",
                    if count == 0 { "empty" } else { "results" },
                    count,
                    0,
                )],
            );
            let counts = BTreeMap::from([
                ("fetch_score_rejected".into(), count),
                ("after_fetch_score".into(), 0),
            ]);
            let value = SearchDiagnostics::new(&report, &["q".into()], 5).finish(
                &[],
                &[],
                None,
                false,
                &counts,
                100,
            );
            assert_eq!(value["candidates"]["unique_accepted"], count);
            assert_eq!(value["candidates"]["not_selected"], count);
            assert_eq!(value["candidates"]["after_fetch_score"], 0);
            assert_eq!(value["candidates"]["returned"], 0);
            assert_eq!(value["evidence"]["completed"], 0);
            assert_eq!(value["search"]["all_failed_queries"], 0);
            if count > 0 {
                assert_eq!(value["pages"][0]["state"], "not_selected");
            }
        }
    }

    #[test]
    fn structured_detail_bounds_do_not_truncate_aggregates() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        let queries: Vec<_> = (0..140).map(|i| format!("private query {i}")).collect();
        let report = report(
            (0..300)
                .map(|i| candidate(&format!("https://private.test/{i}"), &[&queries[0]]))
                .collect(),
            queries.iter().map(|q| provider(q, "empty", 0, 0)).collect(),
        );
        let value = SearchDiagnostics::new(&report, &queries, 5).finish(
            &report.results,
            &[],
            None,
            true,
            &BTreeMap::new(),
            100,
        );
        assert_eq!(value["search"]["query_count"], 140);
        assert_eq!(value["search"]["providers_exhausted_queries"], 140);
        assert_eq!(value["search"]["provider_outcomes"]["empty"], 140);
        assert_eq!(value["search"]["queries"].as_array().unwrap().len(), 128);
        assert_eq!(value["search"]["providers"].as_array().unwrap().len(), 128);
        assert_eq!(value["search"]["queries_omitted"], 12);
        assert_eq!(value["search"]["providers_omitted"], 12);
        assert_eq!(value["pages"].as_array().unwrap().len(), 256);
        assert_eq!(value["pages_omitted"], 44);
        assert_eq!(value["evidence"]["states"]["no_fetch"], 300);
        assert!(!value.to_string().contains("private"));
    }

    #[test]
    fn structured_fetch_joins_failures_quality_cache_and_deadline_by_identity() {
        let _telemetry = kestrelsearch::telemetry::test_export_guard();
        // Report projection needs no transport. These sanitized fixtures also
        // make cache reordering and cancellation independent of proxy settings.
        let options = kestrelsearch::FetchOptions::default();
        let urls: Vec<_> = ["/shell", "/article", "/empty", "/failure"]
            .map(|path| format!("https://example.test{path}"))
            .to_vec();
        let original = report(
            urls.iter()
                .chain([
                    &"https://example.test/skipped.pdf".to_owned(),
                    &"https://example.test/unselected".to_owned(),
                ])
                .map(|url| candidate(url, &["q"]))
                .collect(),
            vec![provider("q", "results", 6, 0)],
        );
        for cache_hits in [0, 2] {
            let body_outcome = if cache_hits == 0 {
                FetchOutcome::Success
            } else {
                FetchOutcome::CacheHit
            };
            let outcomes = [
                body_outcome,
                body_outcome,
                FetchOutcome::NoContent,
                FetchOutcome::RequestFailed,
            ];
            let mut fetch = FetchReport {
                contents: vec![
                    Some("Your browser is not supported.".into()),
                    Some("A concise useful-looking answer.".into()),
                    None,
                    None,
                ],
                // Deliberately differ from input order, as cached reports can.
                pages: [2, 0, 3, 1]
                    .into_iter()
                    .map(|index| PageFetchDiagnostic {
                        url: urls[index].clone(),
                        outcome: outcomes[index],
                        queue_ms: 0,
                        request_ms: 0,
                        download_ms: 0,
                        parse_queue_ms: 0,
                        parse_ms: 0,
                        total_ms: 0,
                        response_bytes: 0,
                        http_version: None,
                    })
                    .collect(),
                budget_exhausted: false,
                cancelled: 0,
                cache_hits,
                cache_misses: 4 - cache_hits,
            };
            let mut candidates = original.results[..5].to_vec();
            // The CLI moves bodies out of FetchReport before projecting diagnostics.
            for (result, content) in candidates.iter_mut().zip(&mut fetch.contents) {
                result.content = content.take();
            }
            let returned = vec![candidates[1].clone()];
            let value = SearchDiagnostics::new(&original, &["q".into()], 5).finish(
                &candidates,
                &returned,
                Some(&fetch),
                false,
                &BTreeMap::from([("after_ranking".into(), 3)]),
                options.max_response_bytes,
            );
            assert_eq!(value["evidence"]["selected"], 5);
            assert_eq!(value["evidence"]["scheduled"], 4);
            assert_eq!(value["evidence"]["completed"], 4);
            assert_eq!(value["evidence"]["extracted"], 2);
            assert_eq!(value["evidence"]["quality"]["boilerplate_only"], 1);
            assert_eq!(value["evidence"]["quality"]["unflagged"], 1);
            assert_eq!(value["evidence"]["quality"]["unknown"], 3);
            assert!(value["evidence"]["usable"].is_null());
            assert_eq!(value["pages"][1]["returned_index"], 0);
            assert_eq!(value["pages"][2]["state"], "empty_extraction");
            assert_eq!(value["pages"][3]["state"], "request_failed");
            assert!(value["pages"][3]["timing"]["censored"].is_null());
            assert_eq!(value["pages"][4]["state"], "skipped_pdf");
            assert_eq!(value["pages"][5]["state"], "not_selected");
            assert_eq!(
                value["pages"][0]["state"],
                if cache_hits == 0 {
                    "extracted"
                } else {
                    "cache_hit"
                }
            );
        }
        let fetch = FetchReport {
            contents: vec![None; 4],
            pages: vec![],
            budget_exhausted: true,
            cancelled: 4,
            cache_hits: 0,
            cache_misses: 4,
        };
        let value = SearchDiagnostics::new(&original, &["q".into()], 5).finish(
            &original.results[..4],
            &[],
            Some(&fetch),
            false,
            &BTreeMap::new(),
            options.max_response_bytes,
        );
        assert_eq!(value["evidence"]["cancelled"], 4);
        assert_eq!(value["evidence"]["states"]["fetch_deadline"], 4);
        assert!(value["pages"][0]["timing"].is_null());
    }
}
