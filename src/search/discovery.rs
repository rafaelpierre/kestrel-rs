//! Bounded recovery of empty queries. Completed providers are never rescheduled.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct Policy {
    initial: Option<Duration>,
    pub overall: Option<Duration>,
}

impl Policy {
    pub fn new(initial: Option<Duration>) -> Self {
        let overall = initial.map(|b| {
            if b >= Duration::from_secs(15) {
                b
            } else {
                b + (b + Duration::from_secs(5)).min(Duration::from_secs(15))
                    + (b + Duration::from_secs(10)).min(Duration::from_secs(15))
                    + Duration::from_secs(2)
            }
        });
        Self { initial, overall }
    }

    fn budget(self, attempt: usize) -> Option<Duration> {
        let b = self.initial?;
        if attempt == 1 {
            return Some(b);
        }
        if attempt > 3 || b >= Duration::from_secs(15) {
            return None;
        }
        Some((b + Duration::from_secs(5 * (attempt as u64 - 1))).min(Duration::from_secs(15)))
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run(
    query: &str,
    query_index: usize,
    engines: &[Engine],
    options: &SearchOptions,
    clients: &SearchClients,
    semaphore: Arc<Semaphore>,
    diagnostics: Arc<Mutex<Vec<ProviderSearchDiagnostic>>>,
    recovery: Option<&crate::SearchRecovery>,
    progress: Option<crate::recovery::ProgressQueue>,
    policy: Policy,
    overall: Option<tokio::time::Instant>,
) -> (Vec<Result<Vec<SearchResult>, KestrelError>>, usize) {
    let mut pending = engines.to_vec();
    let mut retained = Vec::new();
    let mut cancelled = 0;
    for attempt in 1..=3 {
        if recovery.is_some_and(|s| s.is_cancelled()) {
            break;
        }
        let budget = policy.budget(attempt);
        let deadline = budget.map(|b| {
            let end = tokio::time::Instant::now() + b;
            overall.map_or(end, |cap| end.min(cap))
        });
        let job = async {
            run_fanout_query(
                query,
                &pending,
                clients,
                Arc::clone(&semaphore),
                Arc::clone(&diagnostics),
                &options.region,
                options.time_filter,
                options.min_results.unwrap_or(5),
                deadline,
                progress.clone(),
            )
            .await
        };
        let (outcomes, dropped) = DISCOVERY_ATTEMPT.scope(attempt, job).await;
        cancelled += dropped;
        let has_results = outcomes
            .iter()
            .any(|r| r.as_ref().is_ok_and(|r| !r.is_empty()));
        retained.extend(outcomes);
        if has_results || recovery.is_some_and(|s| s.is_cancelled()) {
            break;
        }
        let Some(next_budget) = policy.budget(attempt + 1) else {
            break;
        };
        pending.retain(|engine| {
            diagnostics
                .lock()
                .expect("diagnostic lock")
                .iter()
                .rev()
                .find(|d| d.query == query && d.engine == *engine)
                .is_some_and(|d| d.discovery_attempt == attempt && d.outcome == "deadline")
        });
        if pending.is_empty() {
            break;
        }
        let base = 250 * attempt as u64;
        let delay = Duration::from_millis(base + rand::random::<u64>() % (base + 1));
        let Some(cap) = overall else {
            break;
        };
        if tokio::time::Instant::now() + delay >= cap {
            break;
        }
        eprintln!(
            "[kestrel] Discovery query {}: retry attempt {}/3 in {:.3}s; next budget {:.3}s for {} deadline-exhausted provider(s).",
            query_index + 1,
            attempt + 1,
            delay.as_secs_f64(),
            next_budget.as_secs_f64(),
            pending.len()
        );
        tokio::select! {
            () = tokio::time::sleep(delay) => (),
            () = async { match recovery { Some(s) => s.cancelled().await, None => std::future::pending().await } } => break,
        }
    }
    (retained, cancelled)
}

pub(super) fn failure(providers: &[ProviderSearchDiagnostic]) -> KestrelError {
    let attempts = providers
        .iter()
        .map(|p| p.discovery_attempt)
        .max()
        .unwrap_or(1);
    let mut latest = HashMap::new();
    for p in providers {
        latest.insert((p.engine, &p.query), p);
    }
    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
    for p in latest.values() {
        let cause = match p.outcome.as_str() {
            "deadline" => "exceeded their budgets",
            "rate_limited_deadline" => "hit a deadline after rate-limit/retry guidance",
            "challenge" => "returned a bot challenge",
            "response_too_large" => "exceeded the response-size limit",
            "unrecognized" => "returned an unrecognized search page",
            "cancelled_caller" => "were cancelled",
            _ => "failed with request errors",
        };
        *counts.entry(cause).or_default() += 1;
    }
    let causes = counts
        .into_iter()
        .map(|(cause, n)| format!("{n} {cause}"))
        .collect::<Vec<_>>()
        .join("; ");
    KestrelError::Search(format!(
        "No search results after up to {attempts} discovery attempt(s) per query: {causes}. Counts are latest outcomes per provider/query, not cumulative attempts."
    ))
}

/// Retry-After delta-seconds or HTTP date; invalid/overflowing values are ignored.
pub(super) fn retry_after(value: &str) -> Option<Duration> {
    let delay = if let Ok(seconds) = value.trim().parse::<u64>() {
        Duration::from_secs(seconds)
    } else {
        let date = chrono::DateTime::parse_from_rfc2822(value).ok()?;
        (date.with_timezone(&chrono::Utc) - chrono::Utc::now())
            .to_std()
            .unwrap_or(Duration::ZERO)
    };
    tokio::time::Instant::now()
        .checked_add(delay)
        .map(|_| delay)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    #[derive(Clone, Copy)]
    enum Behavior {
        Deadline,
        Recover,
        Result,
        DelayedResult,
        Records(usize, u64),
        Empty,
        Hard,
        Challenge,
        RateLimit,
    }
    pub struct Fixture {
        behavior: HashMap<(String, Engine), Behavior>,
        calls: Mutex<HashMap<(String, Engine), usize>>,
    }
    tokio::task_local! { pub static FIXTURE: Arc<Fixture>; }
    impl Fixture {
        fn new(rows: &[(&str, Engine, Behavior)]) -> Arc<Self> {
            Arc::new(Self {
                behavior: rows
                    .iter()
                    .map(|(q, e, b)| ((q.to_string(), *e), *b))
                    .collect(),
                calls: Mutex::new(HashMap::new()),
            })
        }
        pub async fn respond(
            &self,
            query: &str,
            engine: Engine,
        ) -> Result<ProviderResponse, KestrelError> {
            let key = (query.to_owned(), engine);
            let call = {
                let mut calls = self.calls.lock().unwrap();
                let n = calls.entry(key.clone()).or_default();
                *n += 1;
                *n
            };
            record_attempt();
            match self.behavior.get(&key).copied().unwrap_or(Behavior::Empty) {
                Behavior::Records(count, delay) => {
                    tokio::time::sleep(Duration::from_secs(delay)).await;
                    let results = (0..count)
                        .map(|index| {
                            SearchResult::parsed(
                                format!("{engine} {index}"),
                                format!("https://example.com/{engine}/{index}"),
                                String::new(),
                                "evidence".into(),
                            )
                        })
                        .collect();
                    Ok(ProviderResponse {
                        results,
                        raw_result_count: count,
                        retries: 0,
                    })
                }
                Behavior::Deadline => std::future::pending().await,
                Behavior::Recover if call == 1 => std::future::pending().await,
                Behavior::Hard => Err(KestrelError::Search(
                    "request failed: deadline exceeded in unrelated text".into(),
                )),
                Behavior::Challenge => Err(KestrelError::Search(
                    "provider returned a bot challenge".into(),
                )),
                Behavior::RateLimit => {
                    observe(|r| r.headers(429, Some("60".into())));
                    std::future::pending().await
                }
                behavior => {
                    if matches!(behavior, Behavior::DelayedResult) {
                        tokio::time::sleep(Duration::from_secs(8)).await;
                    }
                    let results = if matches!(behavior, Behavior::Empty) {
                        Vec::new()
                    } else {
                        vec![SearchResult::parsed(
                            "answer".into(),
                            "https://example.com/shared".into(),
                            String::new(),
                            "evidence".into(),
                        )]
                    };
                    Ok(ProviderResponse {
                        raw_result_count: results.len(),
                        results,
                        retries: 0,
                    })
                }
            }
        }
        fn calls(&self, query: &str, engine: Engine) -> usize {
            self.calls
                .lock()
                .unwrap()
                .get(&(query.into(), engine))
                .copied()
                .unwrap_or(0)
        }
    }
    async fn search(
        fixture: Arc<Fixture>,
        queries: &[&str],
        engines: &[Engine],
        budget: Option<Duration>,
    ) -> Result<SearchReport, KestrelError> {
        let options = SearchOptions {
            engines: engines.to_vec(),
            search_budget: budget,
            ..Default::default()
        };
        FIXTURE
            .scope(
                fixture,
                search_many_detailed(
                    &queries.iter().map(|q| q.to_string()).collect::<Vec<_>>(),
                    &options,
                ),
            )
            .await
    }

    #[tokio::test(start_paused = true)]
    async fn compatibility_quorum_neither_stops_early_nor_delays_result_minimum() {
        // Exercise public options through production discovery, not a collector shim.
        for quorum in [None, Some(0), Some(1), Some(2), Some(usize::MAX)] {
            for (minimum, count, seconds, cancelled) in [
                (None, 5, 2, 2),
                (Some(1), 1, 1, 3),
                (Some(5), 5, 2, 2),
                (Some(6), 7, 3, 1),
                (Some(8), 7, 15, 0),
            ] {
                let fixture = Fixture::new(&[
                    ("q", Engine::Bing, Behavior::Records(1, 1)),
                    ("q", Engine::Yahoo, Behavior::Records(4, 2)),
                    ("q", Engine::Yep, Behavior::Records(2, 3)),
                    ("q", Engine::Mojeek, Behavior::Deadline),
                ]);
                let options = SearchOptions {
                    engines: vec![Engine::Bing, Engine::Yahoo, Engine::Yep, Engine::Mojeek],
                    provider_quorum: quorum,
                    min_results: minimum,
                    search_budget: Some(Duration::from_secs(15)),
                    ..Default::default()
                };
                let started = tokio::time::Instant::now();
                let report = FIXTURE
                    .scope(
                        fixture.clone(),
                        search_many_detailed(&["q".into()], &options),
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    report.results.len(),
                    count,
                    "quorum={quorum:?}, minimum={minimum:?}"
                );
                assert_eq!(started.elapsed(), Duration::from_secs(seconds));
                assert_eq!(report.cancelled, cancelled);
                assert_eq!(
                    report
                        .providers
                        .iter()
                        .filter(|p| p.outcome == "cancelled_min_results")
                        .count(),
                    cancelled
                );
                assert!(
                    report
                        .providers
                        .iter()
                        .all(|p| p.outcome != "cancelled_quorum")
                );
                for engine in options.engines {
                    assert_eq!(fixture.calls("q", engine), 1);
                }
                assert!(
                    report
                        .results
                        .iter()
                        .all(|r| r.query.as_deref() == Some("q") && r.sources.len() == 1)
                );
            }
        }
    }

    #[test]
    fn budgets_are_finite_and_explicit_large_budgets_are_single_attempt() {
        let p = Policy::new(Some(Duration::from_secs(5)));
        assert_eq!(
            (p.budget(1), p.budget(2), p.budget(3), p.budget(4)),
            (
                Some(Duration::from_secs(5)),
                Some(Duration::from_secs(10)),
                Some(Duration::from_secs(15)),
                None
            )
        );
        assert_eq!(p.overall, Some(Duration::from_secs(32)));
        for b in [15, 32, 100] {
            assert_eq!(Policy::new(Some(Duration::from_secs(b))).budget(2), None);
        }
        assert_eq!(Policy::new(None).overall, None);
        assert_eq!(
            Policy::new(Some(Duration::from_secs(14))).budget(3),
            Some(Duration::from_secs(15))
        );
    }
    #[tokio::test(start_paused = true)]
    async fn mixed_deadline_challenge_recovers_only_deadline_work() {
        let f = Fixture::new(&[
            ("q", Engine::Bing, Behavior::Recover),
            ("q", Engine::Mojeek, Behavior::Challenge),
        ]);
        let started = tokio::time::Instant::now();
        let report = search(
            f.clone(),
            &["q"],
            &[Engine::Bing, Engine::Mojeek],
            Some(Duration::from_secs(5)),
        )
        .await
        .unwrap();
        assert_eq!(f.calls("q", Engine::Bing), 2);
        assert_eq!(f.calls("q", Engine::Mojeek), 1);
        assert_eq!(report.results.len(), 1);
        assert_eq!(report.results[0].sources[0].query, "q");
        assert_eq!(
            report
                .providers
                .iter()
                .map(|p| p.discovery_attempt)
                .collect::<Vec<_>>(),
            vec![1, 1, 2]
        );
        assert!(
            (Duration::from_millis(5250)..=Duration::from_millis(5501))
                .contains(&started.elapsed())
        );
    }
    #[tokio::test(start_paused = true)]
    async fn all_deadlines_stop_at_three_with_grouped_latest_counts() {
        let f = Fixture::new(&[
            ("q", Engine::Bing, Behavior::Deadline),
            ("q", Engine::Mojeek, Behavior::Challenge),
        ]);
        let started = tokio::time::Instant::now();
        let error = search(
            f.clone(),
            &["q"],
            &[Engine::Bing, Engine::Mojeek],
            Some(Duration::from_secs(5)),
        )
        .await
        .unwrap_err()
        .to_string();
        assert_eq!(f.calls("q", Engine::Bing), 3);
        assert_eq!(f.calls("q", Engine::Mojeek), 1);
        assert!(error.contains("3 discovery"));
        assert!(error.contains("1 exceeded their budgets"));
        assert!(error.contains("1 returned a bot challenge"));
        assert!(started.elapsed() <= Duration::from_secs(32));
    }
    #[tokio::test(start_paused = true)]
    async fn completed_empty_hard_and_partial_do_not_retry() {
        for behavior in [
            Behavior::Empty,
            Behavior::Hard,
            Behavior::Challenge,
            Behavior::RateLimit,
        ] {
            let f = Fixture::new(&[("q", Engine::Bing, behavior)]);
            let result = search(
                f.clone(),
                &["q"],
                &[Engine::Bing],
                Some(Duration::from_secs(5)),
            )
            .await;
            assert_eq!(result.is_ok(), matches!(behavior, Behavior::Empty));
            assert_eq!(f.calls("q", Engine::Bing), 1);
        }
        let f = Fixture::new(&[
            ("q", Engine::Bing, Behavior::Result),
            ("q", Engine::Mojeek, Behavior::Deadline),
        ]);
        let report = search(
            f.clone(),
            &["q"],
            &[Engine::Bing, Engine::Mojeek],
            Some(Duration::from_secs(5)),
        )
        .await
        .unwrap();
        assert_eq!(report.results.len(), 1);
        assert_eq!(f.calls("q", Engine::Mojeek), 1);
    }
    #[tokio::test(start_paused = true)]
    async fn multi_query_does_not_rerun_successes_and_preserves_deduplication() {
        let f = Fixture::new(&[
            ("site:example.com q", Engine::Bing, Behavior::Result),
            ("q2", Engine::Bing, Behavior::Recover),
        ]);
        let report = search(
            f.clone(),
            &["site:example.com q", "q2"],
            &[Engine::Bing],
            Some(Duration::from_secs(5)),
        )
        .await
        .unwrap();
        assert_eq!(f.calls("site:example.com q", Engine::Bing), 1);
        assert_eq!(f.calls("q2", Engine::Bing), 2);
        assert_eq!(report.results.len(), 1);
        assert_eq!(report.results[0].sources.len(), 2);
    }
    #[tokio::test(start_paused = true)]
    async fn dropping_during_backoff_or_active_work_stops_requests() {
        for advance in [100, 5100, 5600] {
            let f = Fixture::new(&[("q", Engine::Bing, Behavior::Deadline)]);
            let mut job = Box::pin(search(
                f.clone(),
                &["q"],
                &[Engine::Bing],
                Some(Duration::from_secs(5)),
            ));
            assert!(futures_util::poll!(&mut job).is_pending());
            tokio::time::advance(Duration::from_millis(advance)).await;
            assert!(futures_util::poll!(&mut job).is_pending());
            let count = f.calls("q", Engine::Bing);
            drop(job);
            tokio::time::advance(Duration::from_secs(60)).await;
            assert_eq!(f.calls("q", Engine::Bing), count);
        }
    }
    #[tokio::test(start_paused = true)]
    async fn unlimited_and_large_explicit_budgets_do_not_recover() {
        let f = Fixture::new(&[("q", Engine::Bing, Behavior::Deadline)]);
        assert!(
            search(
                f.clone(),
                &["q"],
                &[Engine::Bing],
                Some(Duration::from_secs(15))
            )
            .await
            .is_err()
        );
        assert_eq!(f.calls("q", Engine::Bing), 1);
        let f = Fixture::new(&[("q", Engine::Bing, Behavior::Empty)]);
        assert!(
            search(f.clone(), &["q"], &[Engine::Bing], None)
                .await
                .is_ok()
        );
        assert_eq!(f.calls("q", Engine::Bing), 1);
    }
    #[tokio::test(start_paused = true)]
    async fn dropping_backoff_and_second_attempt_and_precancelled_store_stop_work() {
        for active_second in [false, true] {
            let f = Fixture::new(&[("q", Engine::Bing, Behavior::Deadline)]);
            let tmp = tempfile::tempdir().unwrap();
            let store = crate::SearchRecovery::new(tmp.path().to_owned(), Duration::from_secs(300))
                .unwrap();
            let options = SearchOptions {
                engines: vec![Engine::Bing],
                search_budget: Some(Duration::from_secs(5)),
                ..Default::default()
            };
            let clients = SearchClients::new(&options.engines).unwrap();
            // Test future cancellation independently from disk scheduling, then
            // verify a pre-cancelled recovery store never starts another attempt.
            let mut job = Box::pin(search(
                f.clone(),
                &["q"],
                &[Engine::Bing],
                options.search_budget,
            ));
            assert!(futures_util::poll!(&mut job).is_pending());
            tokio::time::advance(Duration::from_secs(5)).await;
            assert!(futures_util::poll!(&mut job).is_pending());
            if active_second {
                tokio::time::advance(Duration::from_millis(501)).await;
                assert!(futures_util::poll!(&mut job).is_pending());
                assert_eq!(f.calls("q", Engine::Bing), 2);
            } else {
                assert_eq!(f.calls("q", Engine::Bing), 1);
            }
            drop(job);
            store.cancel();
            let before = tokio::time::Instant::now();
            let policy = Policy::new(options.search_budget);
            let result = run(
                "q",
                0,
                &options.engines,
                &options,
                &clients,
                Arc::new(Semaphore::new(1)),
                Arc::new(Mutex::new(Vec::new())),
                Some(&store),
                None,
                policy,
                Some(before + Duration::from_secs(32)),
            )
            .await;
            assert!(result.0.is_empty());
            assert_eq!(before.elapsed(), Duration::ZERO);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn controlled_policy_cost_against_single_long_attempt() {
        for (name, behavior) in [
            ("first-attempt stall", Behavior::Recover),
            ("eight-second response", Behavior::DelayedResult),
            ("persistent stall", Behavior::Deadline),
        ] {
            for budget in [5, 32] {
                let mut durations = Vec::new();
                let mut successes = 0;
                let mut requests = 0;
                for _ in 0..10 {
                    let f = Fixture::new(&[("q", Engine::Bing, behavior)]);
                    let start = tokio::time::Instant::now();
                    successes += usize::from(
                        search(
                            f.clone(),
                            &["q"],
                            &[Engine::Bing],
                            Some(Duration::from_secs(budget)),
                        )
                        .await
                        .is_ok(),
                    );
                    durations.push(start.elapsed().as_millis());
                    requests += f.calls("q", Engine::Bing);
                }
                durations.sort_unstable();
                eprintln!(
                    "POLICY_COST scenario={name:?} first_budget={budget} n=10 successes={successes} sends={requests} p50_ms={} p95_ms={}",
                    durations[4], durations[9]
                );
                assert_eq!(
                    successes,
                    if matches!(behavior, Behavior::Deadline)
                        || (matches!(behavior, Behavior::Recover) && budget == 32)
                    {
                        0
                    } else {
                        10
                    }
                );
                assert!(durations[9] <= 32_000);
            }
        }
    }

    #[test]
    fn retry_after_accepts_seconds_dates_and_rejects_invalid() {
        assert_eq!(retry_after("60"), Some(Duration::from_secs(60)));
        assert_eq!(
            retry_after("Wed, 21 Oct 2015 07:28:00 GMT"),
            Some(Duration::ZERO)
        );
        assert_eq!(retry_after("invalid"), None);
    }
}
