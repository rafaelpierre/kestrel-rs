//! Bing request and HTML adapter.
use super::{
    html::{element_text, selector},
    response::extract_completed,
};
use crate::search::ProviderFailure;
use crate::{
    model::{Engine, SearchResult, TimeFilter},
    search::{ProviderResponse, transport::request_standard_with_retries},
};
use base64::Engine as _;
use scraper::Html;
use std::collections::HashMap;
use url::Url;

// Keep production and request-construction regressions on the same path.
pub(crate) fn bing_request(
    client: &crate::http_client::Client,
    query: &str,
    region: &str,
) -> reqwest::RequestBuilder {
    let mut params = vec![("q", query)];
    let country = region
        .split_once('-')
        .map_or(region, |(country, _)| country);
    if !country.is_empty() {
        params.push(("cc", country));
    }
    // Serialize first so literal '+' becomes %2B before form-encoded spaces
    // become %20. This matches browser space encoding without changing values.
    let encoded = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(params)
        .finish()
        .replace('+', "%20");
    client.get(format!("https://www.bing.com/search?{encoded}"))
}

pub(crate) async fn search_bing(
    query: &str,
    region: &str,
    time_filter: TimeFilter,
    client: &crate::http_client::Client,
) -> Result<ProviderResponse, ProviderFailure> {
    if time_filter != TimeFilter::Any {
        crate::log_event!(
            "search_filter_unsupported",
            "engine" => "bing",
            "query" => query,
            "filter" => "time_filter",
            "value" => time_filter.as_str(),
        );
    }
    let (results, retries) =
        request_standard_with_retries(client, Engine::Bing, query, extract_completed, || {
            bing_request(client, query, region)
        })
        .await?;
    Ok(ProviderResponse {
        results: results?,
        retries,
        raw_result_count: 0,
    })
}

#[cfg(test)]
pub(crate) fn parse_bing_results(html: &str) -> Vec<SearchResult> {
    parse_bing_document(&Html::parse_document(html))
}

pub(crate) fn parse_bing_document(document: &Html) -> Vec<SearchResult> {
    let item = selector("li.b_algo");
    let title = selector("h2 a");
    let snippet = selector(".b_caption p");
    let display = selector(".b_attribution cite, cite");
    document
        .select(&item)
        .filter_map(|entry| {
            let link = entry.select(&title).next()?;
            Some(SearchResult::parsed(
                element_text(link, " "),
                decode_bing_url(link.value().attr("href").unwrap_or_default()),
                entry
                    .select(&display)
                    .next()
                    .map_or_else(String::new, |value| element_text(value, " ")),
                entry
                    .select(&snippet)
                    .next()
                    .map_or_else(String::new, |value| element_text(value, " ")),
            ))
        })
        .collect()
}

fn decode_bing_url(value: &str) -> String {
    if !value.contains("/ck/a") && !value.contains("/cr?") {
        return value.to_owned();
    }
    let Ok(url) = Url::parse(value) else {
        return value.to_owned();
    };
    let params: HashMap<_, _> = url.query_pairs().into_owned().collect();
    if let Some(target) = params.get("rurl") {
        return target.to_owned();
    }
    let Some(encoded) = params.get("u") else {
        return value.to_owned();
    };
    let encoded = encoded.strip_prefix("a1").unwrap_or(encoded);
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(encoded))
        .ok()
        .and_then(|decoded| String::from_utf8(decoded).ok())
        .unwrap_or_else(|| value.to_owned())
}
