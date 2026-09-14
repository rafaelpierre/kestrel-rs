#![cfg(feature = "test-fixtures")]

use serde_json::Value;
use std::process::Command;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

fn without_timings(mut value: Value) -> Value {
    if let Some(object) = value.as_object_mut() {
        object
            .retain(|key, _| !matches!(key.as_str(), "elapsed_seconds" | "elapsed_ms" | "timing"));
        for item in object.values_mut() {
            *item = without_timings(item.take());
        }
    } else if let Some(array) = value.as_array_mut() {
        for item in array {
            *item = without_timings(item.take());
        }
    }
    value
}

#[tokio::test]
async fn capture_preserves_candidates_and_does_not_change_ordinary_output() {
    let server = MockServer::start().await;
    let endpoint = server.uri();
    let body = ["a", "b", "c"].map(|name| format!(
        "<li class=\"b_algo\"><h2><a href=\"{endpoint}/{name}\">{name}</a></h2><div class=\"b_caption\"><p>Fixture snippet.</p></div></li>"
    )).join("");
    Mock::given(path("/bing"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;
    for (name, text) in [
        ("a", "Gardening flowers and vegetables."),
        ("b", "Rust ownership borrowing lifetimes."),
        ("c", "Cooking recipes and ingredients."),
    ] {
        Mock::given(path(format!("/{name}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/plain")
                    .set_body_string(text.repeat(1000)),
            )
            .mount(&server)
            .await;
    }
    tokio::task::spawn_blocking(move || {
        for (format, diagnostics) in [("json", true), ("json", false), ("text", true)] {
            let mut baseline = None;
            for (directory_enabled, run_enabled) in
                [(false, false), (true, false), (false, true), (true, true)]
            {
                let temp = tempfile::tempdir().unwrap();
                let artifacts = temp.path().join("artifacts");
                let mut command = Command::new(env!("CARGO_BIN_EXE_kestrel"));
                command
                    .args([
                        "search",
                        "rust ownership",
                        "-e",
                        "bing",
                        "--min-results",
                        "3",
                        "-k",
                        "1",
                        "--output",
                        format,
                        "--timeout",
                        "5",
                        "--search-budget",
                        "5",
                        "--content-limit",
                        "100000",
                    ])
                    .env("KESTREL_TEST_PROVIDER_ENDPOINT", &endpoint)
                    .env("KESTRELSEARCH_OTEL_ENABLED", "false")
                    .env_remove("KESTRELSEARCH_BENCHMARK_ARTIFACT_DIR")
                    .env_remove("KESTRELSEARCH_BENCHMARK_RUN_ID");
                if !diagnostics {
                    command.arg("--no-diagnostics");
                }
                if directory_enabled {
                    command.env("KESTRELSEARCH_BENCHMARK_ARTIFACT_DIR", &artifacts);
                }
                if run_enabled {
                    command.env("KESTRELSEARCH_BENCHMARK_RUN_ID", "fixture");
                }
                let output = command.output().unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let actual = if format == "json" {
                    without_timings(serde_json::from_slice(&output.stdout).unwrap())
                } else {
                    Value::String(String::from_utf8(output.stdout).unwrap())
                };
                if let Some(expected) = &baseline {
                    assert_eq!(&actual, expected);
                } else {
                    baseline = Some(actual);
                }
                if directory_enabled && run_enabled {
                    let files: Vec<_> = std::fs::read_dir(&artifacts).unwrap().collect();
                    assert_eq!(files.len(), 1);
                    let artifact: Value = serde_json::from_slice(
                        &std::fs::read(files[0].as_ref().unwrap().path()).unwrap(),
                    )
                    .unwrap();
                    assert_eq!(artifact["candidates"].as_array().unwrap().len(), 3);
                    assert_eq!(artifact["results"].as_array().unwrap().len(), 1);
                    assert_eq!(artifact["results"][0]["url"], format!("{endpoint}/b"));
                    for candidate in artifact["candidates"].as_array().unwrap() {
                        assert!(candidate["bm25_score"].is_null());
                        assert!(candidate["content"].as_str().unwrap().len() > 10000);
                        assert!(!candidate["sources"].as_array().unwrap().is_empty());
                    }
                    assert!(artifact["results"][0]["bm25_score"].is_number());
                } else {
                    assert!(!artifacts.exists());
                }
            }
        }
    })
    .await
    .unwrap();
}
