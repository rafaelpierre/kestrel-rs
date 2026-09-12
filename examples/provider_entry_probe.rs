//! Public-entry access diagnostics for #40, not a search-result benchmark.
//! cargo run --example provider_entry_probe -- felo
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde_json::json;
use sha2::{Digest, Sha256};

const LIMIT: usize = 2_000_000;

fn classify(status: u16, body: &[u8]) -> (&'static str, bool) {
    let text = String::from_utf8_lossy(body).to_ascii_lowercase();
    // Evidence hints only: ordinary scripts may contain these strings too.
    let challenge_hint = ["cf-chl-", "verify you are human", "g-recaptcha"]
        .iter()
        .any(|marker| text.contains(marker));
    let outcome = match status {
        429 => "rate_limited",
        400..=599 => "http_error",
        200..=299 if challenge_hint => "possible_challenge",
        200..=299 => "unvalidated_entry_response",
        _ => "unexpected_status",
    };
    (outcome, challenge_hint)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let url = match args.as_slice() {
        [provider] if provider == "felo" => "https://felo.ai/search",
        [provider] if provider == "manus" => "https://manus.im/",
        _ => return Err("usage: provider_entry_probe felo|manus".into()),
    };
    let client = reqwest::Client::builder()
        .user_agent("kestrel-issue-40-entry-probe/1")
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()?;
    let started = Instant::now();
    let mut record = json!({
        "provider": args[0], "url": url,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "scope": "public_entry_only", "client": "reqwest/rustls",
        "user_agent": "kestrel-issue-40-entry-probe/1",
        "timeout_seconds": 10, "max_response_bytes": LIMIT,
        "retries": 0, "session": "fresh_no_cookies",
        "search_results": null, "challenge_hint": false,
    });
    let result = async {
        let response = client.get(url).send().await?;
        let status = response.status().as_u16();
        record["status"] = json!(status);
        // Do not serialize redirect URLs, headers or bodies: they may carry IDs.
        record["redirected"] = json!(response.url().as_str() != url);
        record["http_version"] = json!(format!("{:?}", response.version()));
        let mut stream = response.bytes_stream();
        let mut body = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if body.len().saturating_add(chunk.len()) > LIMIT {
                record["outcome"] = json!("body_limit");
                record["bytes_retained"] = json!(body.len());
                return Ok::<(), reqwest::Error>(());
            }
            body.extend_from_slice(&chunk);
        }
        let (outcome, challenge_hint) = classify(status, &body);
        record["outcome"] = json!(outcome);
        record["challenge_hint"] = json!(challenge_hint);
        record["body_bytes"] = json!(body.len());
        record["body_sha256"] = json!(format!("{:x}", Sha256::digest(&body)));
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
    record["elapsed_seconds"] = json!(started.elapsed().as_secs_f64());
    println!("{}", record);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::classify;

    #[test]
    fn http_failures_retain_independent_challenge_evidence() {
        assert_eq!(classify(403, b"cf-chl-test"), ("http_error", true));
        assert_eq!(
            classify(429, b"verify you are human"),
            ("rate_limited", true)
        );
        assert_eq!(classify(500, b"upstream failed"), ("http_error", false));
    }

    #[test]
    fn entry_pages_never_claim_search_success_or_valid_empty() {
        for body in [
            b"<html>Navigation</html>".as_slice(),
            b"[]",
            b"",
            b"{broken",
        ] {
            assert_eq!(classify(200, body), ("unvalidated_entry_response", false));
        }
        assert_eq!(
            classify(200, b"Verify you are human"),
            ("possible_challenge", true)
        );
        assert_eq!(classify(302, b""), ("unexpected_status", false));
    }
}
