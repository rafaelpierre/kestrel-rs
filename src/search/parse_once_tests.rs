use super::*;

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
