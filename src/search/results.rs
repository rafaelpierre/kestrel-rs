//! Provider normalization, provenance and deterministic fair fusion.
use super::ProviderResponse;
use crate::{
    error::KestrelError,
    model::{Engine, SearchResult, SourceOccurrence},
};
use std::collections::HashMap;
use url::Url;
pub(crate) fn filter_response(query: &str, response: &mut ProviderResponse) {
    crate::telemetry::results("provider.raw_results", &response.results);
    response.raw_result_count = response.results.len();
    normalize_provider_results(&mut response.results, query);
}

pub(crate) fn normalize_provider_results(results: &mut Vec<SearchResult>, query: &str) {
    for (index, result) in results.iter_mut().enumerate() {
        result.engine_rank = Some(index + 1);
    }
    results.retain(|result| {
        let accepted = result_allowed(query, &result.url);
        if !accepted {
            crate::telemetry::results(
                "provider.rejected_invalid_url_or_query",
                std::slice::from_ref(result),
            );
        }
        accepted
    });
}

pub(crate) fn with_provenance(
    results: Vec<SearchResult>,
    engine: Engine,
    query: &str,
) -> Vec<SearchResult> {
    results
        .into_iter()
        .enumerate()
        .map(|(index, mut result)| {
            let rank = result.engine_rank.unwrap_or(index + 1);
            result.engine = Some(engine);
            result.query = Some(query.to_owned());
            result.engine_rank = Some(rank);
            result.sources = vec![SourceOccurrence {
                engine,
                query: query.to_owned(),
                rank,
            }];
            result
        })
        .collect()
}

pub(crate) fn merge_outcomes(
    outcomes: Vec<Result<Vec<SearchResult>, KestrelError>>,
) -> Result<Vec<SearchResult>, KestrelError> {
    let mut buckets = Vec::new();
    let mut failures = Vec::new();
    for outcome in outcomes {
        match outcome {
            Ok(results) => buckets.push(results),
            Err(error) => failures.push(error),
        }
    }
    if buckets.is_empty() && !failures.is_empty() {
        return Err(KestrelError::Search(format!(
            "Every search failed: {}",
            failures
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        )));
    }
    if !failures.is_empty() {
        crate::log_event!(
            "search_partial_failure",
            "mode" => "fanout",
            "failure_count" => failures.len(),
            "success_count" => buckets.len(),
        );
    }
    Ok(merge_round_robin(buckets))
}

pub(crate) fn merge_round_robin(buckets: Vec<Vec<SearchResult>>) -> Vec<SearchResult> {
    let mut iterators: Vec<_> = buckets.into_iter().map(Vec::into_iter).collect();
    let mut merged = Vec::new();
    let mut by_url = HashMap::new();
    loop {
        let mut advanced = false;
        for iterator in &mut iterators {
            let Some(item) = iterator.next() else {
                continue;
            };
            advanced = true;
            let key = result_key(&item);
            if let Some(existing_index) = by_url.get(&key).copied() {
                let existing: &mut SearchResult = &mut merged[existing_index];
                existing.sources.extend(item.sources);
            } else {
                by_url.insert(key, merged.len());
                merged.push(item);
            }
        }
        if !advanced {
            break;
        }
    }
    merged
}

pub(crate) fn result_key(result: &SearchResult) -> String {
    let canonical = canonical_url(&result.url);
    if canonical.is_empty() {
        format!("{}\0{}", result.title, result.snippet)
    } else {
        canonical
    }
}

pub(crate) fn canonical_url(value: &str) -> String {
    let Ok(mut url) = Url::parse(value) else {
        return String::new();
    };
    url.set_fragment(None);
    let retained: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| {
            let key = key.to_ascii_lowercase();
            !key.starts_with("utm_") && !matches!(key.as_str(), "fbclid" | "gclid" | "msclkid")
        })
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    url.set_query(None);
    if !retained.is_empty() {
        url.query_pairs_mut().extend_pairs(retained);
    }
    let trimmed_path = url.path().trim_end_matches('/').to_owned();
    url.set_path(if trimmed_path.is_empty() {
        "/"
    } else {
        &trimmed_path
    });
    url.to_string()
}

/// Split at whitespace outside quoted phrases, preserving all query characters.
fn query_tokens(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quoted = false;
    for c in query.chars() {
        if c == '"' {
            quoted = !quoted;
        }
        if c.is_whitespace() && !quoted {
            if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            }
        } else {
            token.push(c);
        }
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    tokens
}

/// Extract only unambiguous, standalone positive hostname restrictions.
/// Compound boolean expressions, quoted operators and path filters remain upstream.
pub(crate) fn site_domain(query: &str) -> Option<String> {
    let terms = query_tokens(query);
    if query.contains(['(', ')', '|'])
        || terms
            .iter()
            .any(|t| t.eq_ignore_ascii_case("OR") || t.eq_ignore_ascii_case("NOT"))
    {
        return None;
    }
    let sites: Vec<_> = terms
        .iter()
        .filter_map(|token| {
            token
                .get(..5)
                .filter(|prefix| prefix.eq_ignore_ascii_case("site:"))
                .map(|_| &token[5..])
        })
        .collect();
    if sites.len() != 1 {
        return None;
    }
    let domain = sites[0].trim_end_matches('.').to_ascii_lowercase();
    if !domain.contains('.')
        || !domain.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
    {
        return None;
    }
    Some(domain)
}

fn valid_result_url(value: &str) -> Option<Url> {
    let url = Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    Some(url)
}

pub(crate) fn result_allowed(query: &str, value: &str) -> bool {
    let Some(url) = valid_result_url(value) else {
        return false;
    };
    let host = url
        .host_str()
        .expect("validated hostname")
        .trim_end_matches('.');
    site_domain(query).is_none_or(|domain| host == domain || host.ends_with(&format!(".{domain}")))
}
