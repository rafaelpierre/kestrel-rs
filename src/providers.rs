//! Opt-in provider adapters. Wire formats are isolated from orchestration.

use std::collections::BTreeMap;

use base64::Engine as _;
use scraper::{Html, Selector};
use serde_json::Value;
use sha2::{Digest, Sha256};
use url::Url;

use crate::model::{Engine, SearchResult, TimeFilter};
use crate::search::KestrelError;

/// Browser-facing URL, useful for inspecting the exact query independently.
pub fn search_url(engine: Engine, query: &str) -> Option<Url> {
    let (base, parameter) = match engine {
        Engine::Dogpile => ("https://www.dogpile.com/serp", "q"),
        Engine::Ecosia => ("https://www.ecosia.org/search", "q"),
        Engine::Swisscows => ("https://swisscows.com/en/web", "query"),
        Engine::Yep => ("https://yep.com/web", "q"),
        Engine::Qwant => ("https://www.qwant.com/", "q"),
        Engine::Mojeek => ("https://www.mojeek.com/search", "q"),
        _ => return None,
    };
    let mut url = Url::parse(base).expect("constant provider URL");
    url.query_pairs_mut().append_pair(parameter, query);
    if engine == Engine::Qwant {
        url.query_pairs_mut().append_pair("t", "web");
    }
    Some(url)
}

pub(crate) fn request(
    client: &reqwest::Client,
    engine: Engine,
    query: &str,
    region: &str,
    time: TimeFilter,
) -> Result<reqwest::RequestBuilder, KestrelError> {
    // Do not guess UI parameter names or silently ignore explicit filters.
    if (!matches!(engine, Engine::Swisscows | Engine::Qwant) && !region.is_empty())
        || (!matches!(engine, Engine::Swisscows | Engine::Mojeek) && time != TimeFilter::Any)
    {
        return Err(KestrelError::InvalidRequest(format!(
            "{engine} adapter does not yet support --region or --time-filter; use native query operators where documented"
        )));
    }
    let request = match engine {
        Engine::Dogpile => client
            .post("https://www.dogpile.com/api/search")
            .header("Origin", "https://www.dogpile.com")
            .header("Content-Type", "application/json")
            .body(serde_json::json!({"q": query, "qadf": "moderate", "page": 1}).to_string()),
        Engine::Ecosia => client.get(search_url(engine, query).expect("Ecosia URL")),
        Engine::Yep => client
            .get("https://api.yep.com/search")
            .header("Origin", "https://yep.com")
            .header("Referer", "https://yep.com/")
            .query(&[
                ("query", query),
                ("safeSearch", "moderate"),
                ("limit", "20"),
            ]),
        Engine::Qwant => {
            let locale = swiss_locale(region)?.replace('-', "_");
            client
                .get("https://api.qwant.com/v3/search/web")
                .header("Origin", "https://www.qwant.com")
                .header("Referer", "https://www.qwant.com/")
                .header("Accept", "application/json")
                .query(&[
                    ("q", query),
                    ("count", "10"),
                    ("locale", locale.as_str()),
                    ("offset", "0"),
                    ("device", "desktop"),
                    ("safesearch", "1"),
                    ("displayed", "true"),
                    ("llm", "false"),
                ])
        }
        Engine::Mojeek => {
            let mut url = search_url(engine, query).expect("Mojeek URL");
            let since = match time {
                TimeFilter::Any => None,
                TimeFilter::D => Some("day".to_owned()),
                TimeFilter::W => Some(
                    (chrono::Utc::now() - chrono::Duration::days(7))
                        .format("%Y%m%d")
                        .to_string(),
                ),
                TimeFilter::M => Some("month".to_owned()),
                TimeFilter::Y => Some("year".to_owned()),
            };
            if let Some(since) = since {
                url.query_pairs_mut().append_pair("since", &since);
            }
            client.get(url)
        }
        Engine::Swisscows => {
            let locale = swiss_locale(region)?;
            let freshness = match time {
                TimeFilter::Any => "All",
                TimeFilter::D => "Day",
                TimeFilter::W => "Week",
                TimeFilter::M => "Month",
                TimeFilter::Y => "Year",
            };
            let params = BTreeMap::from([
                ("freshness", freshness),
                ("itemsCount", "20"),
                ("locale", locale.as_str()),
                ("offset", "0"),
                ("query", query),
                ("spellcheck", "false"),
            ]);
            let nonce = uuid::Uuid::new_v4().simple().to_string();
            let signature = swiss_signature("/v5/web/search", &params, &nonce);
            client
                .get("https://api.swisscows.com/v5/web/search")
                .query(&params)
                .header("X-Request-Nonce", nonce)
                .header("X-Request-Signature", signature)
                .header(
                    "X-Referer",
                    search_url(engine, query).expect("Swisscows URL").as_str(),
                )
        }
        _ => {
            return Err(KestrelError::InvalidRequest(
                "not an additional provider".into(),
            ));
        }
    };
    Ok(
        if matches!(
            engine,
            Engine::Dogpile | Engine::Yep | Engine::Qwant | Engine::Swisscows
        ) {
            request
                .header("accept", "application/json")
                .header("sec-fetch-mode", "cors")
                .header("sec-fetch-dest", "")
                .header(
                    "sec-fetch-site",
                    if engine == Engine::Dogpile {
                        "same-origin"
                    } else {
                        "same-site"
                    },
                )
        } else {
            request
        },
    )
}

fn swiss_locale(region: &str) -> Result<String, KestrelError> {
    if region.is_empty() {
        return Ok("en-US".into());
    }
    let Some((country, language)) = region.split_once('-') else {
        return Err(KestrelError::InvalidRequest(
            "Swisscows region must be country-language, e.g. us-en".into(),
        ));
    };
    let country = if country.eq_ignore_ascii_case("uk") {
        "GB".into()
    } else {
        country.to_ascii_uppercase()
    };
    const COUNTRIES: &[&str] = &[
        "AR", "AU", "AT", "BE", "BR", "CA", "CL", "CN", "DK", "FI", "FR", "DE", "HK", "HU", "IN",
        "ID", "IT", "JP", "KR", "LV", "MY", "MX", "NL", "NZ", "NO", "PH", "PL", "PT", "RU", "SA",
        "ZA", "ES", "SE", "CH", "TW", "TR", "UA", "GB", "US",
    ];
    if !COUNTRIES.contains(&country.as_str())
        || language.len() != 2
        || !language.bytes().all(|b| b.is_ascii_alphabetic())
    {
        return Err(KestrelError::InvalidRequest(
            "unsupported Swisscows region; expected e.g. us-en or uk-en".into(),
        ));
    }
    Ok(format!("{}-{country}", language.to_ascii_lowercase()))
}

// Public website request signing: sorted, unescaped parameters plus transformed nonce.
// This is not an authentication credential; the browser generates it per request.
fn swiss_signature(path: &str, params: &BTreeMap<&str, &str>, nonce: &str) -> String {
    let suffix: String = nonce
        .bytes()
        .map(|c| match c {
            b'a'..=b'z' => ((c - b'a' + 13) % 26 + b'A') as char,
            b'A'..=b'Z' => ((c - b'A' + 13) % 26 + b'a') as char,
            _ => c as char,
        })
        .collect();
    let args = params
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(format!("{path}?{args}{suffix}")))
}

pub(crate) fn parse(engine: Engine, text: &str) -> Result<Vec<SearchResult>, KestrelError> {
    if engine == Engine::Mojeek {
        return parse_mojeek(text);
    }
    if engine == Engine::Qwant {
        return parse_qwant(text);
    }
    if engine == Engine::Ecosia {
        return parse_ecosia(text);
    }
    let bad = || {
        KestrelError::Search(format!(
            "{engine} returned an unrecognized search page or response"
        ))
    };
    let mut data: Value = serde_json::from_str(text).map_err(|_| bad())?;
    if engine == Engine::Yep && data.get(0).and_then(Value::as_str) != Some("Ok") {
        return Err(bad());
    }
    if engine == Engine::Swisscows
        && let Some(payload) = data.get("payload").and_then(Value::as_str)
    {
        // Decode the transport envelope, not an identity token. HTTPS authenticates transport.
        let part = payload.split('.').nth(1).ok_or_else(bad)?;
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(part.trim_end_matches('='))
            .map_err(|_| bad())?;
        data = serde_json::from_slice(&bytes).map_err(|_| bad())?;
    }
    let array = match engine {
        Engine::Dogpile => data.get("results"),
        Engine::Yep => data.get(1).and_then(|v| v.get("results")),
        Engine::Swisscows => data.get("items"),
        _ => None,
    }
    .and_then(Value::as_array)
    .ok_or_else(bad)?;
    let mut results = Vec::new();
    let mut web_items = 0;
    for item in array {
        if engine == Engine::Swisscows
            && item.get("type").and_then(Value::as_str) != Some("WebPage")
        {
            continue;
        }
        web_items += 1;
        let (url_key, title_key, snippet_key) = match engine {
            Engine::Dogpile => ("clickUrl", "title", "description"),
            Engine::Swisscows => ("url", "name", "description"),
            _ => ("url", "title", "snippet"),
        };
        let (Some(url), Some(title)) = (
            item.get(url_key).and_then(Value::as_str),
            item.get(title_key).and_then(Value::as_str),
        ) else {
            continue;
        };
        if let Some(result) = parsed(
            title,
            url,
            item.get(snippet_key)
                .and_then(Value::as_str)
                .unwrap_or_default(),
        ) {
            results.push(result);
        }
    }
    if web_items > 0 && results.is_empty() {
        return Err(bad());
    }
    Ok(results)
}

fn plain(value: &str) -> String {
    Html::parse_fragment(value)
        .root_element()
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn parsed(title: &str, value: &str, snippet: &str) -> Option<SearchResult> {
    let url = Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let display = url.host_str()?.to_owned();
    let title = plain(title);
    if title.is_empty() {
        return None;
    }
    Some(SearchResult::parsed(
        title,
        url.into(),
        display,
        plain(snippet),
    ))
}

fn parse_ecosia(text: &str) -> Result<Vec<SearchResult>, KestrelError> {
    let doc = Html::parse_document(text);
    let selector = |s| Selector::parse(s).expect("constant selector");
    if doc
        .select(&selector(
            "#challenge-form, #cf-challenge-running, .g-recaptcha",
        ))
        .next()
        .is_some()
        || doc
            .select(&selector("title"))
            .any(|e| e.text().collect::<String>().contains("Firewall"))
    {
        return Err(KestrelError::Search(
            "ecosia returned a bot challenge or firewall".into(),
        ));
    }
    let mut results = Vec::new();
    for item in doc.select(&selector(".result, .web-result")) {
        if item
            .value()
            .classes()
            .any(|c| c == "result--ad" || c == "ad-result")
        {
            continue;
        }
        let Some(link) = item
            .select(&selector("a.result__link, a.result-title, h2 a"))
            .next()
        else {
            continue;
        };
        let snippet = item
            .select(&selector(
                ".result__description, .result__snippet, .web-result-description",
            ))
            .next()
            .map(|e| e.inner_html())
            .unwrap_or_default();
        if let Some(result) = link
            .value()
            .attr("href")
            .and_then(|url| parsed(&link.inner_html(), url, &snippet))
        {
            results.push(result);
        }
    }
    if results.is_empty()
        && doc
            .select(&selector(".no-results, .no-results-message"))
            .next()
            .is_none()
    {
        return Err(KestrelError::Search(
            "ecosia returned an unrecognized search page".into(),
        ));
    }
    Ok(results)
}

fn parse_qwant(text: &str) -> Result<Vec<SearchResult>, KestrelError> {
    let bad =
        || KestrelError::Search("qwant returned an unrecognized search page or response".into());
    let data: Value = serde_json::from_str(text).map_err(|_| bad())?;
    if data.get("url").and_then(Value::as_str).is_some() {
        return Err(KestrelError::Search(
            "qwant returned a bot challenge".into(),
        ));
    }
    if data.get("status").and_then(Value::as_str) != Some("success") {
        return Err(bad());
    }
    let rows = data
        .pointer("/data/result/items/mainline")
        .and_then(Value::as_array)
        .ok_or_else(bad)?;
    let mut results = Vec::new();
    let mut web_items = 0;
    for row in rows {
        if row.get("type").and_then(Value::as_str) != Some("web") {
            continue;
        }
        let items = row.get("items").and_then(Value::as_array).ok_or_else(bad)?;
        web_items += items.len();
        for item in items {
            if let (Some(url), Some(title)) = (
                item.get("url").and_then(Value::as_str),
                item.get("title").and_then(Value::as_str),
            ) && let Some(result) = parsed(
                title,
                url,
                item.get("desc").and_then(Value::as_str).unwrap_or_default(),
            ) {
                results.push(result);
            }
        }
    }
    if web_items > 0 && results.is_empty() {
        return Err(bad());
    }
    Ok(results)
}

pub(crate) fn mojeek_challenge(doc: &Html) -> bool {
    let select = |s| Selector::parse(s).expect("constant selector");
    // The observed challenge has both a page-level title and a dedicated wrapper.
    // Never classify CAPTCHA mentions in ordinary result titles/snippets as blocking.
    let captcha_title = doc.select(&select("head > title")).any(|title| {
        title
            .text()
            .collect::<String>()
            .trim()
            .eq_ignore_ascii_case("captcha")
    });
    let challenge_message = doc.select(&select(".captcha-wrap > p")).any(|message| {
        message
            .text()
            .flat_map(str::split_whitespace)
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase()
            .contains("javascript is required to complete this challenge.")
    });
    captcha_title && challenge_message
}

fn parse_mojeek(text: &str) -> Result<Vec<SearchResult>, KestrelError> {
    let doc = Html::parse_document(text);
    let select = |s| Selector::parse(s).expect("constant selector");
    if mojeek_challenge(&doc) {
        return Err(KestrelError::Search(
            "mojeek returned a bot challenge".into(),
        ));
    }
    let mut results = Vec::new();
    for item in doc.select(&select("ul.results-standard > li")) {
        let Some(link) = item.select(&select("a.ob")).next() else {
            continue;
        };
        let title = item
            .select(&select("h2 a"))
            .next()
            .map(|e| e.inner_html())
            .unwrap_or_default();
        let snippet = item
            .select(&select("p.s"))
            .next()
            .map(|e| e.inner_html())
            .unwrap_or_default();
        if let Some(result) = link
            .value()
            .attr("href")
            .and_then(|url| parsed(&title, url, &snippet))
        {
            results.push(result);
        }
    }
    if results.is_empty() {
        // An empty list alone is not proof of a genuine no-results response.
        let no_results = doc.select(&select(".top-info")).any(|e| {
            e.text()
                .collect::<String>()
                .to_lowercase()
                .contains("no results")
        });
        if !no_results {
            return Err(KestrelError::Search(
                "mojeek returned an unrecognized search page".into(),
            ));
        }
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mojeek_distinguishes_challenges_results_empty_and_unknown() {
        let challenge = include_str!("../tests/fixtures/providers/mojeek-challenge.html");
        assert!(
            parse(Engine::Mojeek, challenge)
                .unwrap_err()
                .to_string()
                .contains("bot challenge")
        );
        assert!(
            parse(
                Engine::Mojeek,
                "<div class='top-info'>No results found.</div>"
            )
            .unwrap()
            .is_empty()
        );
        for unknown in [
            "<html><body>Service unavailable</body></html>".to_owned(),
            "<ul class='results-standard'></ul>".to_owned(),
            challenge.replace("<title>Captcha</title>", "<title>Search</title>"),
            challenge.replace("captcha-wrap", "article"),
            challenge.replace(
                "JavaScript is required to complete this challenge.",
                "CAPTCHA documentation",
            ),
        ] {
            assert!(
                parse(Engine::Mojeek, &unknown)
                    .unwrap_err()
                    .to_string()
                    .contains("unrecognized search page")
            );
        }
        let results = r#"<html><head><title>Captcha</title></head><body>
            <ul class="results-standard"><li>
            <h2><a class="ob" href="https://example.com/captcha">Captcha</a></h2>
            <p class="s">JavaScript is required to complete this challenge. CAPTCHA troubleshooting.</p>
            </li></ul></body></html>"#;
        assert_eq!(parse(Engine::Mojeek, results).unwrap().len(), 1);
        // A challenge must not be accepted as explicit empty or partial results.
        for suffix in [results, "<div class='top-info'>No results found.</div>"] {
            assert!(
                parse(
                    Engine::Mojeek,
                    &challenge.replace("</body>", &format!("{suffix}</body>"))
                )
                .unwrap_err()
                .to_string()
                .contains("bot challenge")
            );
        }
    }

    #[test]
    fn exact_queries_survive_browser_and_transport_encoding() {
        let client = reqwest::Client::new();
        for query in [
            "\"machine learning\"",
            "machine AND learning",
            "machine learning",
            "(Rust OR Python) NOT game",
            r#""say \"hello\"""#,
            "site:postgresql.org EXPLAIN ANALYZE BUFFERS",
            "\"C++ & Rust\" -game OR café",
            "Unicode 日本語 #tag +plus %20",
        ] {
            for engine in [
                Engine::Dogpile,
                Engine::Ecosia,
                Engine::Swisscows,
                Engine::Yep,
                Engine::Qwant,
                Engine::Mojeek,
            ] {
                let url = search_url(engine, query).unwrap();
                let key = if engine == Engine::Swisscows {
                    "query"
                } else {
                    "q"
                };
                assert_eq!(url.query_pairs().find(|(k, _)| k == key).unwrap().1, query);
                let req = request(&client, engine, query, "", TimeFilter::Any)
                    .unwrap()
                    .build()
                    .unwrap();
                if engine == Engine::Dogpile {
                    let body: Value =
                        serde_json::from_slice(req.body().unwrap().as_bytes().unwrap()).unwrap();
                    assert_eq!(body["q"], query);
                    assert_eq!(req.method(), reqwest::Method::POST);
                } else {
                    let key = if matches!(engine, Engine::Ecosia | Engine::Qwant | Engine::Mojeek) {
                        "q"
                    } else {
                        "query"
                    };
                    assert_eq!(
                        req.url().query_pairs().find(|(k, _)| k == key).unwrap().1,
                        query
                    );
                }
            }
        }
    }

    #[test]
    fn parses_provider_specific_contracts() {
        for (engine, fixture) in [
            (
                Engine::Dogpile,
                include_str!("../tests/fixtures/providers/dogpile.json"),
            ),
            (
                Engine::Ecosia,
                include_str!("../tests/fixtures/providers/ecosia.html"),
            ),
            (
                Engine::Swisscows,
                include_str!("../tests/fixtures/providers/swisscows.json"),
            ),
            (
                Engine::Yep,
                include_str!("../tests/fixtures/providers/yep.json"),
            ),
            (
                Engine::Qwant,
                include_str!("../tests/fixtures/providers/qwant.json"),
            ),
            (
                Engine::Mojeek,
                include_str!("../tests/fixtures/providers/mojeek.html"),
            ),
        ] {
            let results = parse(engine, fixture).unwrap();
            assert!(!results.is_empty());
            assert!(
                results
                    .iter()
                    .all(|r| !r.title.contains('<') && !r.snippet.contains('<'))
            );
            assert!(results.iter().all(|r| r.url.starts_with("https://")));
        }
        assert_eq!(
            parse(
                Engine::Ecosia,
                include_str!("../tests/fixtures/providers/ecosia.html")
            )
            .unwrap()
            .len(),
            1
        );
    }

    #[test]
    fn envelope_and_response_errors_are_not_empty_success() {
        let fixture = include_str!("../tests/fixtures/providers/swisscows.json");
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(fixture);
        assert_eq!(
            parse(
                Engine::Swisscows,
                &serde_json::json!({"payload":format!("header.{payload}.signature")}).to_string()
            )
            .unwrap()
            .len(),
            2
        );
        for engine in [
            Engine::Dogpile,
            Engine::Ecosia,
            Engine::Swisscows,
            Engine::Yep,
            Engine::Qwant,
            Engine::Mojeek,
        ] {
            assert!(parse(engine, "<html><nav>Home</nav></html>").is_err());
            assert!(parse(engine, r#"{"error":"unavailable"}"#).is_err());
        }
        assert!(parse(Engine::Swisscows, r#"{"payload":"invalid"}"#).is_err());
        assert!(
            parse(Engine::Dogpile, r#"{"results":[]}"#)
                .unwrap()
                .is_empty()
        );
        assert!(
            parse(Engine::Yep, r#"["Ok",{"results":[]}]"#)
                .unwrap()
                .is_empty()
        );
        assert!(
            parse(Engine::Swisscows, r#"{"items":[]}"#)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn region_and_time_filters_are_explicit() {
        let client = reqwest::Client::new();
        let req = request(&client, Engine::Swisscows, "query", "uk-en", TimeFilter::W)
            .unwrap()
            .build()
            .unwrap();
        let pairs: BTreeMap<_, _> = req.url().query_pairs().collect();
        assert_eq!(pairs["locale"], "en-GB");
        assert_eq!(pairs["freshness"], "Week");
        assert_eq!(pairs["spellcheck"], "false");
        assert!(req.headers().contains_key("X-Request-Signature"));
        for engine in [Engine::Dogpile, Engine::Ecosia, Engine::Yep] {
            assert!(request(&client, engine, "q", "", TimeFilter::D).is_err());
            assert!(request(&client, engine, "q", "us-en", TimeFilter::Any).is_err());
        }
    }
}
