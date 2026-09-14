#![cfg(feature = "test-fixtures")]
#![allow(deprecated)]
use assert_cmd::Command;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

#[tokio::test(flavor = "multi_thread")]
async fn deadline_then_success_keeps_json_and_does_not_retry_challenge() {
    let server = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    Mock::given(path("/bing")).respond_with(move |_: &wiremock::Request| {
        if seen.fetch_add(1,Ordering::SeqCst) == 0 {
            ResponseTemplate::new(200).set_delay(Duration::from_secs(60))
        } else {
            ResponseTemplate::new(200).set_body_string(r#"<ol id="b_results"><li class="b_algo"><h2><a href="https://example.com/rust">Rust ownership</a></h2><p>Rust ownership evidence</p></li></ol>"#)
        }
    }).mount(&server).await;
    Mock::given(path("/mojeek"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("fixtures/providers/mojeek-challenge.html")),
        )
        .expect(1)
        .mount(&server)
        .await;
    let output = Command::cargo_bin("kestrel")
        .unwrap()
        .env("KESTREL_TEST_PROVIDER_ENDPOINT", server.uri())
        .args([
            "search",
            "rust ownership",
            "--engine",
            "bing",
            "--engine",
            "mojeek",
            "--no-fetch",
            "--search-budget",
            "0.1",
            "--output",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let data: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(data["results"][0]["url"], "https://example.com/rust");
    assert_eq!(
        data["diagnostics"]["search"]["providers"][0]["retained_occurrences"],
        0
    );
    assert_eq!(
        data["diagnostics"]["search"]["providers"][2]["retained_occurrences"],
        1
    );
    assert_eq!(
        data["diagnostics"]["search"]["provider_outcomes"]["deadline"],
        1
    );
    assert_eq!(data["diagnostics"]["search"]["budget_exhausted"], false);
    assert_eq!(
        data["diagnostics"]["search"]["providers"][2]["discovery_attempt"],
        2
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("retry attempt 2/3"));
    assert!(stderr.contains("next budget 5.100s for 1"));
}
