use super::*;
use crate::{ContentQualityState, assess_content_quality};
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    id: String,
    html: Option<String>,
    text: Option<String>,
    extracted: Option<String>,
    state: ContentQualityState,
    judgment: String,
    cause: String,
}

#[test]
fn frozen_quality_corpus_preserves_evidence_and_reports_errors() {
    for (name, data) in [
        (
            "development",
            include_str!("../../tests/fixtures/content-quality/development.json"),
        ),
        (
            "held-out",
            include_str!("../../tests/fixtures/content-quality/held-out.json"),
        ),
    ] {
        let cases: Vec<Case> = serde_json::from_str(data).unwrap();
        let mut counts = [0; 4]; // true positive, false positive, false negative, true negative
        for case in cases {
            let content = case
                .html
                .as_deref()
                .and_then(|html| parse_content(html, 20_000))
                .or(case.text);
            if case.html.is_some() {
                assert_eq!(content, case.extracted, "{}: {}", case.id, case.cause);
            }
            let quality = assess_content_quality(content.as_deref());
            assert_eq!(quality.state, case.state, "{}: {}", case.id, case.cause);
            let flagged = quality.state == ContentQualityState::BoilerplateOnly;
            let unusable = case.judgment == "unusable";
            counts[match (flagged, unusable) {
                (true, true) => 0,
                (true, false) => 1,
                (false, true) => 2,
                (false, false) => 3,
            }] += 1;
        }
        eprintln!(
            "{name}: TP={} FP={} FN={} TN={} (unknown counts as unflagged)",
            counts[0], counts[1], counts[2], counts[3]
        );
        assert_eq!(
            counts[1], 0,
            "valid evidence must never be shell-only in this corpus"
        );
    }
}

#[test]
fn root_recovery_is_limited_to_a_known_shell_and_first_article() {
    let cases: Vec<Case> = serde_json::from_str(include_str!(
        "../../tests/fixtures/content-quality/development.json"
    ))
    .unwrap();
    let case = cases
        .iter()
        .find(|case| case.id == "wrong-main-root")
        .unwrap();
    let mut document = Html::parse_document(case.html.as_deref().unwrap());
    remove_page_chrome(&mut document);
    let old_text = clean_text(&extract_weighted_text(main_content(&document)), 20_000);
    assert_eq!(
        old_text.as_deref(),
        Some("Post a comment Your email address will not be published.")
    );
    assert_ne!(old_text, case.extracted);
    for main in [
        "A useful answer already exists here.",
        "Something went wrong.",
    ] {
        let html = format!(
            "<main><p>{main}</p></main><article><p>Do not replace the original root with this unrelated article.</p></article>"
        );
        assert_eq!(parse_content(&html, 20_000).as_deref(), Some(main));
    }
    let html = "<main><h1>Your browser is not supported</h1></main><article><h1>Your browser is not supported</h1></article><article><p>A second article must not trigger unbounded fallback attempts.</p></article>";
    assert_eq!(
        parse_content(html, 20_000).as_deref(),
        Some("Your browser is not supported")
    );
}

#[test]
fn quality_recovery_respects_assessment_bytes_and_extraction_characters() {
    let shell = "Your browser is not supported";
    let html = format!(
        "<main><h1>{shell}</h1></main><article><p>{}</p></article>",
        "é".repeat(17_000)
    );
    assert_eq!(parse_content(&html, 20_000).as_deref(), Some(shell));
    // A truncated message is not proof of a shell, so no fallback is attempted.
    assert_eq!(parse_content(&html, 7).as_deref(), Some("Your br"));
}
