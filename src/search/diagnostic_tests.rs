use super::*;
use crate::benchmarking::TEST_TRACE_DIRECTORY;
use crate::provider_diagnostics::Lifecycle;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

async fn request(yahoo: bool, url: &str) -> Result<(String, usize), KestrelError> {
    if yahoo {
        let client = primp::Client::builder().no_proxy().build().unwrap();
        request_yahoo_with_retries("test", || client.get(url)).await
    } else {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        request_standard_with_retries(&client, Engine::Bing, "test", || client.get(url)).await
    }
}

async fn recorded(yahoo: bool, url: &str) -> (Result<(String, usize), KestrelError>, Lifecycle) {
    let recorder = Arc::new(Mutex::new(Recorder::with_run("mock-run".into())));
    let result = PROVIDER_RECORDER
        .scope(Arc::clone(&recorder), request(yahoo, url))
        .await;
    let snapshot = recorder.lock().unwrap().finish_outcome(
        if result.is_ok() {
            "results"
        } else {
            "request_error"
        },
        false,
    );
    (result, snapshot)
}

fn json_files(directory: &std::path::Path) -> Vec<serde_json::Value> {
    std::fs::read_dir(directory)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .map(|p| serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap())
        .collect()
}

#[tokio::test]
async fn both_backends_record_status_challenge_retry_after_and_raw_correlation() {
    for yahoo in [false, true] {
        for status in [200, 403, 429] {
            let server = MockServer::start().await;
            let count = if status == 429 { 3 } else { 1 };
            Mock::given(method("GET"))
                .respond_with(
                    ResponseTemplate::new(status)
                        .insert_header("retry-after", "120")
                        .insert_header("set-cookie", "private-cookie=do-not-capture")
                        .set_body_string("<form id='captcha'>challenge</form>"),
                )
                .expect(count)
                .mount(&server)
                .await;
            let directory = tempfile::tempdir().unwrap();
            let (result, snapshot) = TEST_TRACE_DIRECTORY
                .scope(
                    Some(directory.path().to_owned()),
                    recorded(yahoo, &server.uri()),
                )
                .await;
            assert_eq!(result.is_ok(), status == 200);
            assert_eq!(snapshot.send_attempts, count as usize);
            assert_eq!(snapshot.attempts.len(), count as usize);
            let mut ids = std::collections::HashSet::new();
            for attempt in &snapshot.attempts {
                assert!(ids.insert(attempt.attempt_id.clone()));
                assert_eq!(attempt.run_id, snapshot.run_id);
                assert_eq!(attempt.search_id, snapshot.search_id);
                assert_eq!(attempt.http_status, Some(status));
                assert_eq!(attempt.retry_after.as_deref(), Some("120"));
                assert_eq!(attempt.challenge, Challenge::Detected);
                assert_eq!(attempt.outcome, Some("response"));
                assert_eq!(attempt.transport_error, None);
            }
            let captures = json_files(directory.path());
            assert_eq!(captures.len(), count as usize);
            for capture in captures {
                assert!(ids.contains(capture["correlation"]["attempt_id"].as_str().unwrap()));
                assert_eq!(capture["correlation"]["run_id"], "mock-run");
                assert!(!capture.to_string().contains("private-cookie"));
                let body = std::fs::read_to_string(
                    directory
                        .path()
                        .join(capture["html_file"].as_str().unwrap()),
                )
                .unwrap();
                assert!(body.contains("challenge"));
            }
        }
    }
}

#[tokio::test]
async fn recovery_and_exhaustion_have_one_logical_outcome() {
    for yahoo in [false, true] {
        for recover in [false, true] {
            let server = MockServer::start().await;
            let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let seen = Arc::clone(&calls);
            Mock::given(method("GET"))
                .respond_with(move |_: &wiremock::Request| {
                    let n = seen.fetch_add(1, Ordering::Relaxed);
                    ResponseTemplate::new(if recover && n > 0 { 200 } else { 500 })
                        .set_body_string("response")
                })
                .mount(&server)
                .await;
            let (result, snapshot) = recorded(yahoo, &server.uri()).await;
            let count = if recover { 2 } else { 3 };
            assert_eq!(calls.load(Ordering::Relaxed), count);
            assert_eq!(snapshot.attempts.len(), count);
            assert_eq!(snapshot.send_attempts, count);
            assert_eq!(
                snapshot.logical_outcome.as_deref(),
                Some(if recover { "results" } else { "request_error" })
            );
            assert_eq!(result.is_ok(), recover);
            assert!(
                snapshot
                    .attempts
                    .iter()
                    .all(|a| a.outcome == Some("response"))
            );
            assert_eq!(snapshot.attempts[0].http_status, Some(500));
            assert_eq!(
                snapshot.attempts.last().unwrap().http_status,
                Some(if recover { 200 } else { 500 })
            );
        }
    }
}

#[tokio::test]
async fn typed_connection_errors_have_no_http_status_or_challenge_judgment() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    for yahoo in [false, true] {
        let (result, snapshot) = recorded(yahoo, &url).await;
        assert!(result.is_err());
        assert_eq!(snapshot.send_attempts, 3);
        for attempt in snapshot.attempts {
            assert_eq!(attempt.http_status, None);
            assert_eq!(attempt.challenge, Challenge::Unknown);
            assert_eq!(attempt.transport_error, Some(TransportKind::Connect));
            assert_eq!(attempt.outcome, Some("transport_error"));
        }
    }
}

#[tokio::test]
async fn queued_and_never_polled_jobs_finalize_on_drop() {
    for polled in [false, true] {
        for quorum in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            TEST_TRACE_DIRECTORY
                .scope(Some(directory.path().to_owned()), async {
                    let diagnostics = Arc::new(Mutex::new(Vec::new()));
                    let provider = async { panic!("queued provider must never run") };
                    let mut job = Box::pin(run_one_job(
                        "test",
                        Engine::Bing,
                        Arc::new(Semaphore::new(0)),
                        Arc::clone(&diagnostics),
                        None,
                        Some(Arc::new(AtomicU8::new(u8::from(quorum)))),
                        provider,
                    ));
                    assert_eq!(diagnostics.lock().unwrap().len(), 1);
                    if polled {
                        assert!(futures_util::poll!(&mut job).is_pending());
                    }
                    drop(job);
                    assert_eq!(diagnostics.lock().unwrap()[0].retries, 0);
                })
                .await;
            let records = json_files(directory.path());
            assert_eq!(records.len(), 1);
            let record = &records[0];
            assert_eq!(
                record["outcome"],
                if quorum {
                    "cancelled_quorum"
                } else {
                    "cancelled_caller"
                }
            );
            assert_eq!(
                record["lifecycle"]["cancellation_phase"],
                if polled { "queue" } else { "not_started" }
            );
            assert_eq!(record["lifecycle"]["send_attempts"], 0);
            assert_eq!(record["lifecycle"]["attempts"].as_array().unwrap().len(), 0);
        }
    }
}

#[tokio::test]
async fn deadline_during_backoff_keeps_completed_attempt() {
    for yahoo in [false, true] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500).set_body_string("failed"))
            .expect(1)
            .mount(&server)
            .await;
        let directory = tempfile::tempdir().unwrap();
        TEST_TRACE_DIRECTORY
            .scope(Some(directory.path().to_owned()), async {
                TEST_RETRY_DELAY
                    .scope(Duration::from_secs(60), async {
                        let diagnostics = Arc::new(Mutex::new(Vec::new()));
                        let provider = async {
                            request(yahoo, &server.uri()).await?;
                            unreachable!()
                        };
                        let result = run_one_job(
                            "test",
                            Engine::Bing,
                            Arc::new(Semaphore::new(1)),
                            diagnostics,
                            Some(tokio::time::Instant::now() + Duration::from_millis(200)),
                            None,
                            provider,
                        )
                        .await;
                        assert!(result.is_err());
                    })
                    .await;
            })
            .await;
        let records = json_files(directory.path());
        let lifecycle = &records
            .iter()
            .find(|r| r.get("lifecycle").is_some())
            .unwrap()["lifecycle"];
        assert_eq!(lifecycle["logical_outcome"], "deadline");
        assert_eq!(lifecycle["cancellation_phase"], "backoff");
        assert_eq!(lifecycle["send_attempts"], 1);
        assert_eq!(lifecycle["attempts"][0]["http_status"], 500);
        assert_eq!(lifecycle["attempts"][0]["outcome"], "response");
        assert_eq!(
            lifecycle["intervals"].as_array().unwrap().last().unwrap()["censored"],
            true
        );
    }
}

#[tokio::test]
async fn quorum_drops_inflight_send_and_preserves_shared_run_id() {
    threshold_drops_inflight_send(None, "cancelled_quorum").await;
}

#[tokio::test]
async fn minimum_drops_inflight_send_and_preserves_shared_run_id() {
    threshold_drops_inflight_send(Some(1), "cancelled_min_results").await;
}

async fn threshold_drops_inflight_send(min_results: Option<usize>, expected: &str) {
    let directory = tempfile::tempdir().unwrap();
    TEST_TRACE_DIRECTORY
        .scope(Some(directory.path().to_owned()), async {
            DIAGNOSTIC_RUN_ID
                .scope("shared-run".into(), async {
                    let diagnostics = Arc::new(Mutex::new(Vec::new()));
                    let signal = Arc::new(AtomicU8::new(0));
                    let semaphore = Arc::new(Semaphore::new(2));
                    let slow = run_one_job(
                        "test",
                        Engine::Bing,
                        Arc::clone(&semaphore),
                        Arc::clone(&diagnostics),
                        None,
                        Some(Arc::clone(&signal)),
                        async {
                            record_attempt();
                            std::future::pending::<Result<ProviderResponse, KestrelError>>().await
                        },
                    );
                    let fast = run_one_job(
                        "test",
                        Engine::Yahoo,
                        semaphore,
                        Arc::clone(&diagnostics),
                        None,
                        Some(Arc::clone(&signal)),
                        async {
                            Ok(ProviderResponse {
                                results: vec![SearchResult::parsed(
                                    "ok".into(),
                                    "https://example.org".into(),
                                    String::new(),
                                    String::new(),
                                )],
                                retries: 0,
                                raw_result_count: 1,
                            })
                        },
                    );
                    let pending = FuturesUnordered::new();
                    type Job<'a> = std::pin::Pin<
                        Box<
                            dyn Future<Output = (usize, Result<Vec<SearchResult>, KestrelError>)>
                                + 'a,
                        >,
                    >;
                    pending.push(Box::pin(async { (0_usize, slow.await) }) as Job<'_>);
                    pending.push(Box::pin(async { (1_usize, fast.await) }) as Job<'_>);
                    let (_, cancelled) =
                        collect_fanout_signalled(pending, Some(1), min_results, Some(signal)).await;
                    assert_eq!(cancelled, 1);
                    assert_eq!(diagnostics.lock().unwrap().len(), 2);
                })
                .await;
        })
        .await;
    let records = json_files(directory.path());
    assert_eq!(records.len(), 2);
    let slow = &records.iter().find(|r| r["engine"] == "bing").unwrap()["lifecycle"];
    assert_eq!(slow["logical_outcome"], expected);
    assert_eq!(slow["cancellation_phase"], "send");
    assert_eq!(slow["attempts"][0]["outcome"], "cancelled");
    assert!(
        records
            .iter()
            .all(|r| r["lifecycle"]["run_id"] == "shared-run")
    );
    assert_ne!(
        records[0]["lifecycle"]["search_id"],
        records[1]["lifecycle"]["search_id"]
    );
}

#[tokio::test]
async fn incomplete_bodies_keep_headers_and_typed_error() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for yahoo in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(stream.read(&mut request).await.unwrap() > 0);
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nRetry-After: Wed, 21 Oct 2015 07:28:00 GMT\r\nConnection: close\r\n\r\nshort").await.unwrap();
        });
        let (result, snapshot) = recorded(yahoo, &url).await;
        server.await.unwrap();
        assert!(result.is_err());
        assert_eq!(snapshot.send_attempts, 1);
        let attempt = &snapshot.attempts[0];
        assert_eq!(attempt.http_status, Some(200));
        assert_eq!(
            attempt.retry_after.as_deref(),
            Some("Wed, 21 Oct 2015 07:28:00 GMT")
        );
        assert_eq!(attempt.outcome, Some("body_error"));
        assert!(matches!(
            attempt.transport_error,
            Some(TransportKind::Body | TransportKind::Decode)
        ));
        assert_eq!(attempt.challenge, Challenge::Unknown);
    }
}

#[tokio::test]
async fn client_timeouts_are_typed_and_censored() {
    for yahoo in [false, true] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(5)))
            .mount(&server)
            .await;
        let recorder = Arc::new(Mutex::new(Recorder::new()));
        let result = PROVIDER_RECORDER
            .scope(Arc::clone(&recorder), async {
                if yahoo {
                    let client = primp::Client::builder().no_proxy().build().unwrap();
                    request_yahoo_with_retries("test", || {
                        client.get(server.uri()).timeout(Duration::from_millis(20))
                    })
                    .await
                } else {
                    let client = reqwest::Client::builder().no_proxy().build().unwrap();
                    request_standard_with_retries(&client, Engine::Bing, "test", || {
                        client.get(server.uri()).timeout(Duration::from_millis(20))
                    })
                    .await
                }
            })
            .await;
        assert!(result.is_err());
        let snapshot = recorder.lock().unwrap().finish(false);
        assert_eq!(snapshot.attempts.len(), 3);
        assert!(
            snapshot
                .attempts
                .iter()
                .all(|a| a.transport_error == Some(TransportKind::Timeout))
        );
        let sends: Vec<_> = snapshot
            .intervals
            .iter()
            .filter(|i| i.phase == Phase::Send)
            .collect();
        assert_eq!(sends.len(), 3);
        assert!(sends.iter().all(|i| i.censored));
    }
}

#[tokio::test]
async fn deadline_during_body_preserves_headers_and_censors_only_active_phase() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for yahoo in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(stream.read(&mut request).await.unwrap() > 0);
            stream
                .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 100\r\n\r\n")
                .await
                .unwrap();
            std::future::pending::<()>().await;
        });
        let directory = tempfile::tempdir().unwrap();
        TEST_TRACE_DIRECTORY
            .scope(Some(directory.path().to_owned()), async {
                let provider = async {
                    request(yahoo, &url).await?;
                    unreachable!()
                };
                let result = run_one_job(
                    "test",
                    Engine::Bing,
                    Arc::new(Semaphore::new(1)),
                    Arc::new(Mutex::new(Vec::new())),
                    Some(tokio::time::Instant::now() + Duration::from_millis(200)),
                    None,
                    provider,
                )
                .await;
                assert!(result.is_err());
            })
            .await;
        server.abort();
        let records = json_files(directory.path());
        let lifecycle = &records[0]["lifecycle"];
        assert_eq!(lifecycle["logical_outcome"], "deadline");
        assert_eq!(lifecycle["cancellation_phase"], "body");
        assert_eq!(lifecycle["attempts"][0]["http_status"], 403);
        assert_eq!(lifecycle["attempts"][0]["challenge"], "unknown");
        assert_eq!(lifecycle["attempts"][0]["outcome"], "cancelled");
        let intervals = lifecycle["intervals"].as_array().unwrap();
        assert_eq!(intervals.last().unwrap()["phase"], "body");
        assert_eq!(intervals.last().unwrap()["censored"], true);
        assert!(
            intervals[..intervals.len() - 1]
                .iter()
                .all(|i| i["censored"] == false)
        );
    }
}

#[test]
fn tls_classification_uses_backend_types() {
    assert!(is_tls_error(&rustls::Error::General("test".into())));
    assert!(is_tls_error(&primp_tls::Error::General("test".into())));
    assert!(is_tls_error(&std::io::Error::other(
        rustls::Error::General("wrapped".into())
    )));
    assert!(is_tls_error(&std::io::Error::other(
        primp_tls::Error::General("wrapped".into())
    )));
    assert!(!is_tls_error(&std::io::Error::other(
        "TLS-looking text is not typed evidence"
    )));
}

#[tokio::test]
async fn opt_out_creates_no_raw_capture_and_redirect_counts_are_explicit() {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::path("/start"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", format!("{}/final", server.uri())),
        )
        .mount(&server)
        .await;
    Mock::given(wiremock::matchers::path("/final"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;
    for yahoo in [false, true] {
        let (result, snapshot) = TEST_TRACE_DIRECTORY
            .scope(None, recorded(yahoo, &format!("{}/start", server.uri())))
            .await;
        assert!(result.is_ok());
        assert_eq!(snapshot.send_attempts, 1);
        assert_eq!(snapshot.attempts[0].http_status, Some(200));
        assert_eq!(snapshot.count_unit, "application_send");
        assert!(!snapshot.redirect_hops_observed);
    }
    assert_eq!(server.received_requests().await.unwrap().len(), 4);
}

#[tokio::test]
async fn quorum_during_real_retry_backoff_does_not_cancel_completed_response() {
    for yahoo in [false, true] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500).set_body_string("failed"))
            .expect(1)
            .mount(&server)
            .await;
        let directory = tempfile::tempdir().unwrap();
        let notify = Arc::new(tokio::sync::Notify::new());
        TEST_TRACE_DIRECTORY
            .scope(
                Some(directory.path().to_owned()),
                TEST_RETRY_DELAY.scope(
                    Duration::from_secs(60),
                    TEST_BACKOFF_ENTERED.scope(Arc::clone(&notify), async {
                        let diagnostics = Arc::new(Mutex::new(Vec::new()));
                        let signal = Arc::new(AtomicU8::new(0));
                        let slow = run_one_job(
                            "test",
                            Engine::Bing,
                            Arc::new(Semaphore::new(1)),
                            Arc::clone(&diagnostics),
                            None,
                            Some(Arc::clone(&signal)),
                            async {
                                request(yahoo, &server.uri()).await?;
                                unreachable!()
                            },
                        );
                        let pending = FuturesUnordered::new();
                        type Job<'a> = std::pin::Pin<
                            Box<
                                dyn Future<
                                        Output = (usize, Result<Vec<SearchResult>, KestrelError>),
                                    > + 'a,
                            >,
                        >;
                        pending.push(Box::pin(async { (0_usize, slow.await) }) as Job<'_>);
                        pending.push(Box::pin(async {
                            notify.notified().await;
                            (
                                1_usize,
                                Ok(vec![SearchResult::parsed(
                                    "ok".into(),
                                    "https://example.org".into(),
                                    String::new(),
                                    String::new(),
                                )]),
                            )
                        }) as Job<'_>);
                        let (_, cancelled) = tokio::time::timeout(
                            Duration::from_secs(3),
                            collect_fanout_signalled(pending, Some(1), None, Some(signal)),
                        )
                        .await
                        .unwrap();
                        assert_eq!(cancelled, 1);
                        let entries = diagnostics.lock().unwrap();
                        assert_eq!(entries[0].outcome, "cancelled_quorum");
                        assert_eq!(entries[0].retries, 0);
                    }),
                ),
            )
            .await;
        let records = json_files(directory.path());
        let lifecycle = &records
            .iter()
            .find(|r| r.get("lifecycle").is_some())
            .unwrap()["lifecycle"];
        assert_eq!(lifecycle["cancellation_phase"], "backoff");
        assert_eq!(lifecycle["send_attempts"], 1);
        assert_eq!(lifecycle["attempts"][0]["http_status"], 500);
        assert_eq!(lifecycle["attempts"][0]["outcome"], "response");
    }
}

#[tokio::test]
async fn successful_http_challenge_is_one_logical_failure_without_changing_result_schema() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string("<form id='captcha'>challenge</form>"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let directory = tempfile::tempdir().unwrap();
    TEST_TRACE_DIRECTORY
        .scope(Some(directory.path().to_owned()), async {
            let diagnostics = Arc::new(Mutex::new(Vec::new()));
            let result = run_one_job(
                "test",
                Engine::Bing,
                Arc::new(Semaphore::new(1)),
                Arc::clone(&diagnostics),
                None,
                None,
                async {
                    let (text, retries) = request(false, &server.uri()).await?;
                    Ok(ProviderResponse {
                        results: parse_provider_response(Engine::Bing, &text)?,
                        retries,
                        raw_result_count: 0,
                    })
                },
            )
            .await;
            assert!(result.is_err());
            let entries = diagnostics.lock().unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].outcome, "challenge");
            let normal = serde_json::to_value(&entries[0]).unwrap();
            assert_eq!(normal.as_object().unwrap().len(), 10);
            assert!(normal.get("lifecycle").is_none());
        })
        .await;
    let records = json_files(directory.path());
    let logical: Vec<_> = records
        .iter()
        .filter(|r| r.get("lifecycle").is_some())
        .collect();
    assert_eq!(logical.len(), 1);
    assert_eq!(logical[0]["lifecycle"]["logical_outcome"], "challenge");
    assert_eq!(
        logical[0]["lifecycle"]["attempts"][0]["outcome"],
        "response"
    );
    assert_eq!(logical[0]["lifecycle"]["attempts"][0]["http_status"], 200);
}

#[tokio::test]
async fn oversized_responses_preserve_status_and_stop_retries() {
    for yahoo in [false, true] {
        for status in [200, 500] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(status).set_body_bytes(vec![
                    b'x';
                    MAX_PROVIDER_RESPONSE_BYTES
                        + 1
                ]))
                .expect(1)
                .mount(&server)
                .await;
            let directory = tempfile::tempdir().unwrap();
            TEST_TRACE_DIRECTORY
                .scope(Some(directory.path().to_owned()), async {
                    let result = run_one_job(
                        "test",
                        if yahoo { Engine::Yahoo } else { Engine::Bing },
                        Arc::new(Semaphore::new(1)),
                        Arc::new(Mutex::new(Vec::new())),
                        None,
                        None,
                        async {
                            request(yahoo, &server.uri()).await?;
                            unreachable!()
                        },
                    )
                    .await;
                    assert!(matches!(
                        result,
                        Err(KestrelError::ProviderResponseTooLarge { .. })
                    ));
                })
                .await;
            let records = json_files(directory.path());
            assert_eq!(records.len(), 1, "oversized bodies must not be captured");
            let lifecycle = &records[0]["lifecycle"];
            assert_eq!(records[0]["outcome"], "response_too_large");
            assert_eq!(lifecycle["send_attempts"], 1);
            assert_eq!(lifecycle["attempts"][0]["http_status"], status);
            assert_eq!(lifecycle["attempts"][0]["outcome"], "response_too_large");
            assert_eq!(lifecycle["attempts"][0]["challenge"], "unknown");
            assert!(lifecycle["attempts"][0]["transport_error"].is_null());
            assert!(
                lifecycle["intervals"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|i| i["phase"] == "body" && i["censored"] == true)
            );
        }
    }
}

#[test]
fn mojeek_challenge_observation_matches_provider_parser() {
    assert_eq!(
        classify_challenge(
            Engine::Mojeek,
            include_str!("../../tests/fixtures/providers/mojeek-challenge.html")
        ),
        Challenge::Detected
    );
    assert_eq!(
        classify_challenge(
            Engine::Mojeek,
            include_str!("../../tests/fixtures/providers/mojeek.html")
        ),
        Challenge::NotDetected
    );
    assert_eq!(
        classify_challenge(
            Engine::Mojeek,
            "<html><head><title>Search</title></head><body><p>CAPTCHA results</p></body></html>"
        ),
        Challenge::NotDetected
    );
}
