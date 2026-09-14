#![cfg(feature = "test-fixtures")]

use serde_json::Value;
use std::{process::Command, time::Duration};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

#[tokio::test]
async fn search_default_budget_retains_fast_pages_and_explains_cancelled_fetches() {
    let server = MockServer::start().await;
    let endpoint = server.uri();
    let body = ["fast", "slow"].map(|name| format!(
        "<li class=\"b_algo\"><h2><a href=\"{endpoint}/{name}\">Page {name}</a></h2><div class=\"b_caption\"><p>Page snippet.</p></div></li>"
    )).join("");
    Mock::given(path("/bing"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;
    for (name, delay) in [("fast", 0), ("slow", 4)] {
        Mock::given(path(format!("/{name}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(format!("Page {name} evidence."), "text/plain")
                    .set_delay(Duration::from_secs(delay)),
            )
            .mount(&server)
            .await;
    }
    tokio::task::spawn_blocking(move || {
        for (format, extra, cancelled) in [
            ("json", vec![], true),
            ("text", vec![], true),
            ("json", vec!["--no-diagnostics"], true),
            ("json", vec!["--fetch-budget", "8"], false),
            ("json", vec!["--no-fetch"], false),
        ] {
            let output = Command::new(env!("CARGO_BIN_EXE_kestrel"))
                .args([
                    "search",
                    "page",
                    "-e",
                    "bing",
                    "--min-results",
                    "2",
                    "--search-budget",
                    "10",
                    "--no-rank",
                    "--output",
                    format,
                ])
                .args(&extra)
                .env("KESTREL_TEST_PROVIDER_ENDPOINT", &endpoint)
                .env("KESTRELSEARCH_OTEL_ENABLED", "false")
                .env_remove("KESTRELSEARCH_BENCHMARK_ARTIFACT_DIR")
                .env_remove("KESTRELSEARCH_BENCHMARK_RUN_ID")
                .output()
                .unwrap();
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert!(output.status.success(), "{stderr}");
            assert_eq!(
                stderr.contains("Fetch budget exhausted"),
                cancelled,
                "{stderr}"
            );
            if cancelled {
                assert!(
                    stderr.contains("(2s); cancelled 1 unfinished page fetch(es)"),
                    "{stderr}"
                );
                assert!(stderr.contains("kestrel fetch \"URL\""));
            }
            let stdout = String::from_utf8(output.stdout).unwrap();
            assert!(!stdout.contains("Fetch budget exhausted"));
            if format == "json" {
                let json: Value = serde_json::from_str(&stdout).unwrap();
                let results = json["results"].as_array().unwrap();
                assert_eq!(results.len(), 2);
                let no_fetch = extra.contains(&"--no-fetch");
                assert_eq!(results[0]["content"].is_string(), !no_fetch);
                assert_eq!(results[1]["content"].is_string(), !cancelled && !no_fetch);
                if !extra.contains(&"--no-diagnostics") {
                    assert_eq!(
                        json["diagnostics"]["evidence"]["budget_exhausted"],
                        cancelled
                    );
                    assert_eq!(
                        json["diagnostics"]["evidence"]["cancelled"],
                        usize::from(cancelled)
                    );
                }
                if no_fetch {
                    assert!(!stderr.contains("Fetching "));
                }
            } else {
                assert!(stdout.contains("Page fast evidence."));
                assert!(!stdout.contains("Page slow evidence."));
            }
        }
    })
    .await
    .unwrap();
}
