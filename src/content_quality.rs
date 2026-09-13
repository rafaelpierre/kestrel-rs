//! Conservative, advisory assessment of retained text, independent of HTTP outcomes.
//!
//! This recognizes a small English message vocabulary, not semantic usefulness.
//! It never rejects content or changes ranking. Unknown and unflagged results
//! must not be interpreted as evidence that a page answers a query.

use serde::{Deserialize, Serialize};

/// Maximum UTF-8 input size assessed. Larger inputs remain unknown; no prefix is judged.
pub const CONTENT_QUALITY_MAX_BYTES: usize = 32_768;
/// Version of the message vocabulary and policy, independent of extraction/cache versions.
pub const CONTENT_QUALITY_VERSION: u32 = 1;

/// Advisory state of the retained text, not of the original complete page.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentQualityState {
    /// No known shell message was found. This is not a claim of usefulness.
    Unflagged,
    /// The entire normalized text consists of recognized shell messages.
    BoilerplateOnly,
    /// Missing, over-limit, mixed, or insufficiently specific evidence.
    Unknown,
}

/// Stable explanations for an advisory assessment.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentQualityReason {
    NoText,
    AssessmentLimitExceeded,
    NoKnownShell,
    BrowserErrorMessages,
    CommentMessages,
    MixedContent,
    InsufficientSignals,
}

/// Recomputed from retained text; never persisted in the page cache.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContentQuality {
    pub version: u32,
    pub state: ContentQualityState,
    pub reasons: Vec<ContentQualityReason>,
}

// Whole messages, including punctuation. Never match arbitrary error substrings,
// site names, article titles, or class-name fragments. Generic failures and
// comment prompts need corroboration from a different message.
const MESSAGES: &[(&str, ContentQualityReason, bool)] = &[
    (
        "your browser is not supported.",
        ContentQualityReason::BrowserErrorMessages,
        true,
    ),
    (
        "your browser is not supported",
        ContentQualityReason::BrowserErrorMessages,
        true,
    ),
    (
        "please update your browser.",
        ContentQualityReason::BrowserErrorMessages,
        true,
    ),
    (
        "please update your browser",
        ContentQualityReason::BrowserErrorMessages,
        true,
    ),
    (
        "something went wrong.",
        ContentQualityReason::BrowserErrorMessages,
        false,
    ),
    (
        "something went wrong",
        ContentQualityReason::BrowserErrorMessages,
        false,
    ),
    (
        "please try again later.",
        ContentQualityReason::BrowserErrorMessages,
        false,
    ),
    (
        "please try again later",
        ContentQualityReason::BrowserErrorMessages,
        false,
    ),
    (
        "leave a reply",
        ContentQualityReason::CommentMessages,
        false,
    ),
    (
        "post a comment",
        ContentQualityReason::CommentMessages,
        false,
    ),
    (
        "your email address will not be published.",
        ContentQualityReason::CommentMessages,
        false,
    ),
    (
        "required fields are marked *",
        ContentQualityReason::CommentMessages,
        false,
    ),
    (
        "you can use some html tags, such as <b>, <i>, <a>.",
        ContentQualityReason::CommentMessages,
        false,
    ),
];

/// Assess at most 32 KiB of retained text with exact, whitespace-normalized,
/// ASCII-case-insensitive message matching. Punctuation, quotation marks, code,
/// and other text are not stripped. Any unmatched text prevents a shell verdict.
/// Byte/character truncation and request success must be assessed separately.
///
/// Ordinary text is unflagged rather than certified useful. A shell-only excerpt
/// may have come from a useful full page; this function cannot infer missing HTML.
pub fn assess_content_quality(content: Option<&str>) -> ContentQuality {
    use ContentQualityReason as Reason;
    use ContentQualityState as State;
    let assessment = |state, reasons| ContentQuality {
        version: CONTENT_QUALITY_VERSION,
        state,
        reasons,
    };
    let Some(content) = content else {
        return assessment(State::Unknown, vec![Reason::NoText]);
    };
    if content.len() > CONTENT_QUALITY_MAX_BYTES {
        return assessment(State::Unknown, vec![Reason::AssessmentLimitExceeded]);
    }
    let normalized = content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    if normalized.is_empty() {
        return assessment(State::Unknown, vec![Reason::NoText]);
    }
    let mut remaining = normalized.as_str();
    let mut reasons = Vec::new();
    let mut distinct = Vec::new();
    let mut specific = false;
    while !remaining.is_empty() {
        let found = MESSAGES.iter().find(|(message, _, _)| {
            remaining
                .strip_prefix(message)
                .is_some_and(|tail| tail.is_empty() || tail.starts_with(' '))
        });
        let Some((message, reason, is_specific)) = found else {
            break;
        };
        if !reasons.contains(reason) {
            reasons.push(*reason);
        }
        // Punctuation variants count as one signal, as do repeated headings.
        let identity = message.trim_end_matches('.');
        if !distinct.contains(&identity) {
            distinct.push(identity);
        }
        specific |= is_specific;
        remaining = remaining[message.len()..].trim_start();
    }
    if remaining.is_empty() {
        return if specific || distinct.len() >= 2 {
            assessment(State::BoilerplateOnly, reasons)
        } else {
            reasons.push(Reason::InsufficientSignals);
            assessment(State::Unknown, reasons)
        };
    }
    // A recognized message elsewhere is mixed evidence, never a reason to reject
    // surrounding prose. Word boundaries prevent incidental substring matches.
    let mixed = MESSAGES.iter().any(|(message, _, _)| {
        normalized.match_indices(message).any(|(start, matched)| {
            (start == 0 || normalized[..start].ends_with(' '))
                && (start + matched.len() == normalized.len()
                    || normalized[start + matched.len()..].starts_with(' '))
        })
    });
    if mixed {
        assessment(State::Unknown, vec![Reason::MixedContent])
    } else {
        assessment(State::Unflagged, vec![Reason::NoKnownShell])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assessment_bounds_and_absence_are_explicit() {
        let _telemetry = crate::telemetry::test_export_guard();
        for text in [None, Some(""), Some(" \n\t")] {
            assert_eq!(
                assess_content_quality(text).reasons,
                [ContentQualityReason::NoText]
            );
        }
        let text = "x".repeat(CONTENT_QUALITY_MAX_BYTES);
        assert_eq!(
            assess_content_quality(Some(&text)).state,
            ContentQualityState::Unflagged
        );
        assert_eq!(
            assess_content_quality(Some(&(text + "é"))).reasons,
            [ContentQualityReason::AssessmentLimitExceeded]
        );
    }

    #[test]
    fn near_matches_and_mixed_text_are_not_shell_only() {
        let _telemetry = crate::telemetry::test_export_guard();
        for text in [
            "Your browser is not supported. Evidence follows.",
            "Evidence precedes. Your browser is not supported.",
            "Your browser is not supported.example",
            "notyour browser is not supported.",
            "your browser is not supported!",
            "Something went wrong. Something went wrong",
            "your browser is not supp",
        ] {
            assert_ne!(
                assess_content_quality(Some(text)).state,
                ContentQualityState::BoilerplateOnly,
                "{text}"
            );
        }
    }

    #[test]
    fn reports_use_content_input_order_and_results_strip_only_their_own_wrapper() {
        let _telemetry = crate::telemetry::test_export_guard();
        let report = crate::FetchReport {
            contents: vec![Some("Your browser is not supported.".into()), None],
            pages: vec![],
            budget_exhausted: true,
            cancelled: 1,
            cache_hits: 0,
            cache_misses: 2,
        };
        assert_eq!(
            report.content_quality(0).unwrap().state,
            ContentQualityState::BoilerplateOnly
        );
        assert_eq!(
            report.content_quality(1).unwrap().state,
            ContentQualityState::Unknown
        );
        assert_eq!(report.content_quality(2), None);
        let mut result = crate::SearchResult::parsed(
            "Title".into(),
            "https://example.com/page".into(),
            String::new(),
            "Snippet".into(),
        );
        result.content =
            Some("Source: https://example.com/page\n\nYour browser is not supported.".into());
        assert_eq!(
            result.content_quality().state,
            ContentQualityState::BoilerplateOnly
        );
        result.url = "https://example.com/other".into();
        assert_eq!(result.content_quality().state, ContentQualityState::Unknown);
    }
}
