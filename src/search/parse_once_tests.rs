use super::*;
use crate::provider_diagnostics::Challenge;

#[test]
fn adapter_normalization_and_fusion_preserve_ranks_sources_and_partial_success() {
    let bing = r#"<li class='b_algo'><h2><a href='javascript:alert(1)'>Invalid</a></h2></li>
        <li class='b_algo'><h2><a href='https://example.org/shared/?utm_source=bing'>Bing shared</a></h2></li>
        <li class='b_algo'><h2><a href='https://example.org/bing'>Bing unique</a></h2></li>"#;
    let dogpile = r#"{"results":[
        {"clickUrl":"https://example.org/shared#section","title":"Dogpile shared","description":"Other snippet"},
        {"clickUrl":"https://example.org/dogpile","title":"Dogpile unique","description":"Unique"}] }"#;
    let mut outcomes = Vec::new();
    for (engine, body) in [(Engine::Bing, bing), (Engine::Dogpile, dogpile)] {
        let parsed = ParsedResponse::new(engine, body);
        let mut response = ProviderResponse {
            results: extract_completed(engine, body, &parsed).unwrap(),
            retries: 0,
            raw_result_count: 0,
        };
        filter_response("site:example.org", &mut response);
        assert_eq!(
            response.raw_result_count,
            if engine == Engine::Bing { 3 } else { 2 }
        );
        outcomes.push(Ok(with_provenance(
            response.results,
            engine,
            "site:example.org",
        )));
    }
    outcomes.insert(1, Err(KestrelError::SearchDeadline));
    let merged = merge_outcomes(outcomes).unwrap();
    assert_eq!(
        merged.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(),
        ["Bing shared", "Bing unique", "Dogpile unique"]
    );
    assert_eq!(merged[0].engine_rank, Some(2));
    assert_eq!(merged[0].url, "https://example.org/shared/?utm_source=bing");
    assert_eq!(
        merged[0].sources,
        vec![
            crate::SourceOccurrence {
                engine: Engine::Bing,
                query: "site:example.org".into(),
                rank: 2
            },
            crate::SourceOccurrence {
                engine: Engine::Dogpile,
                query: "site:example.org".into(),
                rank: 1
            },
        ]
    );
}

#[test]
fn completed_html_validation_and_extraction_share_document() {
    for (engine, card, empty) in [
        (
            Engine::Bing,
            "<li class='b_algo'><h2><a href='https://example.org'>Title</a></h2></li>",
            "<li class='b_no'>No results</li>",
        ),
        (
            Engine::Yahoo,
            "<div class='dd algo'><div class='compTitle'><h3><a href='https://example.org'>Title</a></h3></div></div>",
            "<div class='msgNoResults'>No results</div>",
        ),
        (
            Engine::Duckduckgo,
            "<div class='result results_links results_links_deep web-result'><h2 class='result__title'><a class='result__a' href='https://example.org'>Title</a></h2></div>",
            "<div class='no-results'>No results</div>",
        ),
    ] {
        let body = card.repeat(1000);
        let results = parse_provider_response(engine, &body).unwrap();
        assert_eq!(results.len(), 1000, "{engine}");
        assert!(results.iter().all(|r| r.title == "Title"));
        assert!(parse_provider_response(engine, empty).unwrap().is_empty());
        assert!(parse_provider_response(engine, "<html>unknown</html>").is_err());
        let blocked = format!("<form id='captcha'></form>{body}");
        assert!(
            parse_provider_response(engine, &blocked)
                .unwrap_err()
                .to_string()
                .contains("bot challenge")
        );
    }
}

#[test]
#[ignore = "local parse timing observation; run explicitly with --nocapture"]
fn completed_parse_timing() {
    let card = "<li class='b_algo'><h2><a href='https://example.org'>Title</a></h2><div class='b_caption'><p>Fixture snippet</p></div></li>";
    for cards in [100, 1000, 10000] {
        let body = card.repeat(cards);
        for trial in 0..6 {
            for duplicate in if trial % 2 == 0 {
                [true, false]
            } else {
                [false, true]
            } {
                let started = Instant::now();
                // Reproduce the old dispatcher/extractor overlap: retain the
                // validation DOM while a second DOM extracts the same body.
                let validation = Html::parse_document(&body);
                let results = if duplicate {
                    let extraction = Html::parse_document(&body);
                    parse_bing_document(&extraction)
                } else {
                    parse_bing_document(&validation)
                };
                assert_eq!(results.len(), cards);
                std::hint::black_box(&validation);
                eprintln!(
                    "parse_timing cards={cards} trial={trial} duplicate={duplicate} elapsed_us={}",
                    started.elapsed().as_micros()
                );
            }
        }
    }
}

#[test]
fn json_result_markup_is_not_a_page_level_html_challenge() {
    let text = r#"{"results":[{"clickUrl":"https://example.org","title":"Captcha documentation","description":"<form id='captcha'>Example</form>"}]}"#;
    assert_eq!(
        classify_challenge(Engine::Dogpile, text),
        Challenge::NotDetected
    );
    assert_eq!(
        classify_challenge(Engine::Dogpile, "<form id='captcha'></form>"),
        Challenge::Detected
    );
    assert_eq!(
        classify_challenge(Engine::Qwant, r#"{"url":"https://example.org/challenge"}"#),
        Challenge::Detected
    );
    let result = parse_provider_response(Engine::Dogpile, text).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].snippet, "Example");
    assert!(
        parse_provider_response(Engine::Qwant, r#"{"url":"https://example.org/challenge"}"#)
            .unwrap_err()
            .to_string()
            .contains("bot challenge")
    );
    assert!(parse_provider_response(Engine::Dogpile, "<form id='captcha'></form>").is_err());
}

fn completed_fixtures() -> Vec<(Engine, &'static str)> {
    vec![
        (
            Engine::Duckduckgo,
            r#"<div class="result results_links results_links_deep web-result"><h2 class="result__title"><a class="result__a" href="https://example.org">Title</a></h2></div>"#,
        ),
        (
            Engine::Bing,
            include_str!("../../tests/fixtures/providers/bing-unrelated.html"),
        ),
        (
            Engine::Yahoo,
            r#"<div class="dd algo"><div class="compTitle"><h3><a href="https://example.org">Title</a></h3></div></div>"#,
        ),
        (
            Engine::Dogpile,
            include_str!("../../tests/fixtures/providers/dogpile.json"),
        ),
        (
            Engine::Ecosia,
            include_str!("../../tests/fixtures/providers/ecosia.html"),
        ),
        (
            Engine::Swisscows,
            include_str!("../../tests/fixtures/providers/swisscows.json"),
        ),
        (
            Engine::Yep,
            include_str!("../../tests/fixtures/providers/yep.json"),
        ),
        (
            Engine::Qwant,
            include_str!("../../tests/fixtures/providers/qwant.json"),
        ),
        (
            Engine::Mojeek,
            include_str!("../../tests/fixtures/providers/mojeek.html"),
        ),
    ]
}

fn old_completed(engine: Engine, body: &str) -> Result<Vec<SearchResult>, ProviderFailure> {
    match engine {
        Engine::Duckduckgo => parse_duckduckgo_response(body),
        Engine::Bing | Engine::Yahoo => parse_provider_response(engine, body),
        _ => crate::providers::parse(engine, body),
    }
}

#[test]
fn completed_reuse_preserves_all_adapters_and_classification_boundaries() {
    for (engine, fixture) in completed_fixtures() {
        for body in [
            fixture,
            "",
            "  ",
            "null",
            "{",
            "[]",
            "{}",
            "<html>unknown</html>",
            "<form id='captcha'></form>",
            "<form id='challenge-form'></form>",
            r#"{"url":"https://example.org/challenge"}"#,
        ] {
            let document = ParsedResponse::new(engine, body);
            assert_eq!(
                extract_completed(engine, body, &document).map_err(|e| e.to_string()),
                old_completed(engine, body).map_err(|e| e.to_string()),
                "{engine}: {body}",
            );
            assert_eq!(
                document.challenge(engine, body),
                classify_challenge(engine, body)
            );
        }
    }
    // Broad diagnostic markers must not silently broaden native adapter rejection.
    let body = "<form id='captcha'></form><div class='no-results'></div>";
    let document = ParsedResponse::new(Engine::Duckduckgo, body);
    assert_eq!(
        document.challenge(Engine::Duckduckgo, body),
        Challenge::Detected
    );
    assert!(
        extract_completed(Engine::Duckduckgo, body, &document)
            .unwrap()
            .is_empty()
    );
    assert!(extract_dispatched(Engine::Duckduckgo, body, &document).is_err());
}

#[tokio::test]
async fn completed_transport_extracts_in_worker_without_retrying_parse_failures() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    for (engine, fixture) in completed_fixtures() {
        for (status, body) in [
            (200, fixture),
            (200, "<html>unknown</html>"),
            (403, "<form id='captcha'></form>"),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(status).set_body_string(body))
                .expect(1)
                .mount(&server)
                .await;
            let recorder = Arc::new(Mutex::new(Recorder::new()));
            let response = PROVIDER_RECORDER
                .scope(recorder.clone(), async {
                    if engine == Engine::Yahoo {
                        let client = primp::Client::builder().no_proxy().build().unwrap();
                        request_yahoo_with_retries("fixture", extract_completed, || {
                            client.get(server.uri())
                        })
                        .await
                    } else {
                        let client: crate::http_client::Client = reqwest::Client::builder()
                            .no_proxy()
                            .build()
                            .unwrap()
                            .into();
                        request_standard_with_retries(
                            &client,
                            engine,
                            "fixture",
                            extract_completed,
                            || client.get(server.uri()),
                        )
                        .await
                    }
                })
                .await;
            if status == 200 {
                let (results, retries) = response.unwrap();
                assert_eq!(retries, 0);
                assert_eq!(
                    results.map_err(|e| e.to_string()),
                    old_completed(engine, body).map_err(|e| e.to_string())
                );
            } else {
                assert!(response.unwrap_err().to_string().contains("HTTP 403"));
            }
            assert_eq!(recorder.lock().unwrap().finish(false).send_attempts, 1);
            server.verify().await;
        }
    }
}

/// Run the same fixture work under an external allocation profiler. Fixture
/// construction and warm-up are outside reported timings, but profilers include
/// process/test-harness overhead. No production allocator changes are needed.
#[test]
#[ignore = "local completed-response timing/allocation profiling"]
fn completed_transport_parse_profile() {
    let duplicate = std::env::var("KESTREL_PARSE_PROFILE_MODE").unwrap() == "duplicate";
    let engine = if std::env::var("KESTREL_PARSE_PROFILE_ENGINE").unwrap() == "qwant" {
        Engine::Qwant
    } else {
        Engine::Bing
    };
    let body = if engine == Engine::Bing {
        "<li class='b_algo'><h2><a href='https://example.org'>Title</a></h2><div class='b_caption'><p>Snippet</p></div></li>".repeat(1000)
    } else {
        let item = r#"{"url":"https://example.org","title":"Title","desc":"Snippet"}"#;
        format!(
            r#"{{"status":"success","data":{{"result":{{"items":{{"mainline":[{{"type":"web","items":[{}]}}]}}}}}}}}"#,
            vec![item; 1000].join(",")
        )
    };
    let _ = old_completed(engine, &body).unwrap();
    for trial in 0..10 {
        let started = Instant::now();
        let (challenge, results) = if duplicate {
            (
                classify_challenge(engine, &body),
                old_completed(engine, &body),
            )
        } else {
            let document = ParsedResponse::new(engine, &body);
            (
                document.challenge(engine, &body),
                extract_completed(engine, &body, &document),
            )
        };
        assert_eq!(challenge, Challenge::NotDetected);
        assert_eq!(results.unwrap().len(), 1000);
        eprintln!(
            "completed_profile engine={engine} duplicate={duplicate} trial={trial} elapsed_us={}",
            started.elapsed().as_micros()
        );
    }
}

#[test]
fn completed_transport_constructs_one_representation_including_envelopes() {
    use base64::Engine as _;
    let inner = include_str!("../../tests/fixtures/providers/swisscows.json");
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(inner);
    let envelope = format!(r#"{{"payload":"header.{encoded}.signature"}}"#);
    let malformed = r#"{"payload":"header.not-json.signature"}"#;
    let mut fixtures = completed_fixtures();
    fixtures.extend([
        (Engine::Swisscows, envelope.as_str()),
        (Engine::Swisscows, malformed),
    ]);
    for (engine, body) in fixtures {
        let expected = old_completed(engine, body).map_err(|e| e.to_string());
        for success in [true, false] {
            RESPONSE_PARSES.with(|count| count.set(0));
            let (_, results) = process_completed(engine, body, success, extract_completed);
            assert_eq!(RESPONSE_PARSES.with(|count| count.get()), 1, "{engine}");
            if success {
                assert_eq!(results.unwrap().map_err(|e| e.to_string()), expected);
            } else {
                assert!(results.is_none());
            }
        }
    }
}
