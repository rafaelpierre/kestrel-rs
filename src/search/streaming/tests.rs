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
                        let text = if slow { cards(engine, 3) } else { "ok".into() };
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
                        query_syntax: QuerySyntax::Native,
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
                                request_yahoo_with_retries("test", || {
                                    clients.yahoo.as_ref().unwrap().get(&endpoint)
                                })
                                .await?
                            } else {
                                request_standard_with_retries(
                                    &clients.standard,
                                    engine,
                                    "test",
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
                let (outcomes, cancelled) =
                    collect(pending, Some(2), Some(5), Some(signal), Some(receiver)).await;
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
async fn failed_provider_retracts_partial_results_and_does_not_satisfy_quorum() {
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
    let (outcomes, cancelled) = collect(pending, None, Some(5), None, Some(receiver)).await;
    assert_eq!(cancelled, 0);
    assert!(merge_outcomes(outcomes).is_err());
}

#[tokio::test]
async fn deadline_keeps_closed_records_and_releases_provider_permits() {
    let (sender, receiver) = mpsc::channel(1);
    let pending = FuturesUnordered::<Job<'_>>::new();
    let diagnostics = Arc::new(Mutex::new(Vec::new()));
    let semaphore = Arc::new(Semaphore::new(1));
    let publisher = Publisher {
        sender,
        index: 0,
        engine: Engine::Bing,
        query: "test".into(),
        query_syntax: QuerySyntax::Native,
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
            std::future::pending::<Result<ProviderResponse, KestrelError>>().await
        },
    );
    pending.push(Box::pin(async move {
        (0, PUBLISHER.scope(publisher, job).await)
    }));
    let (outcomes, _) = collect(pending, None, Some(5), None, Some(receiver)).await;
    let results = merge_outcomes(outcomes).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Café 0");
    assert_eq!(semaphore.available_permits(), 1);
    assert_eq!(diagnostics.lock().unwrap()[0].outcome, "deadline");
}

#[tokio::test]
async fn simultaneous_queries_have_independent_thresholds_and_provenance() {
    let query = |query: &'static str, count| async move {
        let (sender, receiver) = mpsc::channel(1);
        let pending = FuturesUnordered::<Job<'_>>::new();
        let publisher = Publisher {
            sender,
            index: 0,
            engine: Engine::Bing,
            query: query.into(),
            query_syntax: QuerySyntax::Native,
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
        let (outcomes, cancelled) = collect(pending, None, Some(count), None, Some(receiver)).await;
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
        query_syntax: QuerySyntax::Native,
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
    let (outcomes, cancelled) =
        collect(pending, None, Some(5), Some(signal.clone()), Some(receiver)).await;
    assert_eq!(cancelled, 0);
    assert_eq!(signal.load(Ordering::Relaxed), FANOUT_RUNNING);
    assert!(merge_outcomes(outcomes).unwrap().is_empty());
    for status in [403, 429, 500] {
        let publisher = Publisher {
            sender: mpsc::channel(1).0,
            index: 0,
            engine: Engine::Bing,
            query: "test".into(),
            query_syntax: QuerySyntax::Native,
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
async fn minimum_cancels_without_waiting_for_requested_second_provider() {
    let (sender, receiver) = mpsc::channel(1);
    let pending = FuturesUnordered::<Job<'_>>::new();
    let publisher = Publisher {
        sender,
        index: 0,
        engine: Engine::Bing,
        query: "test".into(),
        query_syntax: QuerySyntax::Native,
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
        collect(
            pending,
            Some(2),
            Some(5),
            Some(signal.clone()),
            Some(receiver),
        ),
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
async fn portable_constraints_filter_streamed_records_before_counting() {
    let (sender, mut receiver) = mpsc::channel(1);
    let publisher = Publisher {
        sender,
        index: 0,
        engine: Engine::Bing,
        query: "requiredword".into(),
        query_syntax: QuerySyntax::Portable,
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
