//! Shared transport policy. Keep one KestrelClient alive across related operations.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::search::KestrelError;

/// Settings applied to independent search and page-download pools.
#[derive(Clone, Debug)]
pub struct TransportOptions {
    pub pool_idle_timeout: Duration,
    /// Retained idle connections, not a limit on active connections or H2 streams.
    pub max_idle_per_host: usize,
    pub connect_timeout: Duration,
    pub stream_window_bytes: u32,
    pub connection_window_bytes: u32,
    /// Overrides fixed windows when enabled; may grow receive windows dynamically.
    pub adaptive_window: bool,
    /// Disabled by default. Servers may reject frequent PINGs.
    pub http2_ping_interval: Option<Duration>,
    pub http2_ping_timeout: Duration,
    pub http2_ping_while_idle: bool,
    /// Application cache lifetime; system resolver does not expose authoritative TTLs.
    pub dns_cache_ttl: Duration,
    pub dns_cache_capacity: usize,
}

impl Default for TransportOptions {
    fn default() -> Self {
        Self {
            pool_idle_timeout: Duration::from_secs(300),
            max_idle_per_host: 2,
            connect_timeout: Duration::from_secs(10),
            stream_window_bytes: 1024 * 1024,
            connection_window_bytes: 4 * 1024 * 1024,
            adaptive_window: false,
            http2_ping_interval: None,
            http2_ping_timeout: Duration::from_secs(10),
            http2_ping_while_idle: false,
            dns_cache_ttl: Duration::from_secs(60),
            dns_cache_capacity: 256,
        }
    }
}

impl TransportOptions {
    pub fn validate(&self) -> Result<(), KestrelError> {
        let invalid = self.pool_idle_timeout.is_zero()
            || self.connect_timeout.is_zero()
            || self.http2_ping_timeout.is_zero()
            || self.dns_cache_ttl.is_zero()
            || self.max_idle_per_host == 0
            || self.dns_cache_capacity == 0
            || !(65_535..=16 * 1024 * 1024).contains(&self.stream_window_bytes)
            || !(65_535..=64 * 1024 * 1024).contains(&self.connection_window_bytes)
            || self
                .http2_ping_interval
                .is_some_and(|d| d < Duration::from_secs(30))
            || (self.http2_ping_while_idle && self.http2_ping_interval.is_none());
        if invalid {
            return Err(KestrelError::InvalidRequest(
                "invalid transport settings: durations/capacities must be positive, stream window 65535..=16MiB, connection window 65535..=64MiB, PING interval >=30s, and idle PING requires an interval".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn standard_builder(&self) -> reqwest::ClientBuilder {
        reqwest::Client::builder()
            .pool_idle_timeout(self.pool_idle_timeout)
            .pool_max_idle_per_host(self.max_idle_per_host)
            .connect_timeout(self.connect_timeout)
            .tcp_nodelay(true)
            .tcp_keepalive(Duration::from_secs(60))
            .http2_initial_stream_window_size(self.stream_window_bytes)
            .http2_initial_connection_window_size(self.connection_window_bytes)
            .http2_adaptive_window(self.adaptive_window)
            .http2_keep_alive_interval(self.http2_ping_interval)
            .http2_keep_alive_timeout(self.http2_ping_timeout)
            .http2_keep_alive_while_idle(self.http2_ping_while_idle)
            .dns_resolver(Arc::new(CachedDns::new(self)))
            .redirect(reqwest::redirect::Policy::limited(10))
    }

    pub(crate) fn impersonated_builder(
        &self,
        profile: crate::http_client::BrowserProfile,
    ) -> primp::ClientBuilder {
        // Apply policy after impersonation, which otherwise overwrites H2 settings.
        primp::Client::builder()
            .impersonate(profile.browser)
            .impersonate_os(profile.os)
            .pool_idle_timeout(self.pool_idle_timeout)
            .pool_max_idle_per_host(self.max_idle_per_host)
            .connect_timeout(self.connect_timeout)
            .tcp_nodelay(true)
            .tcp_keepalive(Duration::from_secs(60))
            .http2_initial_stream_window_size(self.stream_window_bytes)
            .http2_initial_connection_window_size(self.connection_window_bytes)
            .http2_adaptive_window(self.adaptive_window)
            .http2_keep_alive_interval(self.http2_ping_interval)
            .http2_keep_alive_timeout(self.http2_ping_timeout)
            .http2_keep_alive_while_idle(self.http2_ping_while_idle)
            .dns_resolver(Arc::new(CachedDns::new(self)))
    }
}

type DnsEntries = HashMap<String, (Instant, Vec<SocketAddr>)>;

#[derive(Clone)]
struct CachedDns {
    entries: Arc<Mutex<DnsEntries>>,
    ttl: Duration,
    capacity: usize,
}

impl CachedDns {
    fn new(options: &TransportOptions) -> Self {
        Self {
            entries: Arc::default(),
            ttl: options.dns_cache_ttl,
            capacity: options.dns_cache_capacity,
        }
    }

    fn get(&self, name: &str) -> Option<Vec<SocketAddr>> {
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|_, (time, _)| time.elapsed() < self.ttl);
        entries.get(name).map(|(_, addrs)| addrs.clone())
    }

    fn insert(&self, name: String, addresses: Vec<SocketAddr>) {
        if addresses.is_empty() {
            return;
        }
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|_, (time, _)| time.elapsed() < self.ttl);
        if !entries.contains_key(&name)
            && entries.len() >= self.capacity
            && let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, (time, _))| *time)
                .map(|(name, _)| name.clone())
        {
            entries.remove(&oldest);
        }
        entries.insert(name, (Instant::now(), addresses));
    }

    async fn lookup(&self, name: String) -> std::io::Result<Vec<SocketAddr>> {
        if let Some(addresses) = self.get(&name) {
            return Ok(addresses);
        }
        // Preserve hosts-file, VPN and system DNS behavior. Never hold the lock over I/O.
        let addresses: Vec<_> = tokio::net::lookup_host((name.as_str(), 0)).await?.collect();
        self.insert(name, addresses.clone());
        Ok(addresses)
    }
}

impl reqwest::dns::Resolve for CachedDns {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let this = self.clone();
        let name = name.as_str().to_owned();
        Box::pin(async move {
            let addresses = this.lookup(name).await?;
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

impl primp::dns::Resolve for CachedDns {
    fn resolve(&self, name: primp::dns::Name) -> primp::dns::Resolving {
        let this = self.clone();
        let name = name.as_str().to_owned();
        Box::pin(async move {
            let addresses = this.lookup(name).await?;
            Ok(Box::new(addresses.into_iter()) as primp::dns::Addrs)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::future::join_all;
    use http_body_util::Full;
    use hyper::{body::Bytes, server::conn::http2, service::service_fn};
    use hyper_util::rt::{TokioExecutor, TokioIo};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Server {
        url: String,
        connections: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
        task: tokio::task::JoinHandle<()>,
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn h2_server(max_streams: u32, body_size: usize, delay: Duration) -> Server {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let connections = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let count = connections.clone();
        let max = peak.clone();
        let task = tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (socket, _) = accepted.unwrap();
                        count.fetch_add(1, Ordering::SeqCst);
                        let active = active.clone();
                        let max = max.clone();
                        tasks.spawn(async move {
                            let service = service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
                                let active = active.clone();
                                let max = max.clone();
                                async move {
                                    assert_eq!(request.version(), hyper::Version::HTTP_2);
                                    assert!(!request.headers().contains_key("connection"));
                                    let n = active.fetch_add(1, Ordering::SeqCst) + 1;
                                    max.fetch_max(n, Ordering::SeqCst);
                                    tokio::time::sleep(delay).await;
                                    active.fetch_sub(1, Ordering::SeqCst);
                                    let body = if request.method() == hyper::Method::HEAD { Vec::new() } else { vec![b'a'; body_size] };
                                    Ok::<_, std::convert::Infallible>(hyper::Response::builder()
                                        .header("content-type", "text/plain")
                                        .body(Full::new(Bytes::from(body))).unwrap())
                                }
                            });
                            http2::Builder::new(TokioExecutor::new()).max_concurrent_streams(max_streams)
                                .serve_connection(TokioIo::new(socket), service).await.ok();
                        });
                    }
                    _ = tasks.join_next(), if !tasks.is_empty() => {}
                }
            }
        });
        Server {
            url,
            connections,
            peak,
            task,
        }
    }

    fn h2_client(options: &TransportOptions) -> reqwest::Client {
        options
            .standard_builder()
            .no_proxy()
            .http2_prior_knowledge()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
    }

    #[test]
    fn rejects_invalid_policy() {
        assert!(TransportOptions::default().validate().is_ok());
        for options in [
            TransportOptions {
                stream_window_bytes: 0,
                ..Default::default()
            },
            TransportOptions {
                connection_window_bytes: u32::MAX,
                ..Default::default()
            },
            TransportOptions {
                max_idle_per_host: 0,
                ..Default::default()
            },
            TransportOptions {
                dns_cache_capacity: 0,
                ..Default::default()
            },
            TransportOptions {
                dns_cache_ttl: Duration::ZERO,
                ..Default::default()
            },
            TransportOptions {
                http2_ping_interval: Some(Duration::from_secs(1)),
                ..Default::default()
            },
            TransportOptions {
                http2_ping_while_idle: true,
                ..Default::default()
            },
            TransportOptions {
                connect_timeout: Duration::ZERO,
                ..Default::default()
            },
        ] {
            assert!(crate::KestrelClient::with_transport(options).is_err());
        }
    }

    #[test]
    fn dns_cache_expires_and_bounds_entries() {
        let cache = CachedDns::new(&TransportOptions {
            dns_cache_capacity: 2,
            ..Default::default()
        });
        let addresses = vec!["127.0.0.1:0".parse().unwrap()];
        cache.insert("a".into(), addresses.clone());
        cache.entries.lock().unwrap().get_mut("a").unwrap().0 =
            Instant::now() - Duration::from_secs(1);
        cache.insert("b".into(), addresses.clone());
        cache.insert("c".into(), addresses.clone());
        assert!(cache.get("a").is_none());
        assert_eq!(cache.clone().get("b"), Some(addresses));
        cache.entries.lock().unwrap().get_mut("b").unwrap().0 =
            Instant::now() - Duration::from_secs(61);
        assert!(cache.get("b").is_none());
        cache.insert("empty".into(), vec![]);
        assert!(cache.get("empty").is_none());
        assert_eq!(cache.entries.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn dns_adapter_resolves_system_hosts_and_preserves_zero_port() {
        let cache = CachedDns::new(&TransportOptions::default());
        let addresses = cache.lookup("localhost".into()).await.unwrap();
        assert!(!addresses.is_empty());
        assert!(addresses.iter().all(|a| a.port() == 0));
        assert_eq!(cache.get("localhost"), Some(addresses));
    }

    #[tokio::test]
    async fn h2_reuses_cloned_pool_and_respects_peer_stream_limit() {
        let server = h2_server(2, 128, Duration::from_millis(15)).await;
        let client = h2_client(&TransportOptions::default());
        client
            .get(&server.url)
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let clone = client.clone();
        let responses = join_all((0..12).map(|_| async {
            let response = clone.get(&server.url).send().await.unwrap();
            assert_eq!(response.version(), reqwest::Version::HTTP_2);
            assert_eq!(response.bytes().await.unwrap().len(), 128);
        }));
        tokio::time::timeout(Duration::from_secs(5), responses)
            .await
            .unwrap();
        assert_eq!(server.connections.load(Ordering::SeqCst), 1);
        assert_eq!(server.peak.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn h2_large_bodies_progress_with_fixed_and_adaptive_windows() {
        let server = h2_server(10, 2 * 1024 * 1024, Duration::ZERO).await;
        for adaptive_window in [false, true] {
            let client = h2_client(&TransportOptions {
                stream_window_bytes: 65_535,
                connection_window_bytes: 65_535,
                adaptive_window,
                ..Default::default()
            });
            let responses = join_all((0..3).map(|_| async {
                assert_eq!(
                    client
                        .get(&server.url)
                        .send()
                        .await
                        .unwrap()
                        .bytes()
                        .await
                        .unwrap()
                        .len(),
                    2 * 1024 * 1024
                );
            }));
            tokio::time::timeout(Duration::from_secs(5), responses)
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn impersonated_h2_pool_reuses_connections() {
        let server = h2_server(2, 256 * 1024, Duration::ZERO).await;
        let client = TransportOptions::default()
            .impersonated_builder(crate::http_client::BrowserProfile::random())
            .no_proxy()
            .http2_prior_knowledge()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        for _ in 0..3 {
            let response = client.clone().get(&server.url).send().await.unwrap();
            assert_eq!(response.version(), primp::Version::HTTP_2);
            assert_eq!(response.bytes().await.unwrap().len(), 256 * 1024);
        }
        assert_eq!(server.connections.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn warmup_and_fetch_share_pool_across_kestrel_clones() {
        let server = h2_server(2, 128, Duration::ZERO).await;
        let mut client = crate::KestrelClient::new().unwrap();
        client.fetch = h2_client(&TransportOptions::default());
        let warm = client
            .warm_up_fetch(
                &[
                    format!("{}a?q=secret", server.url),
                    format!("{}b", server.url),
                ],
                Duration::from_secs(2),
            )
            .await
            .unwrap();
        assert_eq!(warm.len(), 1);
        assert_eq!(warm[0].origin, server.url);
        assert_eq!(warm[0].http_version.as_deref(), Some("HTTP/2.0"));
        let report = client
            .clone()
            .fetch_all_detailed(
                std::slice::from_ref(&server.url),
                &crate::FetchOptions::default(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(report.pages[0].http_version.as_deref(), Some("HTTP/2.0"));
        assert_eq!(server.connections.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    #[ignore = "manual local transport benchmark; run with --ignored --nocapture"]
    async fn benchmark_cold_and_warm_h2() {
        let server = h2_server(100, 1024, Duration::ZERO).await;
        let options = TransportOptions::default();
        let mut cold = Vec::new();
        let mut warm = Vec::new();
        for _ in 0..100 {
            let client = h2_client(&options);
            let started = Instant::now();
            client
                .get(&server.url)
                .send()
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap();
            cold.push(started.elapsed().as_micros());
            let started = Instant::now();
            client
                .get(&server.url)
                .send()
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap();
            warm.push(started.elapsed().as_micros());
        }
        for (label, mut samples) in [("cold", cold), ("warm", warm)] {
            samples.sort_unstable();
            println!(
                "{label}: n=100 p50={}us p95={}us p99={}us",
                samples[49], samples[94], samples[98]
            );
        }
        assert_eq!(server.connections.load(Ordering::SeqCst), 100);
    }
}

#[cfg(test)]
mod tls_tests {
    use super::*;
    use http_body_util::Full;
    use hyper::{body::Bytes, service::service_fn};
    use hyper_util::rt::{TokioExecutor, TokioIo};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio_rustls::{TlsAcceptor, rustls};

    struct AbortOnDrop(tokio::task::JoinHandle<()>);
    impl Drop for AbortOnDrop {
        fn drop(&mut self) {
            self.0.abort();
        }
    }

    // Verify production ALPN negotiation and H1 fallback without prior knowledge
    // or disabling certificate verification, using a freshly generated test CA.
    #[tokio::test]
    async fn tls13_alpn_and_http1_fallback_reuse_both_client_pools() {
        for h2 in [true, false] {
            let rcgen::CertifiedKey { cert, signing_key } =
                rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
            let der = cert.der().clone();
            let mut config =
                rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
                    .with_no_client_auth()
                    .with_single_cert(
                        vec![der.clone()],
                        rustls::pki_types::PrivatePkcs8KeyDer::from(signing_key.serialize_der())
                            .into(),
                    )
                    .unwrap();
            config.alpn_protocols = vec![if h2 {
                b"h2".to_vec()
            } else {
                b"http/1.1".to_vec()
            }];
            let acceptor = TlsAcceptor::from(Arc::new(config));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let url = format!("https://localhost:{}/", address.port());
            let connections = Arc::new(AtomicUsize::new(0));
            let count = connections.clone();
            let server = AbortOnDrop(tokio::spawn(async move {
                let mut tasks = tokio::task::JoinSet::new();
                loop {
                    tokio::select! {
                        accepted = listener.accept() => {
                            let (socket, _) = accepted.unwrap();
                            count.fetch_add(1, Ordering::SeqCst);
                            let acceptor = acceptor.clone();
                            tasks.spawn(async move {
                                let socket = acceptor.accept(socket).await.unwrap();
                                assert_eq!(socket.get_ref().1.protocol_version(), Some(rustls::ProtocolVersion::TLSv1_3));
                                let service = service_fn(|_: hyper::Request<hyper::body::Incoming>| async {
                                    Ok::<_, std::convert::Infallible>(hyper::Response::new(Full::new(Bytes::from_static(b"ok"))))
                                });
                                if h2 {
                                    hyper::server::conn::http2::Builder::new(TokioExecutor::new()).serve_connection(TokioIo::new(socket), service).await.ok();
                                } else {
                                    hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(socket), service).await.ok();
                                }
                            });
                        }
                        result = tasks.join_next(), if !tasks.is_empty() => { result.unwrap().unwrap(); }
                    }
                }
            }));
            let options = TransportOptions::default();
            let standard = options
                .standard_builder()
                .no_proxy()
                .resolve("localhost", address)
                .tls_certs_merge([reqwest::Certificate::from_der(&der).unwrap()])
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap();
            let impersonated = options
                .impersonated_builder(crate::http_client::BrowserProfile::random())
                .no_proxy()
                .resolve("localhost", address)
                .add_root_certificate(primp::Certificate::from_der(&der).unwrap())
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap();
            let expected = if h2 { "HTTP/2.0" } else { "HTTP/1.1" };
            for _ in 0..3 {
                let response = standard.clone().get(&url).send().await.unwrap();
                assert_eq!(format!("{:?}", response.version()), expected);
                assert_eq!(response.text().await.unwrap(), "ok");
                let response = impersonated.clone().get(&url).send().await.unwrap();
                assert_eq!(format!("{:?}", response.version()), expected);
                assert_eq!(response.text().await.unwrap(), "ok");
            }
            assert_eq!(connections.load(Ordering::SeqCst), 2);
            drop(server);
        }
    }
}
