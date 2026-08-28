use std::time::{Duration, Instant};

use kestrelsearch::{
    FetchOptions, FetchOutcome, KestrelClient, PageCache, fetch_all, fetch_all_detailed,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const PAGE: &str = "<main><h1>Kestrel heading</h1><p>This is meaningful page content that should be retained by extraction.</p></main>";

#[tokio::test]
async fn fetches_parses_and_preserves_url_order() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/one"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html")
                .set_body_string(PAGE),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/two"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let results = fetch_all(
        &[
            format!("{}/one", server.uri()),
            format!("{}/two", server.uri()),
        ],
        &FetchOptions::default(),
    )
    .await
    .unwrap();
    assert!(
        results[0]
            .as_deref()
            .is_some_and(|content| content.contains("meaningful page content"))
    );
    assert_eq!(results[1], None);
}

#[tokio::test]
async fn detailed_fetch_reports_transfer_and_phase_metadata() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/observed"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html")
                .set_body_string(PAGE),
        )
        .mount(&server)
        .await;
    let url = format!("{}/observed", server.uri());
    let report = fetch_all_detailed(std::slice::from_ref(&url), &FetchOptions::default())
        .await
        .unwrap();
    assert!(report.contents[0].is_some());
    assert_eq!(report.pages.len(), 1);
    assert_eq!(report.pages[0].url, url);
    assert_eq!(report.pages[0].outcome, FetchOutcome::Success);
    assert_eq!(report.pages[0].response_bytes, PAGE.len());
    assert!(!report.budget_exhausted);
    assert_eq!(report.cancelled, 0);
    assert_eq!(report.cache_misses, 1);
}

#[tokio::test]
async fn rejects_unsupported_and_declared_oversized_responses() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/binary"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/octet-stream")
                .set_body_bytes(b"not html"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/large"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html")
                .insert_header("content-length", "101")
                .set_body_bytes(vec![b'x'; 101]),
        )
        .mount(&server)
        .await;
    let options = FetchOptions {
        max_response_bytes: 100,
        ..FetchOptions::default()
    };
    let results = fetch_all(
        &[
            format!("{}/binary", server.uri()),
            format!("{}/large", server.uri()),
        ],
        &options,
    )
    .await
    .unwrap();
    assert_eq!(results, [None, None]);
}

#[tokio::test]
async fn reusable_client_fetches_across_multiple_calls() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/reused"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html; charset=utf-8")
                .set_body_string(PAGE),
        )
        .expect(2)
        .mount(&server)
        .await;
    let client = KestrelClient::new().unwrap();
    let urls = [format!("{}/reused", server.uri())];
    for _ in 0..2 {
        let results = client
            .fetch_all(&urls, &FetchOptions::default())
            .await
            .unwrap();
        assert!(results[0].is_some());
    }
}

#[tokio::test]
async fn fetch_budget_retains_results_completed_before_deadline() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/fast"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html")
                .set_body_string(PAGE),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/slow"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html")
                .set_body_string(PAGE)
                .set_delay(Duration::from_millis(300)),
        )
        .mount(&server)
        .await;
    let client = KestrelClient::new().unwrap();
    let started = Instant::now();
    let report = client
        .fetch_all_detailed(
            &[
                format!("{}/fast", server.uri()),
                format!("{}/slow", server.uri()),
            ],
            &FetchOptions::default(),
            Some(Duration::from_millis(100)),
        )
        .await
        .unwrap();
    let results = report.contents;
    assert!(results[0].is_some());
    assert_eq!(results[1], None);
    assert!(report.budget_exhausted);
    assert_eq!(report.cancelled, 1);
    assert!(started.elapsed() < Duration::from_millis(250));
}

#[tokio::test]
async fn page_cache_avoids_a_second_network_fetch() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/cached"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html")
                .set_body_string(PAGE),
        )
        .expect(1)
        .mount(&server)
        .await;
    let directory = tempfile::tempdir().unwrap();
    let cache = PageCache::new(directory.path(), Duration::from_secs(60)).unwrap();
    let client = KestrelClient::new().unwrap();
    let urls = [format!("{}/cached", server.uri())];
    for expected_hits in [0, 1] {
        let report = client
            .fetch_all_cached_detailed(&urls, &FetchOptions::default(), &cache, None)
            .await
            .unwrap();
        assert!(report.contents[0].is_some());
        assert_eq!(report.cache_hits, expected_hits);
        assert_eq!(report.cache_misses, 1 - expected_hits);
        if expected_hits == 1 {
            assert_eq!(report.pages[0].outcome, FetchOutcome::CacheHit);
        }
    }
}
