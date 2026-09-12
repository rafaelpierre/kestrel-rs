use super::*;
use crate::model::SearchMode;
use std::pin::Pin;

type Job = Pin<Box<dyn Future<Output = (usize, Result<Vec<SearchResult>, KestrelError>)>>>;

fn results(paths: &[&str], engine: Engine) -> Vec<SearchResult> {
    with_provenance(
        paths
            .iter()
            .map(|path| {
                SearchResult::parsed(
                    (*path).into(),
                    format!("https://example.org/{path}"),
                    String::new(),
                    String::new(),
                )
            })
            .collect(),
        engine,
        "test",
    )
}

fn batch(index: usize, values: Vec<SearchResult>) -> Job {
    Box::pin(async move { (index, Ok(values)) })
}

#[tokio::test]
async fn unique_threshold_ignores_duplicates_and_preserves_fusion() {
    let pending = FuturesUnordered::<Job>::new();
    pending.push(batch(
        1,
        results(&["a", "a?utm_source=test", "b"], Engine::Bing),
    ));
    pending.push(batch(
        0,
        results(&["a#fragment", "c", "d", "e", "f"], Engine::Yahoo),
    ));
    pending.push(Box::pin(std::future::pending()));
    let signal = Arc::new(AtomicU8::new(FANOUT_RUNNING));
    let (outcomes, cancelled) =
        collect_fanout_signalled(pending, None, Some(5), Some(signal.clone())).await;
    assert_eq!(cancelled, 1);
    assert_eq!(signal.load(Ordering::Relaxed), FANOUT_MIN_RESULTS);
    let fused = merge_outcomes(outcomes).unwrap();
    assert_eq!(fused.len(), 6); // Keep whole batches, not just the first five.
    assert_eq!(fused[0].engine, Some(Engine::Yahoo)); // Stable configured order.
    assert_eq!(fused[0].sources.len(), 3);
}

#[tokio::test]
async fn minimum_overrides_quorum_without_allowing_quorum_to_stop_early() {
    for (first, second, minimum) in [
        (vec!["a", "b", "c", "d", "e"], vec!["f"], 5),
        (vec!["a"], vec!["b"], 5),
    ] {
        let pending = FuturesUnordered::<Job>::new();
        pending.push(batch(0, results(&first, Engine::Bing)));
        pending.push(batch(1, results(&second, Engine::Yahoo)));
        pending.push(batch(2, results(&["c", "d", "e"], Engine::Duckduckgo)));
        let (outcomes, cancelled) =
            collect_fanout_signalled(pending, Some(2), Some(minimum), None).await;
        if first.len() == 5 {
            assert_eq!(outcomes.len(), 1);
            assert_eq!(cancelled, 2);
        } else {
            assert_eq!(outcomes.len(), 3);
            assert_eq!(cancelled, 0);
        }
    }
}

#[tokio::test]
async fn exhausted_providers_return_partial_results_or_existing_failure() {
    for has_results in [false, true] {
        let pending = FuturesUnordered::<Job>::new();
        pending.push(Box::pin(async {
            (0, Err(KestrelError::Search("offline".into())))
        }));
        if has_results {
            pending.push(batch(1, results(&["a"], Engine::Bing)));
            pending.push(batch(2, vec![]));
        }
        let (outcomes, cancelled) = collect_fanout_signalled(pending, None, Some(5), None).await;
        assert_eq!(cancelled, 0);
        let fused = merge_outcomes(outcomes);
        if has_results {
            assert_eq!(fused.unwrap().len(), 1);
        } else {
            assert!(fused.is_err());
        }
    }
}

#[test]
fn minimum_validation_rejects_zero() {
    for (mode, minimum, valid) in [
        (SearchMode::Fanout, 0, false),
        (SearchMode::Fanout, 5, true),
    ] {
        let options = SearchOptions {
            mode,
            min_results: Some(minimum),
            ..SearchOptions::default()
        };
        assert_eq!(validate_request(&["test".into()], &options).is_ok(), valid);
    }
}

#[tokio::test]
async fn threshold_cancels_http2_body_without_closing_shared_connection() {
    use http_body_util::StreamBody;
    use hyper::{
        body::{Bytes, Frame},
        server::conn::http2,
        service::service_fn,
    };
    use hyper_util::rt::{TokioExecutor, TokioIo};
    use std::{convert::Infallible, sync::atomic::AtomicUsize};

    tokio::time::timeout(Duration::from_secs(5), async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let connections = Arc::new(AtomicUsize::new(0));
        let count = connections.clone();
        let server = tokio::spawn(async move {
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                count.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let service = service_fn(
                        |request: hyper::Request<hyper::body::Incoming>| async move {
                            let tail = if request.uri().path() == "/slow" {
                                futures_util::stream::pending().boxed()
                            } else {
                                futures_util::stream::empty().boxed()
                            };
                            let body = futures_util::stream::once(async {
                                Ok::<_, Infallible>(Frame::data(Bytes::from_static(b"ok")))
                            })
                            .chain(tail);
                            Ok::<_, Infallible>(hyper::Response::new(StreamBody::new(body)))
                        },
                    );
                    let _ = http2::Builder::new(TokioExecutor::new())
                        .serve_connection(TokioIo::new(socket), service)
                        .await;
                });
            }
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .http2_prior_knowledge()
            .build()
            .unwrap();
        let mut response = client.get(format!("{url}/slow")).send().await.unwrap();
        assert_eq!(response.version(), reqwest::Version::HTTP_2);
        assert_eq!(response.chunk().await.unwrap().unwrap(), "ok");
        let pending = FuturesUnordered::<Job>::new();
        pending.push(Box::pin(async move {
            response.text().await.unwrap();
            panic!("unfinished body must be cancelled")
        }));
        pending.push(batch(1, results(&["a", "b", "c", "d", "e"], Engine::Bing)));
        // An unrelated request already uses the same pool when the slow read is dropped.
        let unrelated = client.get(format!("{url}/ok")).send().await.unwrap();
        let (_, cancelled) = collect_fanout_signalled(pending, None, Some(5), None).await;
        assert_eq!(cancelled, 1);
        assert_eq!(unrelated.text().await.unwrap(), "ok");
        assert_eq!(
            client
                .get(format!("{url}/ok"))
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "ok"
        );
        assert_eq!(connections.load(Ordering::SeqCst), 1);
        server.abort();
    })
    .await
    .expect("HTTP/2 cancellation and reuse must finish promptly");
}
