//! Random browser headers and consistent connection-pool policy.

#[cfg(test)]
use std::time::Duration;

use primp::{Impersonate, ImpersonateOS};
use reqwest::header::{HeaderMap, HeaderValue};

/// Choose coherent browser/OS profiles rather than randomizing individual fields.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BrowserProfile {
    pub browser: Impersonate,
    pub os: ImpersonateOS,
    language: &'static str,
}

impl BrowserProfile {
    /// Stable profile used for Bing's browser-impersonated transport.
    pub(crate) fn bing() -> Self {
        Self {
            browser: Impersonate::ChromeV146,
            os: ImpersonateOS::MacOS,
            language: "en-GB,en;q=0.9",
        }
    }

    pub fn random() -> Self {
        let profiles = [
            (Impersonate::ChromeV146, ImpersonateOS::MacOS),
            (Impersonate::FirefoxV146, ImpersonateOS::Windows),
        ];
        let (browser, os) = profiles[rand::random_range(0..profiles.len())];
        let languages = ["en-US,en;q=0.9", "en-GB,en;q=0.9"];
        Self {
            browser,
            os,
            language: languages[rand::random_range(0..languages.len())],
        }
    }

    pub fn headers(self) -> HeaderMap {
        let mut headers = primp::imp::get_browser_settings(self.browser, Some(self.os)).headers;
        headers.insert("accept-language", HeaderValue::from_static(self.language));
        // Keep-alive is configured at the transport layer. Connection is illegal in H2.
        headers.remove("connection");
        headers.remove("keep-alive");
        headers
    }
}

/// Retain generated defaults with the pool; request overrides still take precedence.
#[derive(Clone, Debug)]
pub(crate) struct Client {
    inner: reqwest::Client,
    headers: HeaderMap,
}
impl Client {
    pub(crate) fn new(
        profile: BrowserProfile,
        transport: &crate::TransportOptions,
        timeout: Option<std::time::Duration>,
    ) -> Result<Self, reqwest::Error> {
        let mut builder = standard_builder(profile, transport);
        if let Some(timeout) = timeout {
            builder = builder.timeout(timeout);
        }
        Ok(Self {
            inner: builder.build()?,
            headers: profile.headers(),
        })
    }
    pub(crate) fn request_headers(&self, overrides: &HeaderMap) -> HeaderMap {
        let mut headers = self.headers.clone();
        headers.extend(overrides.clone());
        headers
    }
}
impl std::ops::Deref for Client {
    type Target = reqwest::Client;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
#[cfg(test)]
impl From<reqwest::Client> for Client {
    fn from(inner: reqwest::Client) -> Self {
        Self {
            inner,
            headers: HeaderMap::new(),
        }
    }
}

pub(crate) fn standard_builder(
    profile: BrowserProfile,
    transport: &crate::TransportOptions,
) -> reqwest::ClientBuilder {
    transport
        .standard_builder()
        .default_headers(profile.headers())
}

pub(crate) fn impersonated_builder(
    profile: BrowserProfile,
    transport: &crate::TransportOptions,
) -> primp::ClientBuilder {
    transport.impersonated_builder(profile)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_vary_and_keep_browser_headers_consistent() {
        let _telemetry = crate::telemetry::test_export_guard();
        let mut agents = std::collections::HashSet::new();
        for (browser, os) in [
            (Impersonate::ChromeV146, ImpersonateOS::MacOS),
            (Impersonate::FirefoxV146, ImpersonateOS::Windows),
        ] {
            let profile = BrowserProfile {
                browser,
                os,
                language: "en-GB,en;q=0.9",
            };
            let headers = profile.headers();
            agents.insert(headers["user-agent"].to_str().unwrap().to_owned());
            assert_eq!(headers["accept-language"], "en-GB,en;q=0.9");
            assert!(!headers.contains_key("connection"));
            assert!(!headers.contains_key("keep-alive"));
            if matches!(browser, Impersonate::FirefoxV146) {
                assert!(!headers.contains_key("sec-ch-ua"));
                assert!(
                    headers["user-agent"]
                        .to_str()
                        .unwrap()
                        .contains("Firefox/146")
                );
            } else {
                assert!(headers.contains_key("sec-ch-ua"));
            }
        }
        assert_eq!(agents.len(), 2);
        for _ in 0..256 {
            let profile = BrowserProfile::random();
            assert!(matches!(
                (profile.browser, profile.os),
                (Impersonate::ChromeV146, ImpersonateOS::MacOS)
                    | (Impersonate::FirefoxV146, ImpersonateOS::Windows)
            ));
        }
    }

    #[test]
    fn bing_profile_is_fixed_to_the_validated_chrome_macos_pair() {
        let _telemetry = crate::telemetry::test_export_guard();
        let profile = BrowserProfile::bing();
        assert!(matches!(
            (profile.browser, profile.os),
            (Impersonate::ChromeV146, ImpersonateOS::MacOS)
        ));
        assert_eq!(profile.headers()["accept-language"], "en-GB,en;q=0.9");
    }

    #[test]
    fn retained_defaults_respect_provider_overrides_and_clone() {
        let _telemetry = crate::telemetry::test_export_guard();
        let client = Client::new(
            BrowserProfile::bing(),
            &crate::TransportOptions::default(),
            None,
        )
        .unwrap();
        let request = client
            .get("http://localhost/")
            .header("accept", "application/json")
            .build()
            .unwrap();
        let headers = client.clone().request_headers(request.headers());
        assert_eq!(headers["accept"], "application/json");
        assert_eq!(headers.get_all("accept").iter().count(), 1);
        assert_eq!(
            headers["user-agent"],
            BrowserProfile::bing().headers()["user-agent"]
        );
    }

    #[tokio::test]
    async fn retries_use_same_headers_and_request_headers_override_defaults() {
        let _telemetry = crate::telemetry::test_export_guard();
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::any};
        for (browser, os) in [
            (Impersonate::ChromeV146, ImpersonateOS::MacOS),
            (Impersonate::FirefoxV146, ImpersonateOS::Windows),
        ] {
            let server = MockServer::start().await;
            Mock::given(any())
                .respond_with(ResponseTemplate::new(200))
                .mount(&server)
                .await;
            let profile = BrowserProfile {
                browser,
                os,
                language: "en-GB,en;q=0.9",
            };
            let expected = profile.headers();
            let client = Client::new(profile, &crate::TransportOptions::default(), None).unwrap();
            let mut yahoo = impersonated_builder(profile, &crate::TransportOptions::default())
                .build()
                .unwrap();
            *yahoo.headers_mut() = expected.clone();
            for _ in 0..2 {
                client
                    .get(server.uri())
                    .header("accept", "application/json")
                    .send()
                    .await
                    .unwrap()
                    .bytes()
                    .await
                    .unwrap();
                yahoo
                    .get(server.uri())
                    .header("accept", "application/json")
                    .send()
                    .await
                    .unwrap()
                    .bytes()
                    .await
                    .unwrap();
            }
            let requests = server.received_requests().await.unwrap();
            assert_eq!(requests.len(), 4);
            for request in requests {
                assert_eq!(request.headers["user-agent"], expected["user-agent"]);
                assert_eq!(
                    request.headers["accept-language"],
                    expected["accept-language"]
                );
                assert_eq!(request.headers["accept"], "application/json");
                assert_eq!(request.headers.get_all("accept").iter().count(), 1);
                assert_eq!(
                    request.headers.get("sec-ch-ua-platform"),
                    expected.get("sec-ch-ua-platform")
                );
            }
        }
    }

    #[tokio::test]
    async fn http2_multiplexes_and_clones_reuse_the_connection() {
        let _telemetry = crate::telemetry::test_export_guard();
        use http_body_util::Full;
        use hyper::{body::Bytes, server::conn::http2, service::service_fn};
        use hyper_util::rt::{TokioExecutor, TokioIo};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let connections = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&connections);
        let server = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                count.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let service = service_fn(
                        |request: hyper::Request<hyper::body::Incoming>| async move {
                            assert_eq!(request.version(), hyper::Version::HTTP_2);
                            Ok::<_, std::convert::Infallible>(hyper::Response::new(Full::new(
                                Bytes::from_static(b"ok"),
                            )))
                        },
                    );
                    let _ = http2::Builder::new(TokioExecutor::new())
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        // Cleartext prior knowledge is test-only; production uses ALPN with H1 fallback.
        let client = standard_builder(
            BrowserProfile::random(),
            &crate::TransportOptions::default(),
        )
        .no_proxy()
        .http2_prior_knowledge()
        .build()
        .unwrap();
        let url = format!("http://{address}/");
        let first = client.get(&url).send().await.unwrap();
        assert_eq!(first.version(), reqwest::Version::HTTP_2);
        assert_eq!(first.text().await.unwrap(), "ok");
        let clone = client.clone();
        let (a, b) = tokio::join!(client.get(&url).send(), clone.get(&url).send());
        for response in [a.unwrap(), b.unwrap()] {
            assert_eq!(response.version(), reqwest::Version::HTTP_2);
            assert_eq!(response.text().await.unwrap(), "ok");
        }
        assert_eq!(connections.load(Ordering::SeqCst), 1);
        let yahoo = impersonated_builder(
            BrowserProfile::random(),
            &crate::TransportOptions::default(),
        )
        .no_proxy()
        .http2_prior_knowledge()
        .build()
        .unwrap();
        for client in [yahoo.clone(), yahoo.clone()] {
            let response = client.get(&url).send().await.unwrap();
            assert_eq!(response.version(), primp::Version::HTTP_2);
            assert_eq!(response.text().await.unwrap(), "ok");
        }
        assert_eq!(connections.load(Ordering::SeqCst), 2);
        server.abort();
    }

    #[tokio::test]
    async fn http1_fallback_keeps_the_socket_alive() {
        let _telemetry = crate::telemetry::test_export_guard();
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            let mut sockets = 0;
            let mut served = 0;
            while served < 2 && std::time::Instant::now() < deadline {
                let Ok((mut socket, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                };
                sockets += 1;
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                while served < 2 {
                    let mut request = Vec::new();
                    let mut byte = [0];
                    while !request.ends_with(b"\r\n\r\n") {
                        if socket.read(&mut byte).unwrap_or(0) == 0 {
                            break;
                        }
                        request.push(byte[0]);
                    }
                    if !request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                    socket
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                        .unwrap();
                    served += 1;
                }
            }
            (sockets, served)
        });
        let client = standard_builder(
            BrowserProfile::random(),
            &crate::TransportOptions::default(),
        )
        .no_proxy()
        .build()
        .unwrap();
        for _ in 0..2 {
            let response = client
                .get(format!("http://{address}/"))
                .send()
                .await
                .unwrap();
            assert_eq!(response.version(), reqwest::Version::HTTP_11);
            assert_eq!(response.text().await.unwrap(), "ok");
        }
        assert_eq!(server.join().unwrap(), (1, 2));
    }
}
