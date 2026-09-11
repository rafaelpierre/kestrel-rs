//! Explicit HEAD warm-up using the pools retained by KestrelClient.
use std::time::{Duration, Instant};

use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};

use crate::{Engine, KestrelClient, KestrelError};

/// A non-success status can still warm a connection (for example, HEAD 405).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WarmupResult {
    pub origin: String,
    pub status: Option<u16>,
    pub http_version: Option<String>,
    pub elapsed_ms: u64,
    pub error: Option<String>,
}

impl KestrelClient {
    /// Warm up page-fetch origins with at most four concurrent HEAD requests.
    /// URLs are reduced to scheme/host/port: paths, credentials and queries are not sent.
    /// Redirects follow the normal client policy. This performs HTTP requests, not just connects.
    pub async fn warm_up_fetch(
        &self,
        urls: &[String],
        timeout: Duration,
    ) -> Result<Vec<WarmupResult>, KestrelError> {
        validate_timeout(timeout)?;
        // Validate the entire list before any network activity.
        let origins = origins(urls)?;
        Ok(stream::iter(origins.into_iter().map(|origin| async move {
            let started = Instant::now();
            let result = self
                .fetch
                .head(&origin)
                .timeout(timeout)
                .send()
                .await
                .map(|response| {
                    (
                        response.status().as_u16(),
                        format!("{:?}", response.version()),
                    )
                })
                .map_err(|error| error.to_string());
            outcome(origin, started, result)
        }))
        .buffered(4)
        .collect()
        .await)
    }

    /// Warm selected search-provider pools without submitting a search query.
    pub async fn warm_up_search(
        &self,
        engines: &[Engine],
        timeout: Duration,
    ) -> Result<Vec<WarmupResult>, KestrelError> {
        validate_timeout(timeout)?;
        let mut selected = Vec::new();
        for engine in engines {
            if !selected.contains(engine) {
                selected.push(*engine);
            }
        }
        Ok(stream::iter(selected.into_iter().map(|engine| async move {
            let origin = search_origin(engine);
            let started = Instant::now();
            let result = if engine == Engine::Yahoo {
                self.search
                    .yahoo
                    .as_ref()
                    .expect("KestrelClient builds Yahoo")
                    .head(origin)
                    .timeout(timeout)
                    .send()
                    .await
                    .map(|r| (r.status().as_u16(), format!("{:?}", r.version())))
                    .map_err(|e| e.to_string())
            } else {
                self.search
                    .standard
                    .head(origin)
                    .timeout(timeout)
                    .send()
                    .await
                    .map(|r| (r.status().as_u16(), format!("{:?}", r.version())))
                    .map_err(|e| e.to_string())
            };
            outcome(origin.to_owned(), started, result)
        }))
        .buffered(4)
        .collect()
        .await)
    }
}

fn search_origin(engine: Engine) -> &'static str {
    match engine {
        Engine::Duckduckgo => "https://html.duckduckgo.com/",
        Engine::Bing => "https://www.bing.com/",
        Engine::Yahoo => "https://search.yahoo.com/",
        Engine::Dogpile => "https://www.dogpile.com/",
        Engine::Ecosia => "https://www.ecosia.org/",
        Engine::Swisscows => "https://api.swisscows.com/",
        Engine::Yep => "https://api.yep.com/",
        Engine::Qwant => "https://api.qwant.com/",
        Engine::Mojeek => "https://www.mojeek.com/",
    }
}

fn validate_timeout(timeout: Duration) -> Result<(), KestrelError> {
    if timeout.is_zero() {
        return Err(KestrelError::InvalidRequest(
            "warm-up timeout must be positive".into(),
        ));
    }
    Ok(())
}

fn origins(urls: &[String]) -> Result<Vec<String>, KestrelError> {
    let mut origins = Vec::new();
    for raw in urls {
        let url = url::Url::parse(raw)
            .map_err(|_| KestrelError::InvalidRequest("invalid warm-up URL".into()))?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(KestrelError::InvalidRequest(
                "warm-up requires an HTTP(S) origin".into(),
            ));
        }
        let origin = format!("{}/", url.origin().ascii_serialization());
        if !origins.contains(&origin) {
            origins.push(origin);
        }
    }
    Ok(origins)
}

fn outcome(
    origin: String,
    started: Instant,
    result: Result<(u16, String), String>,
) -> WarmupResult {
    let (status, http_version, error) = match result {
        Ok((status, version)) => (Some(status), Some(version), None),
        Err(error) => (None, None, Some(error)),
    };
    WarmupResult {
        origin,
        status,
        http_version,
        error,
        elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    #[test]
    fn deduplicates_origins_and_rejects_non_http_targets() {
        assert_eq!(
            origins(&[
                "https://user:pass@example.com:443/a?q=secret".into(),
                "https://example.com/b".into(),
                "http://[::1]:8000/path".into(),
            ])
            .unwrap(),
            vec!["https://example.com/", "http://[::1]:8000/"]
        );
        assert!(origins(&["file:///tmp/page".into()]).is_err());
        assert!(origins(&["bad-url".into()]).is_err());
    }

    #[test]
    fn additional_provider_warmup_matches_actual_request_origin() {
        let client = reqwest::Client::new();
        for engine in [
            Engine::Dogpile,
            Engine::Ecosia,
            Engine::Swisscows,
            Engine::Yep,
            Engine::Qwant,
            Engine::Mojeek,
        ] {
            let request =
                crate::providers::request(&client, engine, "test", "", crate::TimeFilter::Any)
                    .unwrap()
                    .build()
                    .unwrap();
            assert_eq!(
                search_origin(engine),
                format!("{}/", request.url().origin().ascii_serialization())
            );
        }
    }

    #[tokio::test]
    async fn warmup_records_http1_head_rejections_and_timeouts() {
        let server = MockServer::start().await;
        Mock::given(method("HEAD"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(405))
            .expect(1)
            .mount(&server)
            .await;
        let client = KestrelClient::new().unwrap();
        let results = client
            .warm_up_fetch(&[server.uri()], Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(results[0].status, Some(405));
        assert_eq!(results[0].http_version.as_deref(), Some("HTTP/1.1"));
        assert!(results[0].error.is_none());
        server.verify().await;
        server.reset().await;
        Mock::given(method("HEAD"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(1)))
            .mount(&server)
            .await;
        let results = client
            .warm_up_fetch(&[server.uri()], Duration::from_millis(20))
            .await
            .unwrap();
        assert!(results[0].error.is_some());
        assert_eq!(results[0].status, None);
    }

    #[tokio::test]
    async fn invalid_warmup_never_sends_partial_requests() {
        let server = MockServer::start().await;
        let client = KestrelClient::new().unwrap();
        assert!(
            client
                .warm_up_fetch(&[server.uri(), "bad-url".into()], Duration::from_secs(1))
                .await
                .is_err()
        );
        assert!(
            client
                .warm_up_fetch(&[server.uri()], Duration::ZERO)
                .await
                .is_err()
        );
        assert!(server.received_requests().await.unwrap().is_empty());
        assert!(
            client
                .warm_up_search(&[], Duration::from_secs(1))
                .await
                .unwrap()
                .is_empty()
        );
    }
}
