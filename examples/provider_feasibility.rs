//! #40: bounded, credential-free protocol probes and matched baseline retrieval.
use futures_util::StreamExt;
use kestrelsearch::{Engine, KestrelClient, SearchOptions};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn body(provider: &str, query: &str) -> Value {
    if provider == "felo" {
        json!({"query":query,"search_uuid":uuid::Uuid::new_v4().to_string(),
        "lang":"","agent_lang":"en","search_options":{"langcode":"en-GB"},
        "search_video":true,"query_from":"default","category":"google","model":"",
        "mode":"concise","stream_protocol":"message_center_v1","enable_task_state":true})
    } else {
        json!({"message":{"content":query}})
    }
}

fn classify(status: u16, bytes: &[u8]) -> (&'static str, Option<String>) {
    let value: Value = serde_json::from_slice(bytes).unwrap_or(Value::Null);
    let code = value
        .pointer("/detail/error_type")
        .or(value.pointer("/error/code"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let outcome = match (status, code.as_deref()) {
        (_, Some("turnstile_session_token_required")) => "challenge",
        (401 | 403, _) => "access_denied",
        (429, _) => "rate_limited",
        (400..=599, _) => "http_error",
        _ => "unrecognized_response",
    };
    (outcome, code)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 1 {
        return Err("usage: provider_feasibility WINDOW_LABEL".into());
    }
    let queries: Vec<Value> = serde_json::from_str(include_str!(
        "../benchmarks/provider-feasibility/queries.json"
    ))?;
    let make_http = || {
        reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36")
        .timeout(Duration::from_secs(5)).redirect(reqwest::redirect::Policy::none()).build()
    };
    let reused_http = make_http()?;
    let reused_baseline = KestrelClient::new()?;
    let mut cooldowns = std::collections::HashMap::<String, Instant>::new();
    let providers = if args[0] == "window-1" {
        ["felo", "manus", "swisscows", "bing"]
    } else {
        ["bing", "swisscows", "manus", "felo"]
    };
    for session in ["fresh_client", "reused_client"] {
        for q in &queries {
            let query = q["query"].as_str().unwrap();
            for provider in providers {
                let mut record = json!({"window":args[0],"session":session,"provider":provider,
                    "query_id":q["id"],"query":query,"budget_seconds":5,"timestamp":chrono::Utc::now().to_rfc3339()});
                if cooldowns
                    .get(provider)
                    .is_some_and(|until| *until > Instant::now())
                {
                    record["outcome"] = json!("skipped_retry_after");
                    record["attempts"] = json!(0);
                    println!("{record}");
                    continue;
                }
                let started = Instant::now();
                if matches!(provider, "felo" | "manus") {
                    let client = if session == "fresh_client" {
                        make_http()?
                    } else {
                        reused_http.clone()
                    };
                    let url = if provider == "felo" {
                        "https://felo.ai/api-proxy/main/search/threads"
                    } else {
                        "https://api.manus.ai/v2/task.create"
                    };
                    record["scope"] = json!(if provider == "felo" {
                        "public_frontend"
                    } else {
                        "credentialed_api_without_credentials"
                    });
                    record["attempts"] = json!(1);
                    record["retries"] = json!(0);
                    let result = async {
                        let response = client
                            .post(url)
                            .header("Content-Type", "application/json")
                            .header(
                                "Origin",
                                if provider == "felo" {
                                    "https://felo.ai"
                                } else {
                                    "https://manus.im"
                                },
                            )
                            .body(body(provider, query).to_string())
                            .send()
                            .await?;
                        let status = response.status().as_u16();
                        record["status"] = json!(status);
                        record["http_version"] = json!(format!("{:?}", response.version()));
                        record["retry_after"] = json!(
                            response
                                .headers()
                                .get("retry-after")
                                .and_then(|v| v.to_str().ok())
                        );
                        let mut stream = response.bytes_stream();
                        let mut bytes = Vec::new();
                        while let Some(chunk) = stream.next().await {
                            let chunk = chunk?;
                            if bytes.len().saturating_add(chunk.len()) > 2_000_000 {
                                record["outcome"] = json!("body_limit");
                                return Ok::<(), reqwest::Error>(());
                            }
                            bytes.extend_from_slice(&chunk);
                        }
                        let (outcome, code) = classify(status, &bytes);
                        record["outcome"] = json!(outcome);
                        record["error_code"] = json!(code);
                        record["body_bytes"] = json!(bytes.len());
                        record["results"] = json!([]);
                        Ok(())
                    }
                    .await;
                    if let Err(error) = result {
                        record["outcome"] = json!(if error.is_timeout() {
                            "timeout"
                        } else {
                            "transport_error"
                        });
                    }
                } else {
                    let client = if session == "fresh_client" {
                        KestrelClient::new()?
                    } else {
                        reused_baseline.clone()
                    };
                    let engine = if provider == "bing" {
                        Engine::Bing
                    } else {
                        Engine::Swisscows
                    };
                    let options = SearchOptions {
                        engines: vec![engine],
                        search_budget: Some(Duration::from_secs(5)),
                        ..Default::default()
                    };
                    match client
                        .search_many_detailed(&[query.to_owned()], &options)
                        .await
                    {
                        Ok(report) => {
                            record["outcome"] = json!(if report.results.is_empty() {
                                "empty_or_failed"
                            } else {
                                "nonempty"
                            });
                            record["results"] = json!(report.results);
                            record["diagnostics"] = json!(report.providers);
                        }
                        Err(error) => {
                            record["outcome"] = json!("search_error");
                            record["error"] = json!(error.to_string());
                        }
                    }
                }
                if record["status"] == 429 {
                    let seconds = record["retry_after"]
                        .as_str()
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(180)
                        .min(86400);
                    cooldowns.insert(
                        provider.to_owned(),
                        Instant::now() + Duration::from_secs(seconds),
                    );
                }
                record["elapsed_seconds"] = json!(started.elapsed().as_secs_f64());
                println!("{record}");
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserve_complete_queries() {
        let queries: Vec<Value> = serde_json::from_str(include_str!(
            "../benchmarks/provider-feasibility/queries.json"
        ))
        .unwrap();
        for q in queries {
            for p in ["felo", "manus"] {
                let text = q["query"].as_str().unwrap();
                let wire: Value = serde_json::from_str(&body(p, text).to_string()).unwrap();
                assert_eq!(
                    wire.pointer(if p == "felo" {
                        "/query"
                    } else {
                        "/message/content"
                    })
                    .unwrap(),
                    text
                );
            }
        }
    }
    #[test]
    fn observed_errors_are_not_empty_results() {
        assert_eq!(
            classify(
                400,
                include_bytes!("../tests/fixtures/providers/felo-token-required.json")
            )
            .0,
            "challenge"
        );
        assert_eq!(
            classify(
                401,
                include_bytes!("../tests/fixtures/providers/manus-unauthenticated.json")
            )
            .0,
            "access_denied"
        );
        assert_eq!(classify(200, b"[]").0, "unrecognized_response");
        assert_eq!(classify(200, b"{broken").0, "unrecognized_response");
        assert_eq!(classify(429, b"{}").0, "rate_limited");
        assert_eq!(classify(503, b"{}").0, "http_error");
    }
}
