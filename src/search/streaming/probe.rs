//! Opt-in live feasibility measurement; never runs in the ordinary test suite.
use super::*;
use crate::model::SearchMode;
use serde::Serialize;

#[derive(Default, Serialize)]
struct Observation {
    http_version: String,
    http_status: u16,
    headers_ms: u64,
    first_chunk_ms: Option<u64>,
    first_record_ms: Option<u64>,
    fifth_unique_record_ms: Option<u64>,
    body_eof_ms: Option<u64>,
    decompressed_bytes_received: usize,
    parse_microseconds: u128,
    records: Vec<SearchResult>,
}
struct Probe {
    started: Instant,
    providers: BTreeMap<String, Observation>,
}

tokio::task_local! {
    static PROBE: Arc<Mutex<Probe>>;
    pub(super) static BATCH_ONLY: bool;
}
fn update(engine: Engine, action: impl FnOnce(&mut Observation, u64)) {
    let _ = PROBE.try_with(|probe| {
        let mut probe = probe.lock().unwrap();
        let elapsed = elapsed_millis(probe.started);
        action(
            probe.providers.entry(engine.to_string()).or_default(),
            elapsed,
        );
    });
}
pub(crate) fn headers(engine: Engine, version: String, status: u16) {
    update(engine, |r, elapsed| {
        r.http_version = version;
        r.http_status = status;
        r.headers_ms = elapsed;
    });
}
pub(crate) fn bytes(engine: Engine, len: usize) {
    update(engine, |r, elapsed| {
        r.first_chunk_ms.get_or_insert(elapsed);
        r.decompressed_bytes_received += len;
    });
}
pub(crate) fn eof(engine: Engine) {
    update(engine, |r, elapsed| r.body_eof_ms = Some(elapsed));
}
pub(crate) fn results(engine: Engine, results: &[SearchResult]) {
    update(engine, |r, elapsed| {
        r.records = results.to_vec();
        if !results.is_empty() {
            r.first_record_ms.get_or_insert(elapsed);
        }
        if results.iter().map(result_key).collect::<HashSet<_>>().len() >= 5 {
            r.fifth_unique_record_ms.get_or_insert(elapsed);
        }
    });
}
pub(crate) fn parse_time(engine: Engine, micros: u128) {
    update(engine, |r, _| r.parse_microseconds += micros);
}

#[tokio::test]
#[ignore = "live providers: explicit feasibility benchmark"]
async fn live_streaming_feasibility() {
    let engines = vec![
        Engine::Duckduckgo,
        Engine::Bing,
        Engine::Yahoo,
        Engine::Dogpile,
        Engine::Ecosia,
        Engine::Swisscows,
        Engine::Yep,
        Engine::Qwant,
        Engine::Mojeek,
    ];
    let reused = SearchClients::new(&engines).unwrap();
    let mut measurements = Vec::new();
    let queries = [
        "Rust ownership borrowing documentation",
        "PostgreSQL EXPLAIN ANALYZE documentation",
        "what is machine learning",
    ];
    for fresh in [true, false] {
        for (query_index, query) in queries.iter().enumerate() {
            for step in 0..2 {
                let mode = ["batch", "stream"][(step + query_index) % 2];
                let owned;
                let clients = if fresh {
                    owned = SearchClients::new(&engines).unwrap();
                    &owned
                } else {
                    &reused
                };
                let options = SearchOptions {
                    engines: engines.clone(),
                    mode: SearchMode::Fanout,
                    min_results: Some(5),
                    provider_quorum: None,
                    search_budget: Some(Duration::from_secs(5)),
                    max_concurrency: 9,
                    ..SearchOptions::default()
                };
                let probe = Arc::new(Mutex::new(Probe {
                    started: Instant::now(),
                    providers: BTreeMap::new(),
                }));
                let result = PROBE
                    .scope(
                        probe.clone(),
                        BATCH_ONLY.scope(
                            mode == "batch",
                            search_many_reusing_clients_detailed(
                                &[(*query).to_owned()],
                                &options,
                                clients,
                            ),
                        ),
                    )
                    .await;
                let elapsed = elapsed_millis(probe.lock().unwrap().started);
                let (results, diagnostics, cancelled, error) = match result {
                    Ok(report) => (report.results, report.providers, report.cancelled, None),
                    Err(error) => (Vec::new(), Vec::new(), 0, Some(error.to_string())),
                };
                let observation = serde_json::json!({
                    "query": query, "mode": mode, "fresh_clients": fresh,
                    "elapsed_ms": elapsed, "providers": probe.lock().unwrap().providers,
                    "diagnostics": diagnostics, "cancelled": cancelled, "error": error,
                    "results": results,
                });
                eprintln!(
                    "feasibility: {mode}, fresh={fresh}, {query}: {elapsed} ms, {} candidates",
                    results.len()
                );
                measurements.push(observation);
            }
        }
    }
    let directory = std::path::Path::new("benchmarks/results");
    std::fs::create_dir_all(directory).unwrap();
    std::fs::write(
        directory.join("streaming-feasibility.json"),
        serde_json::to_string_pretty(&measurements).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
#[ignore = "live providers: repeated four-query fusion and latency evidence"]
async fn live_fusion_latency_evidence() {
    let engines = vec![
        Engine::Duckduckgo,
        Engine::Bing,
        Engine::Yahoo,
        Engine::Dogpile,
        Engine::Ecosia,
        Engine::Swisscows,
        Engine::Yep,
        Engine::Qwant,
        Engine::Mojeek,
    ];
    // One browser/header profile for every arm, including newly constructed clients.
    let profile = crate::http_client::BrowserProfile::random();
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
    // Separate reusable pools per arm prevent one mode warming the other's sockets.
    let batch_clients = build();
    let stream_clients = build();
    let queries = [
        "Rust ownership borrowing documentation",
        "PostgreSQL EXPLAIN ANALYZE documentation",
        "what is machine learning",
        "how do solar panels generate electricity",
    ];
    let directory = std::path::Path::new("benchmarks/results/fusion-evidence-result-minimum");
    std::fs::create_dir_all(directory).unwrap();
    let started_utc = chrono::Utc::now().to_rfc3339();
    let mut measurements = Vec::new();
    for trial in 0..3 {
        for fresh in [true, false] {
            for (query_index, query) in queries.iter().enumerate() {
                for step in 0..2 {
                    let mode = ["batch", "stream"][(step + query_index + trial) % 2];
                    let owned;
                    let clients = if fresh {
                        owned = build();
                        &owned
                    } else if mode == "batch" {
                        &batch_clients
                    } else {
                        &stream_clients
                    };
                    let options = SearchOptions {
                        engines: engines.clone(),
                        mode: SearchMode::Fanout,
                        min_results: Some(5),
                        provider_quorum: Some(2),
                        search_budget: Some(Duration::from_secs(3)),
                        max_concurrency: 9,
                        ..SearchOptions::default()
                    };
                    let probe = Arc::new(Mutex::new(Probe {
                        started: Instant::now(),
                        providers: BTreeMap::new(),
                    }));
                    let result = PROBE
                        .scope(
                            probe.clone(),
                            BATCH_ONLY.scope(
                                mode == "batch",
                                search_many_reusing_clients_detailed(
                                    &[(*query).to_owned()],
                                    &options,
                                    clients,
                                ),
                            ),
                        )
                        .await;
                    let elapsed = elapsed_millis(probe.lock().unwrap().started);
                    let (results, diagnostics, cancelled, error) = match result {
                        Ok(report) => (report.results, report.providers, report.cancelled, None),
                        Err(error) => (Vec::new(), Vec::new(), 0, Some(error.to_string())),
                    };
                    let contributors: HashSet<_> = results
                        .iter()
                        .flat_map(|r| r.sources.iter().map(|s| s.engine))
                        .collect();
                    let final_results: Vec<_> = results.iter().take(5).cloned().collect();
                    measurements.push(serde_json::json!({
                        "query": query, "mode": mode, "trial": trial + 1, "fresh_clients": fresh,
                        "elapsed_ms": elapsed, "providers": probe.lock().unwrap().providers,
                        "diagnostics": diagnostics, "cancelled": cancelled, "error": error,
                        "threshold_met": results.len() >= 5,
                        "diversity_met": contributors.len() >= 2,
                        "contributing_providers": contributors,
                        "results": results, "final_results": final_results,
                    }));
                    eprintln!(
                        "fusion evidence {}/48: trial {}, fresh={fresh}, {mode}, {query}: {elapsed} ms, {} unique, {} providers",
                        measurements.len(),
                        trial + 1,
                        results.len(),
                        contributors.len()
                    );
                    // Checkpoint every run so failures/timeouts cannot erase earlier evidence.
                    std::fs::write(directory.join("runs.json"), serde_json::to_string_pretty(&serde_json::json!({
                        "started_utc": started_utc, "updated_utc": chrono::Utc::now().to_rfc3339(),
                        "build": "debug", "minimum_unique_results": 5, "provider_quorum": null,
                        "requested_provider_quorum": 2,
                        "deadline_seconds": 3, "max_concurrency": 9, "enabled_providers": engines,
                        "profile": {"browser": format!("{:?}", profile.browser), "os": format!("{:?}", profile.os), "accept_language": profile.headers().get("accept-language").unwrap().to_str().unwrap()},
                        "ranking": "provider round-robin fusion, canonical URL deduplication, first five; no page fetching",
                        "runs": measurements,
                    })).unwrap()).unwrap();
                }
            }
        }
    }
}
