use super::*;
use http_body_util::StreamBody;
use hyper::{
    body::{Bytes, Frame},
    server::conn::{http1, http2},
    service::service_fn,
};
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::{convert::Infallible, pin::Pin, sync::atomic::AtomicUsize};

type Job<'a> = Pin<Box<dyn Future<Output = (usize, Result<Vec<SearchResult>, KestrelError>)> + 'a>>;

fn cards(engine: Engine, count: usize) -> String {
    (0..count).map(|i| {
        if engine == Engine::Yahoo {
            format!(r#"<div class="dd algo"><div class="compTitle"><h3><a href="https://example.org/{engine}/{i}">Café {i}</a></h3></div><div class="compText"><p>Complete snippet {i}.</p></div></div>"#)
        } else {
            format!(r#"<li class="b_algo"><h2><a href="https://example.org/{engine}/{i}">Café {i}</a></h2><div class="b_caption"><p>Complete snippet {i}.</p></div></li>"#)
        }
    }).collect()
}

struct Server {
    url: String,
    count: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn server(h2: bool, compressed: bool) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let count = Arc::new(AtomicUsize::new(0));
    let counter = count.clone();
    let task = tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            counter.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let service = service_fn(
                    move |request: hyper::Request<hyper::body::Incoming>| async move {
                        let slow = request.uri().path() != "/ok";
                        let engine = if request.uri().path() == "/yahoo" {
                            Engine::Yahoo
                        } else {
                            Engine::Bing
                        };
                        let text = if request.uri().path() == "/pool" {
                            cards(engine, 15)
                        } else if slow {
                            cards(engine, 3)
                        } else {
                            "ok".into()
                        };
                        let mut payload = text.into_bytes();
                        if compressed && slow {
                            use std::io::Write;
                            let mut encoder = flate2::write::GzEncoder::new(
                                Vec::new(),
                                flate2::Compression::default(),
                            );
                            encoder.write_all(&payload).unwrap();
                            encoder.flush().unwrap();
                            // No gzip footer/EOF arrives: records must be emitted before then.
                            payload = encoder.get_ref().clone();
                        }
                        let tail = if slow {
                            futures_util::stream::pending().boxed()
                        } else {
                            futures_util::stream::empty().boxed()
                        };
                        // Split records and UTF-8 without triggering H2 tiny-frame flood limits.
                        let chunks = payload
                            .chunks(17)
                            .map(|b| Ok::<_, Infallible>(Frame::data(Bytes::copy_from_slice(b))))
                            .collect::<Vec<_>>();
                        let body = futures_util::stream::iter(chunks).chain(tail);
                        let mut response = hyper::Response::new(StreamBody::new(body));
                        response
                            .headers_mut()
                            .insert("content-type", "text/html; charset=utf-8".parse().unwrap());
                        if compressed && slow {
                            response
                                .headers_mut()
                                .insert("content-encoding", "gzip".parse().unwrap());
                        }
                        Ok::<_, Infallible>(response)
                    },
                );
                if h2 {
                    let _ = http2::Builder::new(TokioExecutor::new())
                        .serve_connection(TokioIo::new(socket), service)
                        .await;
                } else {
                    let _ = http1::Builder::new()
                        .serve_connection(TokioIo::new(socket), service)
                        .await;
                }
            });
        }
    });
    Server { url, count, task }
}

#[tokio::test]
async fn both_clients_stream_normalized_results_and_cancel_unfinished_bodies() {
    let _telemetry = crate::telemetry::test_export_guard();
    for h2 in [false, true] {
        for compressed in [false, true] {
            tokio::time::timeout(Duration::from_secs(5), async {
                let server = server(h2, compressed).await;
                let mut standard = reqwest::Client::builder().no_proxy().gzip(true);
                let mut yahoo = primp::Client::builder().no_proxy().gzip(true);
                if h2 {
                    standard = standard.http2_prior_knowledge();
                    yahoo = yahoo.http2_prior_knowledge();
                }
                let clients = SearchClients {
                    parsers: parsing::ParserPool::default(),
                    standard: standard.build().unwrap(),
                    yahoo: Some(yahoo.build().unwrap()),
                };
                // Prewarm both pools, then create unrelated in-flight responses.
                assert_eq!(
                    clients
                        .standard
                        .get(format!("{}/ok", server.url))
                        .send()
                        .await
                        .unwrap()
                        .text()
                        .await
                        .unwrap(),
                    "ok"
                );
                assert_eq!(
                    clients
                        .yahoo
                        .as_ref()
                        .unwrap()
                        .get(format!("{}/ok", server.url))
                        .send()
                        .await
                        .unwrap()
                        .text()
                        .await
                        .unwrap(),
                    "ok"
                );
                let a = clients
                    .standard
                    .get(format!("{}/ok", server.url))
                    .send()
                    .await
                    .unwrap();
                let b = clients
                    .yahoo
                    .as_ref()
                    .unwrap()
                    .get(format!("{}/ok", server.url))
                    .send()
                    .await
                    .unwrap();
                let (sender, receiver) = mpsc::channel(1);
                let pending = FuturesUnordered::<Job<'_>>::new();
                let diagnostics = Arc::new(Mutex::new(Vec::new()));
                let signal = Arc::new(AtomicU8::new(FANOUT_RUNNING));
                let semaphore = Arc::new(Semaphore::new(2));
                for (index, engine) in [Engine::Bing, Engine::Yahoo].into_iter().enumerate() {
                    let endpoint = format!("{}/{}", server.url, engine);
                    let publisher = Publisher {
                        sender: sender.clone(),
                        index,
                        engine,
                        query: "site:example.org test".into(),
                    };
                    let clients = &clients;
                    let job = run_one_job(
                        "site:example.org test",
                        engine,
                        semaphore.clone(),
                        diagnostics.clone(),
                        None,
                        Some(signal.clone()),
                        async move {
                            let (text, retries) = if engine == Engine::Yahoo {
                                request_yahoo_with_retries("test", retain_body, || {
                                    clients.yahoo.as_ref().unwrap().get(&endpoint)
                                })
                                .await?
                            } else {
                                request_standard_with_retries(
                                    &clients.standard,
                                    engine,
                                    "test",
                                    retain_body,
                                    || clients.standard.get(&endpoint),
                                )
                                .await?
                            };
                            Ok(ProviderResponse {
                                results: parse_provider_response(engine, &text)?,
                                retries,
                                raw_result_count: 0,
                            })
                        },
                    );
                    pending.push(Box::pin(async move {
                        (index, PUBLISHER.scope(publisher, job).await)
                    }));
                }
                drop(sender);
                let (outcomes, cancelled) = collect(pending, 5, Some(signal), Some(receiver)).await;
                let results = merge_outcomes(outcomes).unwrap();
                assert!(results.len() >= 5);
                assert_eq!(cancelled, 2); // Neither response body ever ended.
                for result in results {
                    assert!(result.title.starts_with("Café"));
                    assert!(result.snippet.starts_with("Complete snippet"));
                    assert!(result.engine.is_some());
                    assert_eq!(result.query.as_deref(), Some("site:example.org test"));
                    assert!(result.engine_rank.unwrap() <= 3);
                    assert_eq!(result.sources.len(), 1);
                }
                for entry in diagnostics.lock().unwrap().iter() {
                    assert_eq!(entry.outcome, "cancelled_min_results");
                    assert!(entry.result_count > 0);
                }
                assert_eq!(a.text().await.unwrap(), "ok");
                assert_eq!(b.text().await.unwrap(), "ok");
                assert_eq!(
                    clients
                        .standard
                        .get(format!("{}/ok", server.url))
                        .send()
                        .await
                        .unwrap()
                        .text()
                        .await
                        .unwrap(),
                    "ok"
                );
                assert_eq!(
                    clients
                        .yahoo
                        .as_ref()
                        .unwrap()
                        .get(format!("{}/ok", server.url))
                        .send()
                        .await
                        .unwrap()
                        .text()
                        .await
                        .unwrap(),
                    "ok"
                );
                if h2 {
                    assert_eq!(server.count.load(Ordering::SeqCst), 2);
                }
            })
            .await
            .unwrap_or_else(|_| panic!("streaming stalled: h2={h2}, gzip={compressed}"));
        }
    }
}

#[tokio::test]
async fn failed_provider_retracts_partial_results_and_does_not_satisfy_minimum() {
    let _telemetry = crate::telemetry::test_export_guard();
    let (sender, receiver) = mpsc::channel(1);
    let pending = FuturesUnordered::<Job<'_>>::new();
    pending.push(Box::pin(async move {
        let (resume, acknowledged) = oneshot::channel();
        sender
            .send(Batch {
                index: 0,
                results: vec![SearchResult::parsed(
                    "bad".into(),
                    "https://bad.example".into(),
                    String::new(),
                    String::new(),
                )],
                resume,
            })
            .await
            .unwrap();
        acknowledged.await.unwrap();
        (
            0,
            Err(KestrelError::Search(
                "malformed tail before threshold".into(),
            )),
        )
    }));
    let (outcomes, cancelled) = collect(pending, 5, None, Some(receiver)).await;
    assert_eq!(cancelled, 0);
    assert!(merge_outcomes(outcomes).is_err());
}

#[tokio::test]
async fn deadline_keeps_closed_records_and_releases_provider_permits() {
    let _telemetry = crate::telemetry::test_export_guard();
    let (sender, receiver) = mpsc::channel(1);
    let pending = FuturesUnordered::<Job<'_>>::new();
    let diagnostics = Arc::new(Mutex::new(Vec::new()));
    let semaphore = Arc::new(Semaphore::new(1));
    let publisher = Publisher {
        sender,
        index: 0,
        engine: Engine::Bing,
        query: "test".into(),
    };
    let job = run_one_job(
        "test",
        Engine::Bing,
        semaphore.clone(),
        diagnostics.clone(),
        Some(tokio::time::Instant::now() + Duration::from_millis(50)),
        None,
        async {
            let body = ProviderBody::new(Engine::Bing, 200, None, None).unwrap();
            let mut incremental = Incremental::for_body(&body).unwrap();
            // One byte at a time verifies the decoder independently of H2 frame limits.
            for byte in cards(Engine::Bing, 1).bytes() {
                incremental.push(&[byte]).await.unwrap();
            }
            std::future::pending::<Result<ProviderResponse, ProviderFailure>>().await
        },
    );
    pending.push(Box::pin(async move {
        (0, PUBLISHER.scope(publisher, job).await)
    }));
    let (outcomes, _) = collect(pending, 5, None, Some(receiver)).await;
    let results = merge_outcomes(outcomes).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Café 0");
    assert_eq!(semaphore.available_permits(), 1);
    assert_eq!(diagnostics.lock().unwrap()[0].outcome, "deadline");
}

#[tokio::test]
async fn simultaneous_queries_have_independent_thresholds_and_provenance() {
    let _telemetry = crate::telemetry::test_export_guard();
    let query = |query: &'static str, count| async move {
        let (sender, receiver) = mpsc::channel(1);
        let pending = FuturesUnordered::<Job<'_>>::new();
        let publisher = Publisher {
            sender,
            index: 0,
            engine: Engine::Bing,
            query: query.into(),
        };
        pending.push(Box::pin(async move {
            let future = async {
                let body = ProviderBody::new(Engine::Bing, 200, None, None).unwrap();
                let mut incremental = Incremental::for_body(&body).unwrap();
                incremental
                    .push(cards(Engine::Bing, count).as_bytes())
                    .await
                    .unwrap();
                std::future::pending().await
            };
            (0, PUBLISHER.scope(publisher, future).await)
        }));
        let (outcomes, cancelled) = collect(pending, count, None, Some(receiver)).await;
        assert_eq!(cancelled, 1);
        let results = merge_outcomes(outcomes).unwrap();
        assert_eq!(results.len(), count);
        assert!(results.iter().all(|r| r.query.as_deref() == Some(query)));
    };
    tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(query("one", 1), query("five", 5));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn failures_empty_responses_and_filtered_records_never_reach_the_minimum() {
    let _telemetry = crate::telemetry::test_export_guard();
    let (sender, receiver) = mpsc::channel(1);
    let pending = FuturesUnordered::<Job<'_>>::new();
    pending.push(Box::pin(async {
        (0, Err(KestrelError::Search("HTTP 403 challenge".into())))
    }));
    pending.push(Box::pin(async { (1, Ok(Vec::new())) }));
    let publisher = Publisher {
        sender,
        index: 2,
        engine: Engine::Bing,
        query: "site:allowed.example".into(),
    };
    pending.push(Box::pin(async move {
        let future = async {
            let body = ProviderBody::new(Engine::Bing, 200, None, None).unwrap();
            let mut incremental = Incremental::for_body(&body).unwrap();
            incremental
                .push(cards(Engine::Bing, 5).as_bytes())
                .await
                .unwrap();
            assert!(incremental.previous.is_empty());
            (2, Ok(Vec::new()))
        };
        PUBLISHER.scope(publisher, future).await
    }));
    let signal = Arc::new(AtomicU8::new(FANOUT_RUNNING));
    let (outcomes, cancelled) = collect(pending, 5, Some(signal.clone()), Some(receiver)).await;
    assert_eq!(cancelled, 0);
    assert_eq!(signal.load(Ordering::Relaxed), FANOUT_RUNNING);
    assert!(merge_outcomes(outcomes).unwrap().is_empty());
    for status in [403, 429, 500] {
        let publisher = Publisher {
            sender: mpsc::channel(1).0,
            index: 0,
            engine: Engine::Bing,
            query: "test".into(),
        };
        PUBLISHER
            .scope(publisher, async {
                let body = ProviderBody::new(Engine::Bing, status, None, None).unwrap();
                assert!(Incremental::for_body(&body).is_none());
            })
            .await;
    }
}

#[tokio::test]
async fn default_query_mode_counts_metadata_without_query_terms_and_cancels_stragglers() {
    let _telemetry = crate::telemetry::test_export_guard();
    let (sender, receiver) = mpsc::channel(1);
    let pending = FuturesUnordered::<Job<'_>>::new();
    let publisher = Publisher {
        sender,
        index: 0,
        engine: Engine::Bing,
        query: "machine learning".into(),
    };
    pending.push(Box::pin(async move {
        let future = async {
            let body = ProviderBody::new(Engine::Bing, 200, None, None).unwrap();
            let mut incremental = Incremental::for_body(&body).unwrap();
            incremental
                .push(cards(Engine::Bing, 5).as_bytes())
                .await
                .unwrap();
            // Even the contributing provider's body need not reach EOF.
            std::future::pending().await
        };
        (0, PUBLISHER.scope(publisher, future).await)
    }));
    pending.push(Box::pin(std::future::pending()));
    let signal = Arc::new(AtomicU8::new(FANOUT_RUNNING));
    let (outcomes, cancelled) = tokio::time::timeout(
        Duration::from_secs(1),
        collect(pending, 5, Some(signal.clone()), Some(receiver)),
    )
    .await
    .expect("five unique records must cancel both pending bodies without a second provider");
    let results = merge_outcomes(outcomes).unwrap();
    assert_eq!(results.len(), 5);
    assert!(results.iter().all(|r| r.engine == Some(Engine::Bing)));
    assert_eq!(cancelled, 2);
    assert_eq!(signal.load(Ordering::Relaxed), FANOUT_MIN_RESULTS);
}

#[tokio::test]
async fn site_constraints_filter_streamed_records_before_counting() {
    let _telemetry = crate::telemetry::test_export_guard();
    let (sender, mut receiver) = mpsc::channel(1);
    let publisher = Publisher {
        sender,
        index: 0,
        engine: Engine::Bing,
        query: "site:other.example.com".into(),
    };
    PUBLISHER
        .scope(publisher, async {
            let body = ProviderBody::new(Engine::Bing, 200, None, None).unwrap();
            let mut incremental = Incremental::for_body(&body).unwrap();
            incremental
                .push(cards(Engine::Bing, 5).as_bytes())
                .await
                .unwrap();
            assert!(incremental.previous.is_empty());
            assert!(receiver.try_recv().is_err());
        })
        .await;
}

#[tokio::test]
async fn benchmark_minimum_arms_and_fixed_pool_exercise_larger_fetch_caps() {
    let _telemetry = crate::telemetry::test_export_guard();
    use crate::{FetchOptions, fetcher::fetch_all_reusing_client};
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

    tokio::time::timeout(Duration::from_secs(10), async {
        let provider = server(false, false).await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let mut sizes = Vec::new();
        for minimum in [1, 5, 15] {
            let (sender, receiver) = mpsc::channel(1);
            let publisher = Publisher {
                sender,
                index: 0,
                engine: Engine::Bing,
                query: "Café".into(),
            };
            let endpoint = format!("{}/pool", provider.url);
            let pending = FuturesUnordered::<Job<'_>>::new();
            let job = async {
                let (body, _) = request_standard_with_retries(
                    &client, Engine::Bing, "Café", retain_body, || client.get(&endpoint),
                ).await?;
                parse_provider_response(Engine::Bing, &body)
            };
            pending.push(Box::pin(async move { (0, PUBLISHER.scope(publisher, job).await.map_err(ProviderFailure::into_public)) }));
            let (outcomes, cancelled) = collect(pending, minimum, None, Some(receiver)).await;
            let pool = merge_outcomes(outcomes).unwrap();
            assert_eq!(cancelled, 1); // The response deliberately never reaches EOF.
            assert!(pool.len() >= minimum);
            sizes.push(pool.len());
            if minimum == 15 {
                assert_eq!(pool.len(), 15);
                let pages = MockServer::start().await;
                Mock::given(method("GET"))
                    .respond_with(ResponseTemplate::new(200)
                        .insert_header("content-type", "text/html")
                        .set_body_string("<main><h1>Café</h1><p>Complete fixture evidence for the fixed candidate pool and benchmark fetch cap validation.</p></main>"))
                    .mount(&pages).await;
                // Reuse the same collected order and local page content in all cap arms.
                let urls: Vec<_> = pool.iter().map(|r| {
                    format!("{}{}", pages.uri(), url::Url::parse(&r.url).unwrap().path())
                }).collect();
                for cap in [5, 10, 15] {
                    let before = pages.received_requests().await.unwrap().len();
                    let content = fetch_all_reusing_client(&urls[..cap], &FetchOptions::default(), &client).await.unwrap();
                    assert_eq!(pages.received_requests().await.unwrap().len() - before, cap);
                    assert_eq!(content.iter().filter(|c| c.as_deref().is_some_and(|s| s.contains("Complete fixture evidence"))).count(), cap);
                }
            }
        }
        assert!(sizes[0] < sizes[1] && sizes[1] < sizes[2], "{sizes:?}");
    }).await.expect("local benchmark fixtures must finish promptly");
}

#[tokio::test]
async fn records_commit_before_eof_and_survive_caller_cancellation() {
    let _telemetry = crate::telemetry::test_export_guard();
    let server = server(false, false).await;
    let directory = tempfile::tempdir().unwrap();
    let store = crate::SearchRecovery::new(directory.path(), Duration::from_secs(60)).unwrap();
    let key = crate::recovery::UnitKey::new("fixture", Engine::Bing, "", TimeFilter::Any);
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let (sender, receiver) = mpsc::channel(1);
    let publisher = Publisher {
        sender,
        index: 0,
        engine: Engine::Bing,
        query: "fixture".into(),
    };
    let pending = FuturesUnordered::<Job<'_>>::new();
    let endpoint = format!("{}/bing", server.url);
    pending.push(Box::pin(async move {
        let result = PUBLISHER
            .scope(publisher, async {
                let (body, retries) = request_standard_with_retries(
                    &client,
                    Engine::Bing,
                    "fixture",
                    retain_body,
                    || client.get(&endpoint),
                )
                .await
                .map_err(ProviderFailure::into_public)?;
                let _ = retries;
                Ok(with_provenance(
                    parse_provider_response(Engine::Bing, &body)
                        .map_err(ProviderFailure::into_public)?,
                    Engine::Bing,
                    "fixture",
                ))
            })
            .await;
        (0, result)
    }));
    let (queue, writer) = crate::recovery::writer(Some(&store), None);
    let collect = collect_recording(
        pending,
        100,
        None,
        Some(receiver),
        Some((queue.unwrap(), vec![key.clone()])),
        None,
    );
    let running = async { tokio::join!(collect, writer) };
    let observe = async {
        loop {
            if let Some(snapshot) = store.load(&key).await {
                assert_eq!(snapshot.state, crate::recovery::State::Incomplete);
                if snapshot.records.len() == 3 {
                    break;
                }
            }
            tokio::task::yield_now().await;
        }
    };
    tokio::time::timeout(Duration::from_secs(5),async {
        tokio::select! { _ = running => panic!("provider reached EOF unexpectedly"), () = observe => () }
    }).await.unwrap();
    // The losing operation future was dropped; its acknowledged snapshot remains.
    let snapshot = store.load(&key).await.unwrap();
    assert_eq!(snapshot.records.len(), 3);
    assert_eq!(snapshot.state, crate::recovery::State::Incomplete);
    assert_eq!(server.count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
#[ignore = "fresh-process helper for committed_provider_snapshot_survives_kill"]
async fn provider_recording_child() {
    let directory = std::env::var_os("KESTREL_PROVIDER_STORE").unwrap();
    let endpoint = std::env::var("KESTREL_PROVIDER_ENDPOINT").unwrap();
    let store = crate::SearchRecovery::new(directory, Duration::from_secs(60)).unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let (sender, receiver) = mpsc::channel(1);
    let publisher = Publisher {
        sender,
        index: 0,
        engine: Engine::Bing,
        query: "fixture".into(),
    };
    let pending = FuturesUnordered::<Job<'_>>::new();
    pending.push(Box::pin(async move {
        let outcome = PUBLISHER
            .scope(publisher, async {
                let (body, _) = request_standard_with_retries(
                    &client,
                    Engine::Bing,
                    "fixture",
                    retain_body,
                    || client.get(&endpoint),
                )
                .await
                .map_err(ProviderFailure::into_public)?;
                Ok(with_provenance(
                    parse_provider_response(Engine::Bing, &body)
                        .map_err(ProviderFailure::into_public)?,
                    Engine::Bing,
                    "fixture",
                ))
            })
            .await;
        (0, outcome)
    }));
    let key = crate::recovery::UnitKey::new("fixture", Engine::Bing, "", TimeFilter::Any);
    let (queue, writer) = crate::recovery::writer(Some(&store), None);
    let collect = collect_recording(
        pending,
        100,
        None,
        Some(receiver),
        Some((queue.unwrap(), vec![key])),
        None,
    );
    tokio::join!(collect, writer);
    panic!("provider must still be waiting for EOF");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn committed_provider_snapshot_survives_kill() {
    struct Process(std::process::Child);
    impl Drop for Process {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let server = server(false, false).await;
    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("child.log");
    let output = std::fs::File::create(&log).unwrap();
    let mut child = Process(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "search::streaming::tests::provider_recording_child",
                "--ignored",
                "--nocapture",
            ])
            .env("KESTREL_PROVIDER_STORE", directory.path())
            .env("KESTREL_PROVIDER_ENDPOINT", format!("{}/bing", server.url))
            .env("KESTRELSEARCH_OTEL_ENABLED", "false")
            .stdout(std::process::Stdio::from(output.try_clone().unwrap()))
            .stderr(std::process::Stdio::from(output))
            .spawn()
            .unwrap(),
    );
    let store = crate::SearchRecovery::new(directory.path(), Duration::from_secs(60)).unwrap();
    let key = crate::recovery::UnitKey::new("fixture", Engine::Bing, "", TimeFilter::Any);
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if store.load(&key).await.is_some_and(|s| s.records.len() == 3) {
                break;
            }
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "{}",
                std::fs::read_to_string(&log).unwrap()
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    child.0.kill().unwrap();
    assert!(!child.0.wait().unwrap().success());
    let fresh = crate::SearchRecovery::new(directory.path(), Duration::from_secs(60)).unwrap();
    let snapshot = fresh.load(&key).await.unwrap();
    assert_eq!(snapshot.state, crate::recovery::State::Incomplete);
    assert_eq!(snapshot.records.len(), 3);
    assert_eq!(snapshot.records[0].sources[0].query, "fixture");
    assert_eq!(server.count.load(Ordering::SeqCst), 1);
}
