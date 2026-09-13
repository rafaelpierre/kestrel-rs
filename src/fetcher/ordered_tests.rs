use super::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    id: String,
    html: String,
    expected: Option<String>,
}

#[test]
fn ordered_html_golden_fixtures_and_every_character_boundary() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("../../tests/fixtures/extraction/ordered.json")).unwrap();
    for case in cases {
        assert_eq!(
            parse_content(&case.html, 20_000),
            case.expected,
            "{}",
            case.id
        );
        if let Some(expected) = case.expected {
            // Includes multibyte scalars, generated separators, and code whitespace.
            for limit in 0..=expected.chars().count() + 1 {
                let prefix: String = expected.chars().take(limit).collect();
                let prefix = prefix.chars().any(|c| !c.is_whitespace()).then_some(prefix);
                assert_eq!(
                    parse_content(&case.html, limit),
                    prefix,
                    "{} at {limit}",
                    case.id
                );
            }
        }
    }
}

#[test]
fn deeply_nested_wrappers_do_not_duplicate_text_or_recurse() {
    let html = format!(
        "<main>{}<p>Retained</p>{}</main>",
        "<div>".repeat(4000),
        "</div>".repeat(4000)
    );
    assert_eq!(parse_content(&html, 8).as_deref(), Some("Retained"));
}
