//! DuckDuckGo request and HTML adapter.
use super::{
    html::{element_text, selector},
    response::extract_completed,
};
use crate::search::ProviderFailure;
use crate::{
    model::{Engine, SearchResult, TimeFilter},
    search::{ProviderResponse, transport::request_standard_with_retries},
};
use scraper::Html;

pub(crate) fn duckduckgo_request(
    client: &crate::http_client::Client,
    query: &str,
    region: &str,
    time_filter: TimeFilter,
) -> reqwest::RequestBuilder {
    let mut data = vec![("q", query)];
    if !region.is_empty() {
        data.push(("kl", region));
    }
    if time_filter != TimeFilter::Any {
        data.push(("df", time_filter.as_str()));
    }
    client.post("https://html.duckduckgo.com/html/").form(&data)
}

pub(crate) async fn search_duckduckgo(
    query: &str,
    region: &str,
    time_filter: TimeFilter,
    client: &crate::http_client::Client,
) -> Result<ProviderResponse, ProviderFailure> {
    let (results, retries) =
        request_standard_with_retries(client, Engine::Duckduckgo, query, extract_completed, || {
            duckduckgo_request(client, query, region, time_filter)
        })
        .await?;
    Ok(ProviderResponse {
        results: results?,
        retries,
        raw_result_count: 0,
    })
}

#[cfg(test)]
pub(crate) fn parse_duckduckgo_response(html: &str) -> Result<Vec<SearchResult>, ProviderFailure> {
    parse_duckduckgo_document(&Html::parse_document(html))
}

pub(crate) fn parse_duckduckgo_document(
    document: &Html,
) -> Result<Vec<SearchResult>, ProviderFailure> {
    if document
        .select(&selector(
            "form#challenge-form, form[action*='anomaly.js'], .anomaly-modal",
        ))
        .next()
        .is_some()
    {
        return Err(ProviderFailure::challenge(
            "DuckDuckGo returned a bot challenge; try --engine bing or --engine yahoo".into(),
        ));
    }
    let results = parse_duckduckgo_document_results(document);
    if results.is_empty() && document.select(&selector(".no-results")).next().is_none() {
        return Err(ProviderFailure::unrecognized(
            "DuckDuckGo returned an unrecognized search page; try --engine bing or --engine yahoo"
                .into(),
        ));
    }
    Ok(results)
}

#[cfg(test)]
pub(crate) fn parse_duckduckgo_results(html: &str) -> Vec<SearchResult> {
    parse_duckduckgo_document_results(&Html::parse_document(html))
}

fn parse_duckduckgo_document_results(document: &Html) -> Vec<SearchResult> {
    let item = selector("div.result.results_links.results_links_deep.web-result");
    let title = selector("h2.result__title a.result__a");
    let display = selector("a.result__url");
    let snippet = selector("a.result__snippet");
    document
        .select(&item)
        .filter_map(|entry| {
            let link = entry.select(&title).next()?;
            Some(SearchResult::parsed(
                element_text(link, ""),
                link.value().attr("href").unwrap_or_default().to_owned(),
                entry
                    .select(&display)
                    .next()
                    .map_or_else(String::new, |value| element_text(value, "")),
                entry
                    .select(&snippet)
                    .next()
                    .map_or_else(String::new, |value| element_text(value, "")),
            ))
        })
        .collect()
}
