use super::*;
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{any, method, query_param},
};

fn row(ip: &str) -> serde_json::Value {
    json!({"ip": ip, "port": 8080, "protocols": ["http", "https"], "status": "alive",
        "country_code": "GB", "latency_ms": 42, "uptime_pct": 99, "asn": null, "unknown": {"value": true}})
}
fn adapter(server: &MockServer, options: DiscoveryOptions) -> HProxyDiscovery {
    HProxyDiscovery::with_builder(
        Url::parse(&server.uri()).unwrap(),
        reqwest::Client::builder().no_proxy(),
        options,
    )
    .unwrap()
}

#[tokio::test]
async fn one_json_get_coalesces_clones_and_validates_rows() {
    let server = MockServer::start().await;
    let mut socks = row("192.0.2.2");
    socks["protocols"] = json!(["socks4", "socks5"]);
    let mut invalid = row("192.0.2.3");
    invalid["port"] = json!(0);
    let mut recent = row("192.0.2.4");
    recent["status"] = json!("recently_alive");
    let mut nullable = row("2001:db8::1");
    nullable["country_code"] = json!(null);
    nullable["latency_ms"] = json!(null);
    nullable["uptime_pct"] = json!(null);
    Mock::given(method("GET")).and(query_param("format", "json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([row("192.0.2.1"), row("192.0.2.1"), socks, invalid, recent, nullable, null, {"ip": "bad"}]))).expect(1).mount(&server).await;
    let discovery = adapter(&server, DiscoveryOptions::default());
    let clone = discovery.clone();
    assert!(server.received_requests().await.unwrap().is_empty());
    let (a, b) = tokio::join!(discovery.discover(), clone.discover());
    let a = a.unwrap();
    assert!(Arc::ptr_eq(&a, &b.unwrap()));
    assert!(Arc::ptr_eq(&a, &discovery.discover().await.unwrap()));
    assert_eq!(a.endpoints.len(), 2);
    assert_eq!(a.duplicate_rows, 1);
    assert_eq!(a.malformed_rows, 3);
    assert_eq!(a.unsupported_rows, 1);
    assert_eq!(a.filtered_rows, 1);
    assert_eq!((a.pages, a.attempts), (1, 1));
    assert_eq!(a.endpoints[1].proxy_url(), "http://[2001:db8::1]:8080");
    let requests = server.received_requests().await.unwrap();
    let request = &requests[0];
    assert_eq!(request.headers["accept"], "application/json");
    assert!(!request.headers.contains_key("authorization"));
    for name in ["limit", "offset", "recent", "key", "api_key"] {
        assert!(!request.url.query_pairs().any(|(k, _)| k == name));
    }
}

#[tokio::test]
async fn filters_remote_and_local_with_missing_enrichment() {
    let server = MockServer::start().await;
    let mut wrong_country = row("192.0.2.2");
    wrong_country["country_code"] = json!("US");
    let mut slow = row("192.0.2.3");
    slow["latency_ms"] = json!(200);
    let mut unreliable = row("192.0.2.4");
    unreliable["uptime_pct"] = json!(40);
    let mut missing = row("192.0.2.5");
    missing["uptime_pct"] = json!(null);
    let mut recent = row("192.0.2.6");
    recent["status"] = json!("recently_alive");
    let mut http = row("192.0.2.7");
    http["protocols"] = json!(["http"]);
    Mock::given(query_param("country", "GB"))
        .and(query_param("protocol", "https"))
        .and(query_param("min_uptime_pct", "90"))
        .and(query_param("max_latency_ms", "100"))
        .and(query_param("recent", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            row("192.0.2.1"),
            wrong_country,
            slow,
            unreliable,
            missing,
            recent,
            http
        ])))
        .expect(1)
        .mount(&server)
        .await;
    let result = adapter(
        &server,
        DiscoveryOptions {
            country: Some("gb".into()),
            protocols: vec![Protocol::Https],
            min_uptime_pct: Some(90.0),
            max_latency_ms: Some(100),
            include_recent: true,
            ..Default::default()
        },
    )
    .discover()
    .await
    .unwrap();
    assert_eq!(result.endpoints.len(), 2);
    assert_eq!(result.filtered_rows, 5);
}

#[tokio::test]
async fn errors_are_typed_and_shared_without_new_requests() {
    for (body, expected) in [
        ("null", DiscoveryError::InvalidJson),
        ("{", DiscoveryError::InvalidJson),
        ("[]", DiscoveryError::Empty),
        ("[null]", DiscoveryError::Empty),
        (
            r#"[{"ip":"192.0.2.1","port":80,"protocols":["socks5"],"status":"alive"}]"#,
            DiscoveryError::Empty,
        ),
    ] {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
            .expect(1)
            .mount(&server)
            .await;
        let discovery = adapter(&server, DiscoveryOptions::default());
        assert_eq!(discovery.discover().await.unwrap_err(), expected);
        assert_eq!(discovery.clone().discover().await.unwrap_err(), expected);
    }
}

#[tokio::test]
async fn body_entry_and_pagination_limits_never_return_partial_success() {
    for (options, response, error) in [
        (
            DiscoveryOptions {
                max_response_bytes: 2,
                ..Default::default()
            },
            ResponseTemplate::new(200).set_body_json(json!([row("192.0.2.1")])),
            DiscoveryError::ResponseTooLarge,
        ),
        (
            DiscoveryOptions {
                max_entries: 1,
                ..Default::default()
            },
            ResponseTemplate::new(200).set_body_json(json!([row("192.0.2.1"), row("192.0.2.2")])),
            DiscoveryError::TooManyEntries,
        ),
        (
            DiscoveryOptions::default(),
            ResponseTemplate::new(200)
                .insert_header("x-total-available", "2")
                .set_body_json(json!([row("192.0.2.1")])),
            DiscoveryError::IncompleteList,
        ),
        (
            DiscoveryOptions {
                page_size: Some(1),
                ..Default::default()
            },
            ResponseTemplate::new(200)
                .insert_header("x-total-count", "1")
                .insert_header("x-total-available", "2")
                .set_body_json(json!([row("192.0.2.1")])),
            DiscoveryError::IncompleteList,
        ),
        (
            DiscoveryOptions {
                page_size: Some(1),
                ..Default::default()
            },
            ResponseTemplate::new(200).set_body_json(json!([row("192.0.2.1")])),
            DiscoveryError::IncompleteList,
        ),
        (
            DiscoveryOptions::default(),
            ResponseTemplate::new(200)
                .insert_header("x-total-count", "2")
                .set_body_json(json!([row("192.0.2.1")])),
            DiscoveryError::IncompleteList,
        ),
        (
            DiscoveryOptions::default(),
            ResponseTemplate::new(206).set_body_json(json!([row("192.0.2.1")])),
            DiscoveryError::Http(206),
        ),
    ] {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(response)
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            adapter(&server, options).discover().await.unwrap_err(),
            error
        );
    }
}

#[tokio::test]
async fn explicit_pages_use_counts_and_aggregate_budgets() {
    for cap in [10_000, 300] {
        let server = MockServer::start().await;
        for offset in 0..2 {
            Mock::given(query_param("limit", "1"))
                .and(query_param("offset", offset.to_string()))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("x-total-count", "1")
                        .insert_header("x-total-available", "2")
                        .set_body_json(json!([row(&format!("192.0.2.{}", offset + 1))])),
                )
                .expect(1)
                .mount(&server)
                .await;
        }
        let result = adapter(
            &server,
            DiscoveryOptions {
                page_size: Some(1),
                max_pages: 2,
                max_response_bytes: cap,
                ..Default::default()
            },
        )
        .discover()
        .await;
        if cap == 300 {
            assert_eq!(result.unwrap_err(), DiscoveryError::ResponseTooLarge);
        } else {
            let report = result.unwrap();
            assert_eq!(report.endpoints.len(), 2);
            assert_eq!(report.attempts, 2);
        }
    }
}

#[tokio::test]
async fn redirects_cannot_bypass_attempt_accounting() {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(302).insert_header("location", server.uri()))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        adapter(&server, DiscoveryOptions::default())
            .discover()
            .await
            .unwrap_err(),
        DiscoveryError::Http(302)
    );
}

#[tokio::test]
async fn retry_after_and_server_errors_obey_attempts() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    for status in [429, 503] {
        let server = MockServer::start().await;
        let seen = Arc::new(AtomicUsize::new(0));
        let counter = seen.clone();
        Mock::given(any())
            .respond_with(move |_: &wiremock::Request| {
                if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                    ResponseTemplate::new(status).insert_header("retry-after", "1")
                } else {
                    ResponseTemplate::new(200).set_body_json(json!([row("192.0.2.1")]))
                }
            })
            .expect(2)
            .mount(&server)
            .await;
        let discovery = adapter(&server, DiscoveryOptions::default());
        let started = Instant::now();
        let report = discovery.discover().await.unwrap();
        assert_eq!(report.attempts, 2);
        assert!(started.elapsed() >= Duration::from_secs(1));
    }
}

#[tokio::test]
async fn deadline_and_excessive_retry_after_are_bounded() {
    for (response, expected) in [
        (
            ResponseTemplate::new(429).insert_header("retry-after", "120"),
            DiscoveryError::RetryAfter,
        ),
        (
            ResponseTemplate::new(429).insert_header("retry-after", "garbage"),
            DiscoveryError::RetryAfter,
        ),
        (
            ResponseTemplate::new(200).set_delay(Duration::from_secs(2)),
            DiscoveryError::Deadline,
        ),
        (ResponseTemplate::new(503), DiscoveryError::Http(503)),
    ] {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(response)
            .expect(1)
            .mount(&server)
            .await;
        let options = DiscoveryOptions {
            deadline: Duration::from_millis(100),
            max_attempts: 1,
            ..Default::default()
        };
        assert_eq!(
            adapter(&server, options).discover().await.unwrap_err(),
            expected
        );
    }
}

#[tokio::test(start_paused = true)]
async fn shared_throttle_and_cancelled_initialization_keep_original_deadline() {
    let server = MockServer::start().await;
    let discovery = adapter(
        &server,
        DiscoveryOptions {
            deadline: Duration::from_secs(3),
            ..Default::default()
        },
    );
    {
        let mut state = discovery.inner.state.lock().await;
        state.end = Some(Instant::now() + Duration::from_secs(3));
        state.next_attempt = Instant::now() + Duration::from_secs(5);
        state.attempts = 1;
    }
    assert_eq!(
        discovery.discover().await.unwrap_err(),
        DiscoveryError::Deadline
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    assert_eq!(discovery.inner.state.lock().await.attempts, 1);
}

#[test]
fn retry_after_clock_and_invalid_options() {
    let now = chrono::DateTime::parse_from_rfc2822("Mon, 14 Sep 2026 10:00:00 GMT")
        .unwrap()
        .with_timezone(&chrono::Utc);
    assert_eq!(
        retry_after("Mon, 14 Sep 2026 10:00:10 GMT", now).unwrap(),
        Duration::from_secs(10)
    );
    assert_eq!(
        retry_after("Mon, 14 Sep 2026 09:59:00 GMT", now).unwrap(),
        Duration::ZERO
    );
    assert_eq!(
        retry_after("18446744073709551616", now),
        Err(DiscoveryError::RetryAfter)
    );
    for protocol in ["socks4", "socks5", "ftp", ""] {
        assert_eq!(
            protocol.parse::<Protocol>(),
            Err(DiscoveryError::UnsupportedProtocol)
        );
    }
    for options in [
        DiscoveryOptions {
            min_uptime_pct: Some(f64::NAN),
            ..Default::default()
        },
        DiscoveryOptions {
            deadline: Duration::MAX,
            ..Default::default()
        },
        DiscoveryOptions {
            max_attempts: 0,
            ..Default::default()
        },
        DiscoveryOptions {
            max_entries: 0,
            ..Default::default()
        },
        DiscoveryOptions {
            page_size: Some(10001),
            ..Default::default()
        },
        DiscoveryOptions {
            min_attempt_interval: Duration::from_millis(499),
            ..Default::default()
        },
        DiscoveryOptions {
            country: Some("USA".into()),
            ..Default::default()
        },
    ] {
        assert_eq!(options.validate(), Err(DiscoveryError::InvalidOptions));
    }
}

#[tokio::test]
async fn both_transports_forward_http_and_tunnel_verified_tls() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_rustls::{TlsAcceptor, rustls};
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["origin.invalid".into()]).unwrap();
    let der = cert.der().clone();
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![der.clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into(),
        )
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut lines = Vec::new();
        for _ in 0..4 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(socket.read_u8().await.unwrap());
            }
            let text = String::from_utf8(request).unwrap();
            lines.push(text.lines().next().unwrap().to_owned());
            if text.starts_with("CONNECT ") {
                socket
                    .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                    .await
                    .unwrap();
                let mut tls = acceptor.accept(socket).await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(tls.read_u8().await.unwrap());
                }
                assert!(request.starts_with(b"GET /test HTTP/1.1\r\n"));
                tls.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                )
                .await
                .unwrap();
                tls.shutdown().await.unwrap();
            } else {
                socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .await
                    .unwrap();
            }
        }
        lines
    });
    let endpoint = ProxyEndpoint {
        address,
        protocols: vec![Protocol::Http, Protocol::Https],
        country_code: None,
        latency_ms: None,
        uptime_pct: None,
    };
    let standard = crate::TransportOptions::default()
        .standard_builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(endpoint.proxy_url()).unwrap())
        .tls_certs_merge([reqwest::Certificate::from_der(&der).unwrap()])
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let impersonated = crate::TransportOptions::default()
        .impersonated_builder(crate::http_client::BrowserProfile::random())
        .no_proxy()
        .proxy(primp::Proxy::all(endpoint.proxy_url()).unwrap())
        .add_root_certificate(primp::Certificate::from_der(&der).unwrap())
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    for url in ["http://origin.invalid/test", "https://origin.invalid/test"] {
        assert_eq!(
            standard
                .get(url)
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "ok"
        );
        assert_eq!(
            impersonated
                .get(url)
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "ok"
        );
    }
    let lines = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.as_str() == "GET http://origin.invalid/test HTTP/1.1")
            .count(),
        2
    );
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.as_str() == "CONNECT origin.invalid:443 HTTP/1.1")
            .count(),
        2
    );
}

#[tokio::test]
async fn injected_bootstrap_proxy_is_used() {
    let server = MockServer::start().await;
    Mock::given(query_param("format", "json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([row("192.0.2.1")])))
        .expect(1)
        .mount(&server)
        .await;
    let discovery = HProxyDiscovery::with_builder(
        Url::parse("http://discovery.invalid/api/proxy-list").unwrap(),
        reqwest::Client::builder()
            .no_proxy()
            .proxy(reqwest::Proxy::all(server.uri()).unwrap()),
        DiscoveryOptions::default(),
    )
    .unwrap();
    assert_eq!(discovery.discover().await.unwrap().endpoints.len(), 1);
}

#[tokio::test]
async fn chunked_body_limit_and_exact_limit() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for extra in [false, true] {
        let body = serde_json::to_vec(&json!([row("192.0.2.1")])).unwrap();
        let limit = body.len();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(socket.read_u8().await.unwrap());
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                .await
                .unwrap();
            socket
                .write_all(format!("{:x}\r\n", body.len()).as_bytes())
                .await
                .unwrap();
            socket.write_all(&body).await.unwrap();
            socket
                .write_all(if extra {
                    b"\r\n1\r\n \r\n0\r\n\r\n"
                } else {
                    b"\r\n0\r\n\r\n"
                })
                .await
                .unwrap();
        });
        let discovery = HProxyDiscovery::with_builder(
            Url::parse(&format!("http://{address}/")).unwrap(),
            reqwest::Client::builder().no_proxy(),
            DiscoveryOptions {
                max_response_bytes: limit,
                ..Default::default()
            },
        )
        .unwrap();
        let result = discovery.discover().await;
        if extra {
            assert_eq!(result.unwrap_err(), DiscoveryError::ResponseTooLarge);
        } else {
            assert_eq!(result.unwrap().decoded_bytes, limit);
        }
        server.await.unwrap();
    }
}

#[test]
fn deduplication_precedes_retained_limit_and_unions_capabilities() {
    let mut http = row("192.0.2.1");
    http["protocols"] = json!(["http"]);
    let mut https = http.clone();
    https["protocols"] = json!(["https", "socks5"]);
    let bytes = serde_json::to_vec(&json!([http, https])).unwrap();
    let result = parse_page(
        &bytes,
        &DiscoveryOptions {
            max_entries: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(result.duplicate_rows, 1);
    assert_eq!(
        result.endpoints[0].protocols,
        vec![Protocol::Http, Protocol::Https]
    );
}

#[tokio::test(start_paused = true)]
async fn controlled_clock_throttles_attempts_and_preserves_retry_after() {
    let start = Instant::now();
    let options = DiscoveryOptions {
        max_attempts: 121,
        deadline: Duration::from_secs(90),
        ..Default::default()
    };
    let end = start + options.deadline;
    let mut state = State {
        end: Some(end),
        next_attempt: start,
        attempts: 0,
    };
    for _ in 0..121 {
        state.reserve_attempt(&options, end).await.unwrap();
    }
    assert_eq!(Instant::now() - start, Duration::from_secs(60));
    assert_eq!(
        state.reserve_attempt(&options, end).await,
        Err(DiscoveryError::Attempts)
    );
    let mut state = State {
        end: Some(end),
        next_attempt: Instant::now() + Duration::from_secs(5),
        attempts: 0,
    };
    let before = Instant::now();
    state.reserve_attempt(&options, end).await.unwrap();
    assert_eq!(Instant::now() - before, Duration::from_secs(5));
}

#[tokio::test(start_paused = true)]
async fn cancellation_during_throttle_does_not_reset_deadline_or_attempts() {
    let discovery = HProxyDiscovery::with_builder(
        Url::parse("http://127.0.0.1:9/").unwrap(),
        reqwest::Client::builder().no_proxy(),
        DiscoveryOptions::default(),
    )
    .unwrap();
    let start = Instant::now();
    {
        let mut state = discovery.inner.state.lock().await;
        state.end = Some(start + Duration::from_secs(10));
        state.next_attempt = start + Duration::from_secs(5);
        state.attempts = 2;
    }
    let mut lookup = Box::pin(discovery.discover());
    assert!(futures_util::poll!(&mut lookup).is_pending());
    drop(lookup);
    let state = discovery.inner.state.lock().await;
    assert_eq!(state.attempts, 2);
    assert_eq!(state.next_attempt, start + Duration::from_secs(5));
    drop(state);
    tokio::time::advance(Duration::from_secs(11)).await;
    assert_eq!(
        discovery.clone().discover().await.unwrap_err(),
        DiscoveryError::Deadline
    );
}

#[test]
fn country_alias_and_invalid_numeric_enrichment() {
    let mut exported_country = row("192.0.2.1");
    exported_country
        .as_object_mut()
        .unwrap()
        .remove("country_code");
    exported_country["country"] = json!("gb");
    let mut bad_uptime = row("192.0.2.2");
    bad_uptime["uptime_pct"] = json!(101);
    let mut bad_latency = row("192.0.2.3");
    bad_latency["latency_ms"] = json!(-1);
    let parsed = parse_page(
        &serde_json::to_vec(&json!([exported_country, bad_uptime, bad_latency])).unwrap(),
        &DiscoveryOptions {
            country: Some("GB".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(parsed.endpoints.len(), 1);
    assert_eq!(parsed.endpoints[0].country_code.as_deref(), Some("GB"));
    assert_eq!(parsed.malformed_rows, 2);
}

#[tokio::test]
async fn request_timeout_retries_are_bounded() {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
        .expect(2)
        .mount(&server)
        .await;
    let discovery = adapter(
        &server,
        DiscoveryOptions {
            request_timeout: Duration::from_millis(50),
            max_attempts: 2,
            ..Default::default()
        },
    );
    assert_eq!(
        discovery.discover().await.unwrap_err(),
        DiscoveryError::Transport
    );
    assert_eq!(discovery.inner.state.lock().await.attempts, 2);
}

#[tokio::test]
async fn changing_paginated_totals_fail_closed() {
    let server = MockServer::start().await;
    for (offset, total) in [(0, 2), (1, 3)] {
        Mock::given(query_param("offset", offset.to_string()))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-total-count", "1")
                    .insert_header("x-total-available", total.to_string())
                    .set_body_json(json!([row(&format!("192.0.2.{}", offset + 1))])),
            )
            .expect(1)
            .mount(&server)
            .await;
    }
    assert_eq!(
        adapter(
            &server,
            DiscoveryOptions {
                page_size: Some(1),
                max_pages: 3,
                ..Default::default()
            }
        )
        .discover()
        .await
        .unwrap_err(),
        DiscoveryError::IncompleteList
    );
}
