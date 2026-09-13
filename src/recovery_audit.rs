//! Synchronized baseline reproductions for #121, compiled only into the unit-test binary.
use std::{
    io::Write,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use tokio::io::{AsyncReadExt, AsyncWriteExt};

tokio::task_local! {
    pub(crate) static EVENTS: PathBuf;
}

pub(crate) fn observe(event: &str, evidence: &str) {
    let _ = EVENTS.try_with(|directory| {
        let temporary = directory.join(format!("{event}.tmp"));
        let mut file = std::fs::File::create(&temporary).unwrap();
        file.write_all(evidence.as_bytes()).unwrap();
        file.sync_all().unwrap();
        std::fs::rename(temporary, directory.join(event)).unwrap();
    });
}

#[tokio::test]
#[ignore = "subprocess helper; invoked by interrupted_work_is_repeated"]
async fn page_child() {
    let directory = PathBuf::from(std::env::var_os("KESTREL_AUDIT_DIR").unwrap());
    let endpoint = std::env::var("KESTREL_AUDIT_ENDPOINT").unwrap();
    EVENTS
        .scope(directory.clone(), async {
            let client = crate::KestrelClient::new().unwrap();
            let cache =
                crate::PageCache::new(directory.join("cache"), Duration::from_secs(60)).unwrap();
            client
                .fetch_all_cached_detailed(
                    &[format!("{endpoint}/fast"), format!("{endpoint}/slow")],
                    &crate::FetchOptions {
                        timeout: Duration::from_secs(30),
                        ..Default::default()
                    },
                    &cache,
                    None,
                )
                .await
                .unwrap();
            panic!("blocked slow page must prevent batch completion");
        })
        .await;
}

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interrupted_work_is_repeated() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let counts = Arc::new([
        AtomicUsize::new(0),
        AtomicUsize::new(0),
        AtomicUsize::new(0),
    ]);
    let server_counts = counts.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let counts = server_counts.clone();
            tokio::spawn(async move {
                let mut request = Vec::new();
                loop {
                    let mut byte = [0];
                    if socket.read_exact(&mut byte).await.is_err() {
                        return;
                    }
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                    assert!(request.len() < 16_384);
                }
                let request = String::from_utf8(request).unwrap();
                let (index, body, blocked) = if request.starts_with("GET /fast ") {
                    (
                        0,
                        "<main>Completed fast page evidence survives extraction.</main>",
                        false,
                    )
                } else if request.starts_with("GET /slow ") {
                    (1, "", true)
                } else {
                    assert!(request.starts_with("GET /provider "));
                    (
                        2,
                        "<li class=\"b_algo\"><h2><a href=\"https://example.org/record\">Accepted record</a></h2><div class=\"b_caption\"><p>Provider evidence.</p></div></li>",
                        true,
                    )
                };
                counts[index].fetch_add(1, Ordering::SeqCst);
                let length = if blocked { 100_000 } else { body.len() };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n{body}"
                );
                socket.write_all(response.as_bytes()).await.unwrap();
                if blocked {
                    // Wait for client cancellation, without sending EOF or a timed response.
                    let mut byte = [0];
                    let _ = socket.read(&mut byte).await;
                }
            });
        }
    });
    for (helper, event, expected) in [
        (
            "recovery_audit::page_child",
            "page-extracted",
            "Completed fast page evidence",
        ),
        (
            "search::streaming::tests::recovery_provider_child",
            "provider-accepted",
            "Accepted record",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut evidence = Vec::new();
        for run in 1..=2 {
            let event_path = directory.path().join(event);
            let _ = std::fs::remove_file(&event_path);
            let output =
                std::fs::File::create(directory.path().join(format!("child-{run}.log"))).unwrap();
            let child = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", helper, "--ignored", "--nocapture"])
                .env("KESTREL_AUDIT_DIR", directory.path())
                .env("KESTREL_AUDIT_ENDPOINT", &endpoint)
                .env("KESTRELSEARCH_OTEL_ENABLED", "false")
                .stdout(Stdio::from(output.try_clone().unwrap()))
                .stderr(Stdio::from(output))
                .spawn()
                .unwrap();
            let mut child = Process(child);
            let deadline = Instant::now() + Duration::from_secs(15);
            while !event_path.exists()
                || (event == "page-extracted" && counts[1].load(Ordering::SeqCst) < run)
            {
                assert!(
                    child.0.try_wait().unwrap().is_none(),
                    "helper exited before event: {}",
                    std::fs::read_to_string(directory.path().join(format!("child-{run}.log")))
                        .unwrap()
                );
                assert!(Instant::now() < deadline, "synchronized event timed out");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            let retained = std::fs::read_to_string(&event_path).unwrap();
            assert!(retained.contains(expected));
            evidence.push(retained);
            // kill is abrupt (SIGKILL on Unix); Drop always reaps even on assertion failure.
            child.0.kill().unwrap();
            assert!(!child.0.wait().unwrap().success());
            assert!(
                !directory.path().join("cache").exists(),
                "baseline unexpectedly persisted interrupted work"
            );
        }
        assert_eq!(
            evidence[0], evidence[1],
            "restart repeats identical accepted evidence"
        );
    }
    assert_eq!(
        counts.each_ref().map(|n| n.load(Ordering::SeqCst)),
        [2, 2, 2]
    );
    server.abort();
}
