use std::time::Duration;

use kestrelsearch::{
    FetchOptions, KestrelClient, KestrelError, PageCache, SearchOptions, TransportOptions,
    fetch_all, fetch_all_detailed, search_many, search_many_detailed,
};
use wiremock::MockServer;

fn invalid<T>(result: Result<T, KestrelError>) {
    assert!(matches!(result, Err(KestrelError::InvalidRequest(_))));
}

#[tokio::test]
async fn numeric_fetch_validation_precedes_requests_and_cached_results() {
    let server = MockServer::start().await;
    let urls = vec![server.uri()];
    let client = KestrelClient::new().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let cache = PageCache::new(directory.path(), Duration::from_secs(60)).unwrap();
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .insert_header("content-type", "text/html")
                .set_body_string(
                    "<main><p>Useful cached page content for numeric validation tests.</p></main>",
                ),
        )
        .expect(1)
        .mount(&server)
        .await;
    let primed = client
        .fetch_all_cached(&urls, &FetchOptions::default(), &cache, None)
        .await
        .unwrap();
    assert!(primed[0].is_some());
    server.verify().await;
    server.reset().await;
    let mut cases = Vec::new();
    for value in [0, tokio::sync::Semaphore::MAX_PERMITS + 1, usize::MAX] {
        cases.push(FetchOptions {
            max_concurrency: value,
            ..Default::default()
        });
        cases.push(FetchOptions {
            parse_concurrency: value,
            ..Default::default()
        });
    }
    for timeout in [Duration::ZERO, Duration::MAX] {
        cases.push(FetchOptions {
            timeout,
            ..Default::default()
        });
    }
    for options in cases {
        invalid(fetch_all(&urls, &options).await);
        invalid(fetch_all_detailed(&urls, &options).await);
        invalid(client.fetch_all(&urls, &options).await);
        invalid(
            client
                .fetch_all_with_budget(&urls, &options, Duration::from_secs(1))
                .await,
        );
        invalid(client.fetch_all_detailed(&[], &options, None).await);
        invalid(client.fetch_all_cached(&urls, &options, &cache, None).await);
        invalid(
            client
                .fetch_all_cached_detailed(&urls, &options, &cache, None)
                .await,
        );
    }
    for budget in [Duration::ZERO, Duration::MAX] {
        let options = FetchOptions::default();
        invalid(client.fetch_all_with_budget(&urls, &options, budget).await);
        invalid(
            client
                .fetch_all_detailed(&urls, &options, Some(budget))
                .await,
        );
        invalid(
            client
                .fetch_all_cached(&urls, &options, &cache, Some(budget))
                .await,
        );
        invalid(
            client
                .fetch_all_cached_detailed(&[], &options, &cache, Some(budget))
                .await,
        );
        invalid(client.warm_up_fetch(&urls, budget).await);
        invalid(
            client
                .warm_up_search(&[kestrelsearch::Engine::Bing], budget)
                .await,
        );
    }
    assert!(server.received_requests().await.unwrap().is_empty());
    let options = FetchOptions {
        max_concurrency: tokio::sync::Semaphore::MAX_PERMITS,
        parse_concurrency: tokio::sync::Semaphore::MAX_PERMITS,
        timeout: Duration::from_nanos(1),
        ..Default::default()
    };
    assert!(client.fetch_all(&[], &options).await.unwrap().is_empty());
}

#[tokio::test]
async fn numeric_search_validation_rejects_before_provider_execution() {
    let client = KestrelClient::new().unwrap();
    let queries = vec!["test".into()];
    let mut cases = Vec::new();
    for value in [0, tokio::sync::Semaphore::MAX_PERMITS + 1, usize::MAX] {
        cases.push(SearchOptions {
            max_concurrency: value,
            ..Default::default()
        });
    }
    for budget in [Duration::ZERO, Duration::MAX] {
        cases.push(SearchOptions {
            search_budget: Some(budget),
            ..Default::default()
        });
    }
    for options in cases {
        invalid(search_many(&queries, &options).await);
        invalid(search_many_detailed(&queries, &options).await);
        invalid(client.search_many(&queries, &options).await);
        invalid(client.search_many_detailed(&queries, &options).await);
    }
}

#[test]
fn numeric_transport_durations_are_checked() {
    for options in [
        TransportOptions {
            pool_idle_timeout: Duration::MAX,
            ..Default::default()
        },
        TransportOptions {
            connect_timeout: Duration::MAX,
            ..Default::default()
        },
        TransportOptions {
            http2_ping_timeout: Duration::MAX,
            ..Default::default()
        },
        TransportOptions {
            http2_ping_interval: Some(Duration::MAX),
            ..Default::default()
        },
        TransportOptions {
            dns_cache_ttl: Duration::MAX,
            ..Default::default()
        },
    ] {
        invalid(KestrelClient::with_transport(options));
    }
}
