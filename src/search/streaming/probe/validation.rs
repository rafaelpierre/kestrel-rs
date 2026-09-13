//! Current-contract experiments. This entire module and its collector hook are test-only.
use super::*;
use serde::Deserialize;
use std::io::Write;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum Policy {
    Full,
    Batch,
    Stream,
    Diversity,
}
const POLICIES: [Policy; 4] = [
    Policy::Full,
    Policy::Batch,
    Policy::Stream,
    Policy::Diversity,
];

#[derive(Default, Serialize)]
struct CollectorObservation {
    first_five_unique_ms: Option<u64>,
    threshold_observed_ms: Option<u64>,
}
tokio::task_local! {
    static POLICY: Policy;
    static COLLECTOR: Arc<Mutex<CollectorObservation>>;
}

/// Count the collector's filtered canonical candidates, including streamed snapshots.
/// Full fanout still publishes snapshots so time-to-five is measured equivalently.
pub(in crate::search::streaming) fn observe_collector(
    buckets: &[&Vec<SearchResult>],
    production_reached: bool,
) -> bool {
    POLICY
        .try_with(|policy| {
            let unique = buckets
                .iter()
                .flat_map(|r| r.iter().map(result_key))
                .collect::<HashSet<_>>()
                .len();
            let contributors = buckets.iter().filter(|r| !r.is_empty()).count();
            let reached = match policy {
                Policy::Full => false,
                Policy::Batch | Policy::Stream => production_reached,
                Policy::Diversity => unique >= 5 && contributors >= 2,
            };
            let _ = COLLECTOR.try_with(|observation| {
                let _ = PROBE.try_with(|probe| {
                    let elapsed = elapsed_millis(probe.lock().unwrap().started);
                    let mut observation = observation.lock().unwrap();
                    if unique >= 5 {
                        observation.first_five_unique_ms.get_or_insert(elapsed);
                    }
                    if reached {
                        observation.threshold_observed_ms.get_or_insert(elapsed);
                    }
                });
            });
            reached
        })
        .unwrap_or(production_reached)
}

#[derive(Deserialize, Serialize)]
struct Corpus {
    version: u32,
    queries: Vec<Query>,
}
#[derive(Deserialize, Serialize)]
struct Query {
    id: String,
    category: String,
    query: String,
    intent: String,
}
fn command(program: &str, args: &[&str]) -> String {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "{program} failed");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
fn positive_env(name: &str, default: usize) -> usize {
    let value = std::env::var(name)
        .map(|s| s.parse().expect("positive integer"))
        .unwrap_or(default);
    assert!(value > 0, "{name} must be positive");
    value
}

// Keep diagnostics even when merging all failed providers returns an error.
// Uses the same validation, provider jobs, collector and merger as the public API.
async fn measured_search(
    query: &str,
    options: &SearchOptions,
    clients: &SearchClients,
) -> (
    Result<Vec<SearchResult>, KestrelError>,
    Vec<ProviderSearchDiagnostic>,
    usize,
) {
    let (queries, engines) = validate_request(&[query.to_owned()], options).unwrap();
    let diagnostics = Arc::new(Mutex::new(Vec::new()));
    let deadline = options
        .search_budget
        .map(|budget| crate::numeric::deadline("search budget", budget).unwrap());
    let (outcomes, cancelled) = DIAGNOSTIC_RUN_ID
        .scope(
            uuid::Uuid::new_v4().to_string(),
            run_fanout_query(
                &queries[0],
                &engines,
                clients,
                Arc::new(Semaphore::new(options.max_concurrency)),
                diagnostics.clone(),
                &options.region,
                options.time_filter,
                options.query_syntax,
                options.provider_quorum,
                Some(options.min_results.unwrap_or(5)),
                deadline,
            ),
        )
        .await;
    let diagnostics = Arc::try_unwrap(diagnostics).unwrap().into_inner().unwrap();
    (merge_outcomes(outcomes), diagnostics, cancelled)
}

#[tokio::test]
#[ignore = "live providers: current-contract four-policy benchmark; explicit output directory required"]
async fn live_streaming_validation() {
    let directory = std::path::PathBuf::from(
        std::env::var("KESTREL_VALIDATION_OUTPUT")
            .expect("set KESTREL_VALIDATION_OUTPUT to a new directory"),
    );
    let trials = positive_env("KESTREL_VALIDATION_TRIALS", 3);
    let budget_seconds = positive_env("KESTREL_VALIDATION_BUDGET", 3);
    let query_limit = positive_env("KESTREL_VALIDATION_QUERIES", 8);
    let context = std::env::var("KESTREL_VALIDATION_CONTEXT")
        .expect("set KESTREL_VALIDATION_CONTEXT to a non-sensitive region/network description");
    assert!(!context.trim().is_empty());
    let corpus: Corpus = serde_json::from_str(include_str!(
        "../../../../benchmarks/streaming-queries-v1.json"
    ))
    .unwrap();
    assert!(query_limit <= corpus.queries.len());
    // Atomic directory creation prevents concurrent invocations from overwriting evidence.
    std::fs::create_dir(&directory).expect("output must be new and its parent must exist");
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("runs.jsonl"))
        .unwrap();
    let engines = SearchOptions::default().engines;
    let profile = crate::http_client::BrowserProfile::bing_experiment();
    let build = || {
        let transport = crate::TransportOptions::default();
        let standard = crate::http_client::standard_builder(profile, &transport)
            .timeout(SEARCH_TIMEOUT)
            .build()
            .unwrap();
        let mut yahoo = crate::http_client::impersonated_builder(profile, &transport)
            .timeout(SEARCH_TIMEOUT)
            .build()
            .unwrap();
        *yahoo.headers_mut() = profile.headers();
        SearchClients {
            standard,
            yahoo: Some(yahoo),
        }
    };
    let pools: Vec<_> = POLICIES.iter().map(|_| build()).collect();
    use sha2::{Digest, Sha256};
    let executable = std::env::current_exe().unwrap();
    let metadata = serde_json::json!({
        "schema_version": 1, "started_utc": chrono::Utc::now().to_rfc3339(),
        "revision": command("git", &["rev-parse", "HEAD"]),
        "tracked_diff_sha256": format!("{:x}", Sha256::digest(command("git", &["diff", "HEAD"]).as_bytes())),
        "test_binary_sha256": format!("{:x}", Sha256::digest(std::fs::read(executable).unwrap())),
        "rustc": command("rustc", &["--version"]), "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH, "context": context,
        "build": if cfg!(debug_assertions) { "debug" } else { "release" },
        "corpus": corpus, "query_limit": query_limit, "trials": trials,
        "policies": POLICIES, "enabled_providers": engines, "profile": format!("{profile:?}"),
        "minimum": 5, "diversity_minimum": 2, "deadline_seconds": budget_seconds,
        "concurrency": 9, "pacing_ms": 250, "query_syntax": "portable",
        "ranking": "production round-robin fusion; first five; no page fetch or body ranking",
        "reused_clients": "separate pool per policy; first use is cold; no excluded warmup",
        "timing": "monotonic search-only; excludes client construction; deadline-bounded full fanout",
        "unmeasured": ["isolated parser CPU", "peak RSS", "compressed wire bytes", "server cancellation latency", "compression/buffering", "independent provider feasibility"],
        "probe_limits": "provider fields aggregate a logical search, not individual attempts; missing provider map entries mean unobserved, not zero; incremental parser time includes worker wait and excludes final batch parsing"
    });
    std::fs::write(
        directory.join("metadata.json"),
        serde_json::to_vec_pretty(&metadata).unwrap(),
    )
    .unwrap();
    for trial in 0..trials {
        for (query_index, query) in corpus.queries.iter().take(query_limit).enumerate() {
            for fresh in [true, false] {
                for step in 0..POLICIES.len() {
                    let index = (step + trial + query_index) % POLICIES.len();
                    let policy = POLICIES[index];
                    let owned;
                    let clients = if fresh {
                        owned = build();
                        &owned
                    } else {
                        &pools[index]
                    };
                    let options = SearchOptions {
                        engines: engines.clone(),
                        mode: SearchMode::Fanout,
                        min_results: Some(5),
                        provider_quorum: None,
                        search_budget: Some(Duration::from_secs(budget_seconds as u64)),
                        max_concurrency: 9,
                        ..SearchOptions::default()
                    };
                    let probe = Arc::new(Mutex::new(Probe {
                        started: Instant::now(),
                        providers: BTreeMap::new(),
                    }));
                    let collector = Arc::new(Mutex::new(CollectorObservation::default()));
                    let (result, diagnostics, cancelled) = PROBE
                        .scope(
                            probe.clone(),
                            COLLECTOR.scope(
                                collector.clone(),
                                POLICY.scope(
                                    policy,
                                    BATCH_ONLY.scope(
                                        matches!(policy, Policy::Batch),
                                        measured_search(&query.query, &options, clients),
                                    ),
                                ),
                            ),
                        )
                        .await;
                    let elapsed = elapsed_millis(probe.lock().unwrap().started);
                    let (results, error) = match result {
                        Ok(results) => (results, None),
                        Err(error) => (Vec::new(), Some(error.to_string())),
                    };
                    let providers: std::collections::BTreeSet<_> = results
                        .iter()
                        .flat_map(|r| r.sources.iter().map(|s| s.engine.to_string()))
                        .collect();
                    let observation = serde_json::json!({
                        "query_id": query.id, "query": query.query, "trial": trial + 1,
                        "fresh_clients": fresh, "policy": policy, "elapsed_ms": elapsed,
                        "collector": *collector.lock().unwrap(),
                        "providers": probe.lock().unwrap().providers,
                        "diagnostics": diagnostics, "cancelled": cancelled, "error": error,
                        "contributing_providers": providers, "threshold_met_at_return": results.len() >= 5,
                        "final_keys": results.iter().take(5).map(result_key).collect::<Vec<_>>(),
                        "results": results, "final_results": results.iter().take(5).collect::<Vec<_>>()
                    });
                    serde_json::to_writer(&mut output, &observation).unwrap();
                    writeln!(output).unwrap();
                    output.flush().unwrap();
                    eprintln!(
                        "validation: trial {}, {}, fresh={fresh}, {policy:?}: {elapsed} ms, {} results",
                        trial + 1,
                        query.id,
                        results.len()
                    );
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
        }
    }
    std::fs::write(directory.join("COMPLETE"), "all scheduled runs recorded\n").unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    type Job = Pin<Box<dyn Future<Output = (usize, Result<Vec<SearchResult>, KestrelError>)>>>;
    fn records(start: usize, count: usize) -> Vec<SearchResult> {
        (start..start + count)
            .map(|i| {
                SearchResult::parsed(
                    "machine learning".into(),
                    format!("https://example.org/{i}"),
                    String::new(),
                    String::new(),
                )
            })
            .collect()
    }
    #[tokio::test]
    async fn policies_distinguish_five_results_from_two_contributing_providers() {
        let first = records(0, 5);
        let duplicate = records(0, 1);
        let empty = vec![];
        assert!(
            POLICY
                .scope(Policy::Stream, async { observe_collector(&[&first], true) })
                .await
        );
        assert!(
            !POLICY
                .scope(Policy::Full, async {
                    observe_collector(&[&first, &duplicate], true)
                })
                .await
        );
        assert!(
            !POLICY
                .scope(Policy::Diversity, async {
                    observe_collector(&[&first, &empty], true)
                })
                .await
        );
        // Contributing providers can corroborate the same URL; no extra unique URL is required.
        assert!(
            POLICY
                .scope(Policy::Diversity, async {
                    observe_collector(&[&first, &duplicate], true)
                })
                .await
        );
        assert!(
            !POLICY
                .scope(Policy::Diversity, async {
                    observe_collector(&[&duplicate, &duplicate], false)
                })
                .await
        );
        assert!(observe_collector(&[&first], true)); // Unscoped production behavior is unchanged.
    }
    #[tokio::test]
    async fn streamed_snapshot_cancels_only_the_immediate_policy() {
        for policy in [Policy::Stream, Policy::Diversity, Policy::Full] {
            let (sender, receiver) = mpsc::channel(1);
            let ready = Arc::new(tokio::sync::Notify::new());
            let second = ready.clone();
            let pending = FuturesUnordered::<Job>::new();
            pending.push(Box::pin(async move {
                let (resume, acknowledged) = oneshot::channel();
                sender
                    .send(Batch {
                        index: 0,
                        results: records(0, 5),
                        resume,
                    })
                    .await
                    .unwrap();
                let _ = acknowledged.await;
                ready.notify_one();
                (0, Ok(records(0, 5)))
            }));
            pending.push(Box::pin(async move {
                second.notified().await;
                (1, Ok(records(5, 1)))
            }));
            let (outcomes, cancelled) = tokio::time::timeout(
                Duration::from_secs(1),
                POLICY.scope(
                    policy,
                    collect(pending, None, Some(5), None, Some(receiver)),
                ),
            )
            .await
            .unwrap();
            let count = merge_outcomes(outcomes).unwrap().len();
            if matches!(policy, Policy::Stream) {
                assert_eq!(count, 5);
                assert_eq!(cancelled, 2);
            } else {
                assert_eq!(count, 6);
            }
        }
    }

    #[tokio::test]
    async fn full_and_diversity_wait_for_exhaustion_when_target_is_unavailable() {
        for policy in [Policy::Full, Policy::Diversity] {
            let pending = FuturesUnordered::<Job>::new();
            pending.push(Box::pin(async { (0, Ok(records(0, 7))) }));
            pending.push(Box::pin(async {
                tokio::task::yield_now().await;
                (1, Err(KestrelError::SearchDeadline))
            }));
            let (outcomes, cancelled) = POLICY
                .scope(policy, collect(pending, None, Some(5), None, None))
                .await;
            assert_eq!(cancelled, 0);
            assert_eq!(merge_outcomes(outcomes).unwrap().len(), 7); // Overshoot is retained.
        }
    }
    #[tokio::test]
    async fn collector_clock_counts_canonical_unique_candidates() {
        let probe = Arc::new(Mutex::new(Probe {
            started: Instant::now(),
            providers: BTreeMap::new(),
        }));
        let clock = Arc::new(Mutex::new(CollectorObservation::default()));
        PROBE
            .scope(
                probe,
                COLLECTOR.scope(
                    clock.clone(),
                    POLICY.scope(Policy::Full, async {
                        let four = records(0, 4);
                        let mut duplicate = records(0, 1);
                        duplicate[0].url.push_str("#fragment");
                        observe_collector(&[&four, &duplicate], false);
                        assert!(clock.lock().unwrap().first_five_unique_ms.is_none());
                        observe_collector(&[&records(0, 5)], true);
                        assert!(clock.lock().unwrap().first_five_unique_ms.is_some());
                        assert!(clock.lock().unwrap().threshold_observed_ms.is_none());
                    }),
                ),
            )
            .await;
    }
}
