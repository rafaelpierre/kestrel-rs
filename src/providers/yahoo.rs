//! Yahoo request and HTML adapter.
use super::{
    html::{element_text, selector},
    response::extract_completed,
};
use crate::search::ProviderFailure;
use crate::{
    model::{SearchResult, TimeFilter},
    search::{
        ProviderResponse,
        transport::{SEARCH_TIMEOUT, request_yahoo_with_retries},
    },
};
use scraper::{ElementRef, Html, Selector};
use url::Url;
pub(crate) fn yahoo_request(
    client: &primp::Client,
    query: &str,
    region: &str,
    time_filter: TimeFilter,
) -> primp::RequestBuilder {
    let mut params = vec![("p", query), ("ei", "UTF-8")];
    if !region.is_empty() {
        params.push(("vl", region));
    }
    if time_filter != TimeFilter::Any {
        params.push(("btf", time_filter.as_str()));
    }
    client
        .get("https://search.yahoo.com/search")
        .query(&params)
        .timeout(SEARCH_TIMEOUT)
}

pub(crate) async fn search_yahoo(
    query: &str,
    region: &str,
    time_filter: TimeFilter,
    client: &primp::Client,
) -> Result<ProviderResponse, ProviderFailure> {
    let (results, retries) = request_yahoo_with_retries(query, extract_completed, || {
        yahoo_request(client, query, region, time_filter)
    })
    .await?;
    Ok(ProviderResponse {
        results: results?,
        retries,
        raw_result_count: 0,
    })
}

#[cfg(test)]
pub(crate) fn parse_yahoo_results(html: &str) -> Vec<SearchResult> {
    parse_yahoo_document(&Html::parse_document(html))
}

pub(crate) fn parse_yahoo_document(document: &Html) -> Vec<SearchResult> {
    let primary = selector("div.dd.algo");
    let fallback = selector("div.compTitle");
    let entries: Vec<_> = document.select(&primary).collect();
    let entries = if entries.is_empty() {
        document.select(&fallback).collect()
    } else {
        entries
    };
    let title_link = selector(".compTitle > a, h3 a");
    let title_selector = selector("h3");
    let snippet = selector(".compText p, .compText");
    entries
        .into_iter()
        .filter_map(|entry| {
            let is_title_only = entry.value().classes().any(|class| class == "compTitle");
            let container = if is_title_only {
                yahoo_result_container(entry, &snippet)
            } else {
                entry
            };
            let link = entry
                .select(&title_link)
                .next()
                .or_else(|| container.select(&title_link).next())?;
            let raw_url = link.value().attr("href").unwrap_or_default();
            let url = decode_yahoo_url(raw_url);
            let title = link
                .value()
                .attr("aria-label")
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    link.select(&title_selector)
                        .next()
                        .or_else(|| entry.select(&title_selector).next())
                        .or_else(|| container.select(&title_selector).next())
                        .map_or_else(|| element_text(link, " "), |value| element_text(value, " "))
                });
            let display_url = Url::parse(&url)
                .ok()
                .and_then(|parsed| parsed.host_str().map(str::to_owned))
                .unwrap_or_default();
            Some(SearchResult::parsed(
                title,
                url,
                display_url,
                container
                    .select(&snippet)
                    .next()
                    .map_or_else(String::new, |value| element_text(value, " ")),
            ))
        })
        .collect()
}

fn yahoo_result_container<'a>(entry: ElementRef<'a>, snippet: &Selector) -> ElementRef<'a> {
    let mut container = entry;
    for _ in 0..5 {
        let Some(parent) = container.parent().and_then(ElementRef::wrap) else {
            break;
        };
        container = parent;
        if container.select(snippet).next().is_some() {
            break;
        }
    }
    container
}

fn decode_yahoo_url(value: &str) -> String {
    let Some(encoded) = value
        .split("/RU=")
        .nth(1)
        .and_then(|tail| tail.split("/RK=").next())
    else {
        return value.to_owned();
    };
    url::form_urlencoded::parse(encoded.as_bytes())
        .next()
        .map(|(decoded, _)| decoded.into_owned())
        .unwrap_or_else(|| percent_decode(encoded))
}

fn percent_decode(value: &str) -> String {
    let with_prefix = format!("x={value}");
    url::form_urlencoded::parse(with_prefix.as_bytes())
        .next()
        .map(|(_, value)| value.into_owned())
        .unwrap_or_else(|| value.to_owned())
}
