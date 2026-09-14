//! Worker-local response classification and completed adapter dispatch.
use super::{
    bing::parse_bing_document, duckduckgo::parse_duckduckgo_document, html::selector,
    yahoo::parse_yahoo_document,
};
use crate::search::ProviderFailure;
use crate::{
    model::{Engine, SearchResult},
    provider_diagnostics::Challenge,
};
use scraper::Html;
#[cfg(test)]
thread_local! {
    pub(crate) static RESPONSE_PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Worker-local representation: scraper DOMs never cross an async boundary.
pub(crate) enum ParsedResponse {
    Html(Html),
    Json(Option<serde_json::Value>),
}

impl ParsedResponse {
    pub(crate) fn new(engine: Engine, text: &str) -> Self {
        #[cfg(test)]
        RESPONSE_PARSES.with(|count| count.set(count.get() + 1));
        if matches!(
            engine,
            Engine::Dogpile | Engine::Yep | Engine::Swisscows | Engine::Qwant
        ) && matches!(text.trim_start().as_bytes().first(), Some(b'{' | b'['))
        {
            Self::Json(serde_json::from_str(text).ok())
        } else {
            Self::Html(Html::parse_document(text))
        }
    }

    pub(crate) fn challenge(&self, engine: Engine, text: &str) -> Challenge {
        if text.trim().is_empty() {
            return Challenge::Unknown;
        }
        match self {
            Self::Json(data) => {
                if engine == Engine::Qwant
                    && data
                        .as_ref()
                        .is_some_and(|v| v.get("url").and_then(|v| v.as_str()).is_some())
                {
                    Challenge::Detected
                } else {
                    Challenge::NotDetected
                }
            }
            Self::Html(document) => classify_html_challenge(engine, document),
        }
    }
}

#[cfg(test)]
pub(crate) fn classify_challenge(engine: Engine, text: &str) -> Challenge {
    ParsedResponse::new(engine, text).challenge(engine, text)
}

fn classify_html_challenge(engine: Engine, document: &Html) -> Challenge {
    if document.select(&selector("#b_captcha, #captcha, form[action*='captcha'], .g-recaptcha, #challenge-form, #cf-challenge-running, form[action*='anomaly.js'], .anomaly-modal")).next().is_some() {
        return Challenge::Detected;
    }
    if engine == Engine::Mojeek && crate::providers::mojeek_challenge(document) {
        return Challenge::Detected;
    }
    if engine == Engine::Ecosia
        && document
            .select(&selector("title"))
            .any(|e| e.text().collect::<String>().contains("Firewall"))
    {
        return Challenge::Detected;
    }
    // This means no known marker was detected, not proof the provider is usable.
    Challenge::NotDetected
}

pub(crate) fn extract_completed(
    engine: Engine,
    _text: &str,
    parsed: &ParsedResponse,
) -> Result<Vec<SearchResult>, ProviderFailure> {
    match (engine, parsed) {
        (Engine::Duckduckgo, ParsedResponse::Html(doc)) => parse_duckduckgo_document(doc),
        (Engine::Ecosia | Engine::Mojeek, ParsedResponse::Html(doc)) => {
            crate::providers::parse_html(engine, doc)
        }
        _ => extract_dispatched(engine, _text, parsed),
    }
}

pub(crate) fn extract_dispatched(
    engine: Engine,
    _text: &str,
    parsed: &ParsedResponse,
) -> Result<Vec<SearchResult>, ProviderFailure> {
    if matches!(
        engine,
        Engine::Dogpile | Engine::Yep | Engine::Swisscows | Engine::Qwant
    ) {
        return match parsed {
            ParsedResponse::Json(Some(data)) => crate::providers::parse_json(engine, data),
            _ => Err(crate::providers::unrecognized(engine)),
        };
    }
    match parsed {
        ParsedResponse::Html(doc) => parse_provider_document(engine, doc),
        _ => Err(crate::providers::unrecognized(engine)),
    }
}

/// Run classification and the caller's extraction in one bounded worker. The
/// returned value is Send, so the worker-local DOM cannot escape its permit.
/// HTTP failures need diagnostics only; extraction failures on successful HTTP
/// responses stay inside T and do not enter the transport retry policy.
pub(crate) fn process_completed<T>(
    engine: Engine,
    text: &str,
    success: bool,
    extract: fn(Engine, &str, &ParsedResponse) -> T,
) -> (Challenge, Option<T>) {
    let document = ParsedResponse::new(engine, text);
    let challenge = document.challenge(engine, text);
    (challenge, success.then(|| extract(engine, text, &document)))
}

#[cfg(test)]
pub(crate) fn retain_body(_engine: Engine, text: &str, _parsed: &ParsedResponse) -> String {
    text.to_owned()
}

pub(crate) fn parse_provider_response(
    engine: Engine,
    html: &str,
) -> Result<Vec<SearchResult>, ProviderFailure> {
    extract_dispatched(engine, html, &ParsedResponse::new(engine, html))
}

fn parse_provider_document(
    engine: Engine,
    document: &Html,
) -> Result<Vec<SearchResult>, ProviderFailure> {
    if document
        .select(&selector(
            "#b_captcha, #captcha, form[action*='captcha'], .g-recaptcha",
        ))
        .next()
        .is_some()
    {
        return Err(ProviderFailure::challenge(format!(
            "{engine} returned a bot challenge"
        )));
    }
    let results = match engine {
        Engine::Bing => parse_bing_document(document),
        Engine::Yahoo => parse_yahoo_document(document),
        Engine::Duckduckgo => return parse_duckduckgo_document(document),
        _ => return crate::providers::parse_html(engine, document),
    };
    let empty_marker = match engine {
        Engine::Bing => "li.b_no, .b_no",
        Engine::Yahoo => ".msgNoResults, .no-results",
        _ => unreachable!(),
    };
    if results.is_empty() && document.select(&selector(empty_marker)).next().is_none() {
        return Err(ProviderFailure::unrecognized(format!(
            "{engine} returned an unrecognized search page"
        )));
    }
    Ok(results)
}
