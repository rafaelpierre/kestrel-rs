//! Fresh-process exercise of the real CLI parser and page-attachment stage.
use super::*;
use std::{
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
#[ignore = "subprocess helper for committed_pages_survive_process_kill"]
async fn page_recovery_child() {
    let directory = std::env::var("KESTREL_RECOVERY_DIR").unwrap();
    let endpoint = std::env::var("KESTREL_RECOVERY_ENDPOINT").unwrap();
    let Commands::Search(args) = Cli::try_parse_from([
        "kestrel",
        "search",
        "fixture",
        "--cache-ttl",
        "60",
        "--cache-dir",
        &directory,
        "--no-rank",
        "--timeout",
        "30",
        "--fetch-budget",
        "20",
    ])
    .unwrap()
    .command
    else {
        panic!("search expected")
    };
    let mut results = ["fast", "slow"].map(|name| SearchResult {
        title: name.into(),
        url: format!("{endpoint}/{name}"),
        display_url: String::new(),
        snippet: String::new(),
        content: None,
        bm25_score: None,
        engine: None,
        query: None,
        engine_rank: None,
        sources: Vec::new(),
    });
    let report = attach_page_content(
        &KestrelClient::new().unwrap(),
        &mut results,
        &args,
        &mut Vec::new(),
    )
    .await
    .unwrap();
    assert!(!report.budget_exhausted);
    for (result, expected) in results
        .iter()
        .zip(["Fast committed evidence", "Slow completed evidence"])
    {
        assert!(result.content.as_deref().unwrap().contains(expected));
    }
    std::fs::write(
        std::path::Path::new(&directory).join("report.json"),
        serde_json::to_vec(
            &serde_json::json!({"hits":report.cache_hits,"contents":report.contents}),
        )
        .unwrap(),
    )
    .unwrap();
}

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn committed_pages_survive_process_kill() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let counts = Arc::new([AtomicUsize::new(0), AtomicUsize::new(0)]);
    let server_counts = counts.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let counts = server_counts.clone();
            tokio::spawn(async move {
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    if socket.read_exact(&mut byte).await.is_err() {
                        return;
                    }
                    request.push(byte[0]);
                    assert!(request.len() < 16_384);
                }
                let fast = request.starts_with(b"GET /fast ");
                assert!(fast || request.starts_with(b"GET /slow "));
                let index = usize::from(!fast);
                let attempt = counts[index].fetch_add(1, Ordering::SeqCst);
                if !fast && attempt == 0 {
                    // A real unfinished response; killing its client releases this socket.
                    let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 100000\r\n\r\n").await;
                    let mut byte = [0];
                    let _ = socket.read(&mut byte).await;
                    return;
                }
                let body = if fast {
                    "<main>Fast committed evidence from the deterministic local page.</main>"
                } else {
                    "<main>Slow completed evidence from the deterministic local page.</main>"
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    let directory = tempfile::tempdir().unwrap();
    let mut completed = Vec::new();
    for run in 0..3 {
        let log = directory.path().join(format!("child-{run}.log"));
        let output = std::fs::File::create(&log).unwrap();
        let mut child = Process(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "cli::recovery_tests::page_recovery_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env("KESTREL_RECOVERY_DIR", directory.path())
                .env("KESTREL_RECOVERY_ENDPOINT", &endpoint)
                .env("KESTRELSEARCH_OTEL_ENABLED", "false")
                .stdout(Stdio::from(output.try_clone().unwrap()))
                .stderr(Stdio::from(output))
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(15);
        if run == 0 {
            loop {
                let committed = std::fs::read_dir(directory.path())
                    .unwrap()
                    .filter_map(Result::ok)
                    .any(|entry| {
                        if entry.path().extension().and_then(|e| e.to_str()) != Some("txt") {
                            return false;
                        }
                        std::fs::read(entry.path())
                            .ok()
                            .and_then(|bytes| {
                                serde_json::from_slice::<serde_json::Value>(&bytes).ok()
                            })
                            .is_some_and(|page| {
                                page["content"]
                                    .as_str()
                                    .is_some_and(|text| text.contains("Fast committed evidence"))
                            })
                    });
                if committed && counts[1].load(Ordering::SeqCst) == 1 {
                    break;
                }
                assert!(
                    child.0.try_wait().unwrap().is_none(),
                    "{}",
                    std::fs::read_to_string(&log).unwrap()
                );
                assert!(Instant::now() < deadline, "commit observation timed out");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            child.0.kill().unwrap();
            assert!(!child.0.wait().unwrap().success());
        } else {
            loop {
                if let Some(status) = child.0.try_wait().unwrap() {
                    assert!(
                        status.success(),
                        "{}",
                        std::fs::read_to_string(&log).unwrap()
                    );
                    break;
                }
                assert!(Instant::now() < deadline, "recovery timed out");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            let report: serde_json::Value = serde_json::from_slice(
                &std::fs::read(directory.path().join("report.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(report["hits"], run);
            completed.push(report["contents"].clone());
        }
    }
    assert_eq!(completed[0], completed[1]);
    assert_eq!(counts.each_ref().map(|n| n.load(Ordering::SeqCst)), [1, 2]);
    server.abort();
}
