//! Local protocol tests: the server never finishes capped responses.

use super::*;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};

use hyper::body::{Body, Bytes, Frame};
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{Barrier, Notify};

const PREFIX: &str = "<main><p>This readable text survives an unfinished HTTP response.</p>";

struct TestBody {
    bytes: Option<Bytes>,
    // The pending body's destruction signals that the client cancelled it.
    cancelled: Option<Arc<Notify>>,
}

impl Body for TestBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        if let Some(bytes) = self.bytes.take() {
            Poll::Ready(Some(Ok(Frame::data(bytes))))
        } else if self.cancelled.is_some() {
            Poll::Pending
        } else {
            Poll::Ready(None)
        }
    }
}

impl Drop for TestBody {
    fn drop(&mut self) {
        if let Some(cancelled) = &self.cancelled {
            cancelled.notify_one();
        }
    }
}

#[tokio::test]
async fn h2_cap_cancels_stream_without_aborting_another_request_or_pool() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let accepted = connections.clone();
    let barrier = Arc::new(Barrier::new(2));
    let cancelled = Arc::new(Notify::new());
    let server = tokio::spawn(async move {
        let mut tasks = tokio::task::JoinSet::new();
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            accepted.fetch_add(1, Ordering::SeqCst);
            let barrier = barrier.clone();
            let cancelled = cancelled.clone();
            tasks.spawn(async move {
                let service = service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
                    let barrier = barrier.clone();
                    let cancelled = cancelled.clone();
                    async move {
                        let path = request.uri().path();
                        if path == "/cap" || path == "/other" {
                            // Both requests must be active before sending the prefix.
                            barrier.wait().await;
                        }
                        if path == "/other" {
                            // A successful response proves the capped body was dropped
                            // on the server and the concurrent stream survived.
                            cancelled.notified().await;
                        }
                        Ok::<_, Infallible>(hyper::Response::new(TestBody {
                            bytes: Some(Bytes::from_static(PREFIX.as_bytes())),
                            cancelled: (path == "/cap").then_some(cancelled),
                        }))
                    }
                });
                hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                    .serve_connection(TokioIo::new(socket), service)
                    .await
                    .unwrap();
            });
        }
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .http2_prior_knowledge()
        .build()
        .unwrap();
    let base = format!("http://{address}");
    client
        .get(format!("{base}/warm"))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let options = FetchOptions {
        max_response_bytes: PREFIX.len(),
        ..FetchOptions::default()
    };
    let report = tokio::time::timeout(
        Duration::from_secs(3),
        fetch_all_reusing_client_with_diagnostics(
            &[format!("{base}/cap"), format!("{base}/other")],
            &options,
            &client,
            None,
        ),
    )
    .await
    .expect("fetch must not wait for EOF")
    .unwrap();
    for page in &report.pages {
        assert_eq!(page.http_version.as_deref(), Some("HTTP/2.0"));
        assert_eq!(page.outcome, FetchOutcome::Success);
        assert_eq!(page.response_bytes, PREFIX.len());
    }
    assert!(report.contents.iter().all(Option::is_some));
    client
        .get(format!("{base}/after"))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(connections.load(Ordering::SeqCst), 1);
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn h1_cap_disconnects_without_waiting_for_eof_or_next_chunk() {
    // Known length, unknown length, and a single chunk crossing the boundary.
    for (declared, suffix) in [(true, ""), (false, ""), (false, " unread suffix")] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                socket.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
            }
            let headers = if declared {
                "Content-Length: 100000\r\n"
            } else {
                "Transfer-Encoding: chunked\r\n"
            };
            socket
                .write_all(
                    format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n{headers}\r\n")
                        .as_bytes(),
                )
                .await
                .unwrap();
            let data = format!("{PREFIX}{suffix}");
            let body = if declared {
                data
            } else {
                format!("{:x}\r\n{data}\r\n", data.len())
            };
            socket.write_all(body.as_bytes()).await.unwrap();
            // No remaining bytes, chunk terminator, or EOF until client disconnects.
            let result = socket.read(&mut byte).await;
            assert!(matches!(result, Ok(0)) || result.is_err());
        });
        let options = FetchOptions {
            max_response_bytes: PREFIX.len(),
            ..FetchOptions::default()
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let report = tokio::time::timeout(
            Duration::from_secs(3),
            fetch_all_reusing_client_with_diagnostics(
                &[format!("http://{address}")],
                &options,
                &client,
                None,
            ),
        )
        .await
        .expect("fetch must stop at cap without EOF")
        .unwrap();
        assert_eq!(report.pages[0].outcome, FetchOutcome::Success);
        assert_eq!(report.pages[0].response_bytes, PREFIX.len());
        assert_eq!(
            report.contents[0].as_deref(),
            Some("This readable text survives an unfinished HTTP response.")
        );
        tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .expect("unread response must disconnect")
            .unwrap();
    }
}

#[tokio::test]
async fn failure_before_cap_does_not_return_partial_success() {
    for stall in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut byte = [0];
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                socket.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
            }
            socket
                .write_all(
                    format!("HTTP/1.1 200 OK\r\nContent-Length: 100000\r\n\r\n{PREFIX}").as_bytes(),
                )
                .await
                .unwrap();
            if stall {
                let _ = socket.read(&mut byte).await;
            }
            // Otherwise close early, before the declared length or byte cap.
        });
        let options = FetchOptions {
            max_response_bytes: 1000,
            timeout: Duration::from_millis(100),
            ..FetchOptions::default()
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let report = tokio::time::timeout(
            Duration::from_secs(3),
            fetch_all_reusing_client_with_diagnostics(
                &[format!("http://{address}")],
                &options,
                &client,
                None,
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(report.pages[0].outcome, FetchOutcome::RequestFailed);
        assert_eq!(report.contents[0], None);
        tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap();
    }
}
