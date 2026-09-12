//! Opt-in live experiment. No production configuration or relevance heuristics.
use super::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

fn clean_url(value: &str, search: bool) -> String {
    let Ok(mut url) = Url::parse(value) else {
        return String::new();
    };
    if !matches!(url.scheme(), "http" | "https") {
        return String::new();
    }
    let pairs: Vec<_> = url
        .query_pairs()
        .into_owned()
        .filter(|(k, _)| search && matches!(k.as_str(), "q" | "cc"))
        .collect();
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_fragment(None);
    url.set_query(None);
    if !pairs.is_empty() {
        url.query_pairs_mut().extend_pairs(pairs);
    }
    url.into()
}

fn clean_results(results: &mut [SearchResult]) {
    for r in results {
        r.url = clean_url(&r.url, false);
        // Display URLs may include provider tracking; the destination is sufficient.
        r.display_url.clear();
    }
}

fn evidence(html: &str) -> Value {
    let document = Html::parse_document(html);
    let title = document
        .select(&selector("title"))
        .next()
        .map(|e| element_text(e, " "))
        .unwrap_or_default();
    let mut results = parse_bing_results(html);
    clean_results(&mut results);
    json!({"document_title":title,"response_sha256":format!("{:x}",Sha256::digest(html)),
        "response_bytes":html.len(),"organic_entries":results})
}

async fn isolated(
    query: &str,
    standard: &reqwest::Client,
    impersonated: &primp::Client,
    variant: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    let region = if variant == "bing-no-region" {
        ""
    } else {
        "gb-en"
    };
    let mut builder = bing_request(standard, query, region);
    if variant == "bing-browser-form" {
        // Observed on the browser's search-box submission, not an invented parameter.
        builder = builder.query(&[("form", "QBRE")]);
    }
    let mut request = builder.build()?;
    if variant == "bing-percent-space" {
        let query = request
            .url()
            .query()
            .unwrap_or_default()
            .replace('+', "%20");
        request.url_mut().set_query(Some(&query));
    }
    let initial = request.url().clone();
    // The candidate changes transport only; query serialization and headers match.
    let (status, final_url, version, cache_headers, html) = if variant != "bing-impersonated" {
        let response = standard.execute(request).await?;
        let headers: std::collections::BTreeMap<_, _> = ["age", "cache-control", "via", "x-cache"]
            .into_iter()
            .filter_map(|k| {
                response
                    .headers()
                    .get(k)
                    .and_then(|v| v.to_str().ok())
                    .map(|v| (k, v.to_owned()))
            })
            .collect();
        let meta = (
            response.status().as_u16(),
            clean_url(response.url().as_str(), true),
            format!("{:?}", response.version()),
            headers,
        );
        (meta.0, meta.1, meta.2, meta.3, response.text().await?)
    } else {
        let response = impersonated.get(initial.as_str()).send().await?;
        let headers: std::collections::BTreeMap<_, _> = ["age", "cache-control", "via", "x-cache"]
            .into_iter()
            .filter_map(|k| {
                response
                    .headers()
                    .get(k)
                    .and_then(|v| v.to_str().ok())
                    .map(|v| (k, v.to_owned()))
            })
            .collect();
        let meta = (
            response.status().as_u16(),
            clean_url(response.url().as_str(), true),
            format!("{:?}", response.version()),
            headers,
        );
        (meta.0, meta.1, meta.2, meta.3, response.text().await?)
    };
    if let Ok(directory) = std::env::var("KESTREL_BING_RAW_DIR") {
        fs::create_dir_all(&directory)?;
        let key = format!("{variant}-{:x}", Sha256::digest(query));
        fs::write(Path::new(&directory).join(format!("{key}.html")), &html)?;
    }
    let mut capture = evidence(&html);
    let parsed = parse_provider_response(Engine::Bing, &html);
    let (mut results, error) = match parsed {
        Ok(r) if (200..300).contains(&status) => (r, None),
        Ok(_) => (vec![], Some(format!("HTTP {status}"))),
        Err(e) => (vec![], Some(e.to_string())),
    };
    clean_results(&mut results);
    capture["results"] = json!(results);
    capture["error"] = json!(error);
    // Constructed locally from public fixture queries; preserve exact wire encoding.
    capture["initial_url"] = json!(initial.as_str());
    capture["final_url"] = json!(final_url);
    capture["http_status"] = json!(status);
    capture["http_version"] = json!(version);
    capture["cache_headers"] = json!(cache_headers);
    capture["redirect_chain"] = Value::Null;
    capture["attempts"] = json!(1);
    Ok(capture)
}

#[tokio::test]
#[ignore = "live network experiment; set KESTREL_BING_EXPERIMENT_DIR to a new directory"]
async fn capture_live_matrix() {
    let directory =
        std::env::var("KESTREL_BING_EXPERIMENT_DIR").expect("explicit output directory required");
    let directory = Path::new(&directory);
    fs::create_dir_all(directory).unwrap();
    let output = directory.join("runs.json");
    assert!(
        !output.exists(),
        "use a new directory to preserve earlier evidence"
    );
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../../benchmarks/bing-fidelity/queries.json")).unwrap();
    let profile = crate::http_client::BrowserProfile::bing_experiment();
    let headers: std::collections::BTreeMap<_, _> = profile
        .headers()
        .iter()
        .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_owned())))
        .collect();
    let budget = Duration::from_secs(3);
    let standard =
        crate::http_client::standard_builder(profile, &crate::TransportOptions::default())
            .timeout(budget)
            .build()
            .unwrap();
    let mut impersonated =
        crate::http_client::impersonated_builder(profile, &crate::TransportOptions::default())
            .timeout(budget)
            .build()
            .unwrap();
    *impersonated.headers_mut() = profile.headers();
    let clients = SearchClients {
        standard: standard.clone(),
        yahoo: Some(impersonated.clone()),
    };
    let mut rows = vec![];
    // Alternate configuration order by query to reduce order/time confounding.
    let normal_variants = [
        "bing-standard",
        "bing-impersonated",
        "bing-no-region",
        "bing-browser-form",
        "fallback",
        "fanout-q1",
        "fanout-all",
    ];
    let variants: &[&str] = if std::env::var_os("KESTREL_BING_ENCODING_ONLY").is_some() {
        &["bing-standard", "bing-percent-space"]
    } else {
        &normal_variants
    };
    for (index, case) in cases.iter().enumerate() {
        let query = case["query"].as_str().unwrap();
        for offset in 0..variants.len() {
            let variant = variants[(offset + index) % variants.len()];
            let started = Instant::now();
            let timestamp = chrono::Utc::now().to_rfc3339();
            let mut row = if variant.starts_with("bing-") {
                match tokio::time::timeout(
                    budget,
                    isolated(query, &standard, &impersonated, variant),
                )
                .await
                {
                    Ok(Ok(row)) => row,
                    Ok(Err(_)) => json!({"results":[],"error":"transport_error"}),
                    Err(_) => json!({"results":[],"error":"deadline"}),
                }
            } else {
                let options = SearchOptions {
                    mode: if variant == "fallback" {
                        SearchMode::Fallback
                    } else {
                        SearchMode::Fanout
                    },
                    provider_quorum: (variant == "fanout-q1").then_some(1),
                    region: "gb-en".into(),
                    search_budget: Some(budget),
                    ..Default::default()
                };
                match search_many_reusing_clients_detailed(&[query.into()], &options, &clients)
                    .await
                {
                    Ok(mut report) => {
                        clean_results(&mut report.results);
                        for d in &mut report.providers {
                            // reqwest errors may embed redirect tracking; retain the outcome category.
                            if d.error.is_some() {
                                d.error = Some(d.outcome.clone());
                            }
                        }
                        json!({"results":report.results,"providers":report.providers,"cancelled":report.cancelled,"error":null})
                    }
                    Err(_) => json!({"results":[],"error":"all_providers_failed"}),
                }
            };
            row["id"] = json!(format!("{}-{variant}", case["id"].as_str().unwrap()));
            row["query_id"] = case["id"].clone();
            row["query"] = json!(query);
            row["variant"] = json!(variant);
            row["started_at"] = json!(timestamp);
            row["elapsed_ms"] = json!(started.elapsed().as_millis() as u64);
            row["budget_ms"] = json!(3000);
            eprintln!(
                "{}: {} results in {} ms",
                row["id"],
                row["results"].as_array().unwrap().len(),
                row["elapsed_ms"]
            );
            rows.push(row);
            // Persist every scheduled result, including empty/error outcomes.
            fs::write(&output, serde_json::to_vec_pretty(&json!({
                "schema":1,"expected_rows": cases.len() * variants.len(),
                "completed": rows.len() == cases.len() * variants.len(), "headers":headers,"region":"gb-en","session":"reused clients, no cookie jar",
                "network_context":"local host; browser egress equivalence not verified",
                "isolated_policy":"single attempt; orchestration uses production retries",
                "url_sanitization":"search q/cc retained; result query strings and fragments removed; judge original title/snippet alongside path",
                "runs":rows
            })).unwrap()).unwrap();
        }
    }
}

#[test]
fn evidence_removes_tracking_and_credentials() {
    assert_eq!(
        clean_url(
            "https://user:secret@bing.com/search?q=moon+gravity&cc=gb&rdrig=secret#secret",
            true
        ),
        "https://bing.com/search?q=moon+gravity&cc=gb"
    );
    assert_eq!(
        clean_url("https://example.com/path?token=secret#secret", false),
        "https://example.com/path"
    );
    assert_eq!(clean_url("javascript:alert(1)", false), "");
}

#[test]
fn replays_sanitized_live_response_blocks() {
    let quoted = parse_provider_response(
        Engine::Bing,
        include_str!("../../tests/fixtures/providers/bing-live-quoted.html"),
    )
    .unwrap();
    assert_eq!(quoted.len(), 2);
    assert_eq!(
        quoted[0].title,
        "USE | English meaning - Cambridge Dictionary"
    );
    assert_eq!(
        quoted[0].url,
        "https://dictionary.cambridge.org/dictionary/english/use"
    );
    assert_eq!(
        quoted[1].title,
        "USE Definition & Meaning - Merriam-Webster"
    );
    assert!(
        quoted[0]
            .snippet
            .starts_with("USE definition: 1. to put something")
    );
    let tokio = parse_provider_response(
        Engine::Bing,
        include_str!("../../tests/fixtures/providers/bing-live-tokio.html"),
    )
    .unwrap();
    assert_eq!(tokio.len(), 2);
    assert_eq!(
        tokio[0].url,
        "https://docs.rs/tokio/latest/tokio/sync/watch/struct.Receiver.html"
    );
    assert!(tokio[0].snippet.contains("returned Ref type is never held"));
    assert!(tokio[1].snippet.contains("Receiver::borrow_and_update"));
    let site = parse_provider_response(
        Engine::Bing,
        include_str!("../../tests/fixtures/providers/bing-live-site.html"),
    )
    .unwrap();
    assert_eq!(site.len(), 2);
    assert_eq!(site[0].title, "Tokyo - Wikipedia");
    assert!(site.iter().all(|r| !result_allowed(
        "site:docs.rs tokio watch Receiver borrow_and_update",
        &r.url
    )));
}
