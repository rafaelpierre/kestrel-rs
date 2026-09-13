use serde_json::Value;
use std::process::Command;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

fn command(endpoint: &str) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kestrel"));
    for (key, _) in std::env::vars().filter(|(key, _)| {
        key.starts_with("OTEL_")
            || key.starts_with("KESTRELSEARCH_OTEL_")
            || key == "TRACEPARENT"
            || key == "TRACESTATE"
    }) {
        cmd.env_remove(key);
    }
    cmd.env("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint)
        .env("OTEL_EXPORTER_OTLP_PROTOCOL", "http/json")
        .env("OTEL_TRACES_SAMPLER", "always_on")
        .env("KESTRELSEARCH_OTEL_CONTENT", "sanitized")
        .env(
            "OTEL_EXPORTER_OTLP_HEADERS",
            "x-honeycomb-team=ingest-secret-canary",
        )
        .env(
            "TRACEPARENT",
            "00-11111111111111111111111111111111-2222222222222222-01",
        );
    cmd
}
async fn fixture(server: &MockServer, export_status: u16) {
    Mock::given(method("POST"))
        .and(path("/v1/traces"))
        .respond_with(ResponseTemplate::new(export_status).set_body_json(serde_json::json!({})))
        .mount(server)
        .await;
    Mock::given(method("GET")).and(path("/page")).respond_with(ResponseTemplate::new(200).insert_header("content-type","text/plain").set_body_string("Readable evidence https://user:password@example.com/path?token=secret#fragment ingest-secret-canary".repeat(50))).mount(server).await;
}
fn spans(requests: &[wiremock::Request]) -> Vec<Value> {
    requests
        .iter()
        .filter(|r| r.method == "POST")
        .flat_map(|r| {
            let json: Value = serde_json::from_slice(&r.body).unwrap();
            json["resourceSpans"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|resource| {
                    resource["scopeSpans"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .flat_map(|scope| scope["spans"].as_array().unwrap().iter().cloned())
                })
                .collect::<Vec<_>>()
        })
        .collect()
}
#[tokio::test]
async fn exports_fetch_hierarchy_payload_limits_and_inherited_parent() {
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
    let server = MockServer::start().await;
    fixture(&server, 200).await;
    let mut cmd = command(&server.uri());
    cmd.env("KESTRELSEARCH_OTEL_PAYLOAD_BYTES", "256").args([
        "fetch",
        &format!("{}/page?token=secret", server.uri()),
        "--output",
        "json",
    ]);
    let output = tokio::task::spawn_blocking(move || cmd.output().unwrap())
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["content"]
            .as_str()
            .unwrap()
            .contains("Readable evidence")
    );
    let requests = server.received_requests().await.unwrap();
    let spans = spans(&requests);
    let root = spans
        .iter()
        .find(|s| s["name"] == "kestrel.cli.fetch")
        .unwrap();
    assert_eq!(root["parentSpanId"], "2222222222222222");
    assert_eq!(root["traceId"], "11111111111111111111111111111111");
    for name in [
        "kestrel.fetch",
        "kestrel.page",
        "kestrel.http_attempt",
        "kestrel.send",
        "kestrel.body",
        "kestrel.extraction",
    ] {
        let span = spans
            .iter()
            .find(|s| s["name"] == name)
            .unwrap_or_else(|| panic!("missing {name}: {spans:?}"));
        assert!(
            spans.iter().any(|s| s["spanId"] == span["parentSpanId"]),
            "orphan {name}"
        );
    }
    let exported = serde_json::to_string(&spans).unwrap();
    for secret in [
        "ingest-secret-canary",
        "password",
        "token=secret",
        "#fragment",
    ] {
        assert!(!exported.contains(secret), "leaked {secret}");
    }
    assert!(exported.contains("Readable evidence"));
    assert!(exported.contains("kestrel.payload.truncated"));
    for span in &spans {
        for event in span["events"].as_array().into_iter().flatten() {
            for attr in event["attributes"].as_array().unwrap() {
                if attr["key"] == "kestrel.payload" {
                    assert!(attr["value"]["stringValue"].as_str().unwrap().len() <= 300);
                }
            }
        }
    }
}
#[tokio::test]
async fn failed_export_preserves_cli_results_and_redacts_diagnostics() {
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
    let server = MockServer::start().await;
    fixture(&server, 401).await;
    let mut cmd = command(&server.uri());
    cmd.args([
        "fetch",
        &format!("{}/page", server.uri()),
        "--output",
        "json",
    ]);
    let output = tokio::task::spawn_blocking(move || cmd.output().unwrap())
        .await
        .unwrap();
    assert!(output.status.success());
    assert!(serde_json::from_slice::<Value>(&output.stdout).is_ok());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("OTLP export failed"));
    assert!(!stderr.contains("ingest-secret-canary"));
}
#[tokio::test]
async fn disabled_export_and_metadata_mode_do_not_export_content() {
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
    for disabled in [false, true] {
        let server = MockServer::start().await;
        fixture(&server, 200).await;
        let mut cmd = command(&server.uri());
        cmd.env("KESTRELSEARCH_OTEL_CONTENT", "none");
        if disabled {
            cmd.env("KESTRELSEARCH_OTEL_ENABLED", "false");
        }
        cmd.args(["fetch", &format!("{}/page", server.uri())]);
        let output = tokio::task::spawn_blocking(move || cmd.output().unwrap())
            .await
            .unwrap();
        assert!(output.status.success());
        let data = spans(&server.received_requests().await.unwrap());
        assert_eq!(data.is_empty(), disabled);
        assert!(
            !serde_json::to_string(&data)
                .unwrap()
                .contains("Readable evidence")
        );
    }
}

#[tokio::test]
async fn trace_specific_configuration_overrides_general_and_defaults_to_protobuf() {
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
    for protobuf in [false, true] {
        let server = MockServer::start().await;
        fixture(&server, 200).await;
        let mut cmd = command("not-a-valid-endpoint");
        cmd.env(
            "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
            format!("{}/v1/traces", server.uri()),
        )
        .env("OTEL_EXPORTER_OTLP_HEADERS", "invalid")
        .env(
            "OTEL_EXPORTER_OTLP_TRACES_HEADERS",
            "x-honeycomb-team=trace-secret",
        )
        .env("OTEL_EXPORTER_OTLP_PROTOCOL", "grpc");
        if protobuf {
            cmd.env_remove("OTEL_EXPORTER_OTLP_PROTOCOL");
        } else {
            cmd.env("OTEL_EXPORTER_OTLP_TRACES_PROTOCOL", "http/json");
        }
        cmd.args(["fetch", &format!("{}/page", server.uri())]);
        let output = tokio::task::spawn_blocking(move || cmd.output().unwrap())
            .await
            .unwrap();
        assert!(output.status.success());
        let requests = server.received_requests().await.unwrap();
        let export = requests.iter().find(|r| r.method == "POST").unwrap();
        assert_eq!(export.headers["x-honeycomb-team"], "trace-secret");
        assert_eq!(
            export.headers["content-type"],
            if protobuf {
                "application/x-protobuf"
            } else {
                "application/json"
            }
        );
        assert!(!export.body.is_empty());
    }
}

#[tokio::test]
async fn invalid_configuration_is_redacted_without_changing_help_exit() {
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
    for (key, value) in [
        ("OTEL_EXPORTER_OTLP_PROTOCOL", "grpc"),
        (
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "https://user:secret@example.com",
        ),
        ("OTEL_EXPORTER_OTLP_TIMEOUT", "0"),
        ("KESTRELSEARCH_OTEL_PAYLOAD_BYTES", "999999999"),
        ("KESTRELSEARCH_OTEL_CONTENT", "invalid"),
        ("OTEL_TRACES_SAMPLER", "invalid"),
        ("OTEL_EXPORTER_OTLP_HEADERS", "invalid"),
    ] {
        let mut cmd = command("http://127.0.0.1:1");
        cmd.env(key, value).arg("--help");
        let output = tokio::task::spawn_blocking(move || cmd.output().unwrap())
            .await
            .unwrap();
        assert!(output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("telemetry disabled"), "{key}: {stderr}");
        assert!(!stderr.contains("ingest-secret-canary"));
        assert!(!stderr.contains("user:secret"));
    }
}

#[tokio::test]
async fn aggregate_payload_budget_is_visible_and_bounds_large_content() {
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
    let server = MockServer::start().await;
    fixture(&server, 200).await;
    Mock::given(method("GET"))
        .and(path("/large"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/plain")
                .set_body_string("evidence ".repeat(20000)),
        )
        .mount(&server)
        .await;
    let mut cmd = command(&server.uri());
    cmd.env("KESTRELSEARCH_OTEL_PAYLOAD_BYTES", "65536").args([
        "fetch",
        &format!("{}/large", server.uri()),
        "--content-limit",
        "200000",
    ]);
    let output = tokio::task::spawn_blocking(move || cmd.output().unwrap())
        .await
        .unwrap();
    assert!(output.status.success());
    let data = spans(&server.received_requests().await.unwrap());
    assert!(
        serde_json::to_string(&data)
            .unwrap()
            .contains("kestrel.payload.budget_exhausted")
    );
    for span in data {
        let total: usize = span["events"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|event| event["attributes"].as_array().unwrap())
            .filter(|attr| attr["key"] == "kestrel.payload")
            .map(|attr| attr["value"]["stringValue"].as_str().unwrap().len())
            .sum();
        assert!(total <= 65536);
    }
}

#[tokio::test]
async fn slow_exporter_times_out_without_holding_command_open() {
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(10)))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/plain")
                .set_body_string("Useful local content"),
        )
        .mount(&server)
        .await;
    let mut cmd = command(&server.uri());
    cmd.env("OTEL_EXPORTER_OTLP_TIMEOUT", "50").args([
        "fetch",
        &format!("{}/page", server.uri()),
        "--output",
        "json",
    ]);
    let started = std::time::Instant::now();
    let output = tokio::task::spawn_blocking(move || cmd.output().unwrap())
        .await
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("OTLP export failed"));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "export must time out before the ten-second receiver delay"
    );
}
