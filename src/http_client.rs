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
    #[cfg(test)]
    pub(crate) fn bing_experiment() -> Self {
        Self {
            browser: Impersonate::ChromeV146,
            os: ImpersonateOS::MacOS,
            language: "en-GB,en;q=0.9",
        }
    }

    pub fn random() -> Self {
        let browsers = [
            Impersonate::ChromeV146,
            Impersonate::ChromeV148,
            Impersonate::FirefoxV146,
        ];
        let systems = [
            ImpersonateOS::Windows,
            ImpersonateOS::MacOS,
            ImpersonateOS::Linux,
        ];
        let languages = ["en-US,en;q=0.9", "en-GB,en;q=0.9"];
        Self {
            browser: browsers[rand::random_range(0..browsers.len())],
            os: systems[rand::random_range(0..systems.len())],
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
        let mut agents = std::collections::HashSet::new();
        for browser in [
            Impersonate::ChromeV146,
            Impersonate::ChromeV148,
            Impersonate::FirefoxV146,
        ] {
            for os in [
                ImpersonateOS::Windows,
                ImpersonateOS::MacOS,
                ImpersonateOS::Linux,
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
        }
        assert_eq!(agents.len(), 9);
    }

    #[tokio::test]
    async fn retries_use_same_headers_and_request_headers_override_defaults() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::any};
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let profile = BrowserProfile::random();
        let expected = profile.headers();
        let client = standard_builder(profile, &crate::TransportOptions::default())
            .build()
            .unwrap();
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
        }
        for request in server.received_requests().await.unwrap() {
            assert_eq!(request.headers["user-agent"], expected["user-agent"]);
            assert_eq!(
                request.headers["accept-language"],
                expected["accept-language"]
            );
            assert_eq!(request.headers["accept"], "application/json");
        }
    }
    #[tokio::test]
    async fn http2_multiplexes_and_clones_reuse_the_connection() {
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
