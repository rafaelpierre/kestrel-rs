#![cfg(feature = "test-fixtures")]
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Fixture {
    endpoint: String,
    counts: Arc<Mutex<BTreeMap<String, usize>>>,
    block_provider: Arc<AtomicBool>,
    block_page: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let counts = Arc::new(Mutex::new(BTreeMap::new()));
        let block_provider = Arc::new(AtomicBool::new(true));
        let block_page = Arc::new(AtomicBool::new(false));
        let (c, b, p, e) = (
            counts.clone(),
            block_provider.clone(),
            block_page.clone(),
            endpoint.clone(),
        );
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let (c, b, p, e) = (c.clone(), b.clone(), p.clone(), e.clone());
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") {
                        let mut byte = [0];
                        if socket.read_exact(&mut byte).await.is_err() {
                            return;
                        }
                        request.push(byte[0]);
                        assert!(request.len() < 16384);
                    }
                    let request = String::from_utf8(request).unwrap();
                    let target = request.split_whitespace().nth(1).unwrap();
                    let url = url::Url::parse(&format!("{e}{target}")).unwrap();
                    let route = url.path();
                    let query = url
                        .query_pairs()
                        .find(|(k, _)| k == "q")
                        .map(|(_, v)| v.into_owned())
                        .unwrap_or_default();
                    let key = format!("{route}:{query}");
                    *c.lock().unwrap().entry(key).or_default() += 1;
                    let (body, blocked) = match route {
                        "/bing" | "/yahoo" => {
                            let body=["fast","slow"].into_iter().map(|name|if route=="/bing" {format!("<li class=\"b_algo\"><h2><a href=\"{e}/{name}\">{name} evidence</a></h2><div class=\"b_caption\"><p>Complete fixture snippet.</p></div></li>")}else{format!("<div class=\"dd algo\"><div class=\"compTitle\"><h3><a href=\"{e}/{name}\">{name} evidence</a></h3></div><div class=\"compText\"><p>Complete fixture snippet.</p></div></div>")}).collect::<String>();
                            (body, route == "/bing" && b.load(Ordering::SeqCst))
                        }
                        "/fast" => (
                            "<main>Fast complete page evidence from a local fixture.</main>".into(),
                            false,
                        ),
                        "/slow" => (
                            "<main>Slow complete page evidence from a local fixture.</main>".into(),
                            p.load(Ordering::SeqCst),
                        ),
                        _ => panic!("unexpected route {route}"),
                    };
                    let length = body.len() + if blocked { 100000 } else { 0 };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n{body}"
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    if blocked {
                        let mut byte = [0];
                        let _ = socket.read(&mut byte).await;
                    }
                });
            }
        });
        Self {
            endpoint,
            counts,
            block_provider,
            block_page,
            task,
        }
    }
    fn count(&self, route: &str) -> usize {
        self.counts
            .lock()
            .unwrap()
            .iter()
            .filter(|(k, _)| k.starts_with(route))
            .map(|(_, v)| v)
            .sum()
    }
}
struct Process {
    child: Child,
    stdout: PathBuf,
    stderr: PathBuf,
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn spawn(f: &Fixture, root: &Path, label: &str, min: &str, pages: bool, extra: &[&str]) -> Process {
    let stdout = root.join(format!("{label}.json"));
    let stderr = root.join(format!("{label}.log"));
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kestrel"));
    cmd.args([
        "search",
        "one",
        "-q",
        "two",
        "-e",
        "bing",
        "-e",
        "yahoo",
        "--no-rank",
        "-k",
        "20",
        "--min-results",
        min,
        "--recovery-dir",
    ])
    .arg(root.join("progress"))
    .args(["--output", "json"]);
    if !extra.contains(&"--search-budget") {
        cmd.args(["--search-budget", "20"]);
    }
    if !extra.contains(&"--recovery-ttl") {
        cmd.args(["--recovery-ttl", "60"]);
    }
    if pages {
        cmd.args(["--cache-ttl", "60", "--cache-dir"])
            .arg(root.join("pages"))
            .args(["--fetch-budget", "20", "--timeout", "20"]);
    } else {
        cmd.arg("--no-fetch");
    }
    let child = cmd
        .args(extra)
        .env("KESTREL_TEST_PROVIDER_ENDPOINT", &f.endpoint)
        .env("KESTRELSEARCH_OTEL_ENABLED", "false")
        .stdout(Stdio::from(std::fs::File::create(&stdout).unwrap()))
        .stderr(Stdio::from(std::fs::File::create(&stderr).unwrap()))
        .spawn()
        .unwrap();
    Process {
        child,
        stdout,
        stderr,
    }
}
fn snapshots(root: &Path) -> Vec<Value> {
    std::fs::read_dir(root.join("progress"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("txt"))
        .filter_map(|e| serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok())
        .collect()
}
async fn until(mut ready: impl FnMut() -> bool, process: &mut Process) {
    tokio::time::timeout(Duration::from_secs(12), async {
        while !ready() {
            assert!(
                process.child.try_wait().unwrap().is_none(),
                "child exited: {}",
                std::fs::read_to_string(&process.stderr).unwrap()
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}
async fn finish(process: &mut Process, code: i32) -> Option<Value> {
    let status = tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            if let Some(status) = process.child.try_wait().unwrap() {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        status.code(),
        Some(code),
        "{}",
        std::fs::read_to_string(&process.stderr).unwrap()
    );
    if code == 0 {
        Some(serde_json::from_slice(&std::fs::read(&process.stdout).unwrap()).unwrap())
    } else {
        None
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_replays_repeated_interruptions_and_skips_completed_work() {
    let f = Fixture::new().await;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut first = spawn(&f, root, "first", "99", false, &[]);
    until(
        || {
            let s = snapshots(root);
            s.len() == 4
                && s.iter()
                    .all(|s| s["records"].as_array().unwrap().len() == 2)
                && s.iter().filter(|s| s["state"] == "complete").count() == 2
        },
        &mut first,
    )
    .await;
    let generation = snapshots(root)[0]["generation"].clone();
    first.child.kill().unwrap();
    assert!(!first.child.wait().unwrap().success());
    assert_eq!((f.count("/bing"), f.count("/yahoo")), (2, 2));
    let mut second = spawn(&f, root, "second", "99", false, &[]);
    until(
        || {
            snapshots(root)
                .iter()
                .filter(|s| s["key"]["engine"] == "bing")
                .all(|s| {
                    s["generation"] != generation && s["records"].as_array().unwrap().len() == 2
                })
                && f.count("/bing") == 4
        },
        &mut second,
    )
    .await;
    second.child.kill().unwrap();
    assert!(!second.child.wait().unwrap().success());
    let mut small = spawn(&f, root, "small", "1", false, &[]);
    let small = finish(&mut small, 0).await.unwrap();
    assert_eq!(small["results"].as_array().unwrap().len(), 2);
    for row in small["results"].as_array().unwrap() {
        assert_eq!(row["sources"].as_array().unwrap().len(), 4);
    }
    assert_eq!((f.count("/bing"), f.count("/yahoo")), (4, 2));
    f.block_provider.store(false, Ordering::SeqCst);
    let mut complete = spawn(&f, root, "complete", "99", false, &[]);
    let complete = finish(&mut complete, 0).await.unwrap();
    assert_eq!(small["results"], complete["results"]);
    assert_eq!((f.count("/bing"), f.count("/yahoo")), (6, 2));
    assert!(snapshots(root).iter().all(|s| s["state"] == "complete"));
    let mut warm = spawn(&f, root, "warm", "999", false, &[]);
    let warm = finish(&mut warm, 0).await.unwrap();
    assert_eq!(complete["results"], warm["results"]);
    assert_eq!((f.count("/bing"), f.count("/yahoo")), (6, 2));
    let stdout = root.join("reordered.json");
    let stderr = root.join("reordered.log");
    let mut reordered = Process {
        child: Command::new(env!("CARGO_BIN_EXE_kestrel"))
            .args([
                "search",
                "two",
                "-q",
                "one",
                "-e",
                "yahoo",
                "-e",
                "bing",
                "--no-fetch",
                "--no-rank",
                "-k",
                "20",
                "--min-results",
                "999",
                "--recovery-ttl",
                "60",
                "--recovery-dir",
            ])
            .arg(root.join("progress"))
            .args(["--output", "json"])
            .env("KESTREL_TEST_PROVIDER_ENDPOINT", &f.endpoint)
            .env("KESTRELSEARCH_OTEL_ENABLED", "false")
            .stdout(Stdio::from(std::fs::File::create(&stdout).unwrap()))
            .stderr(Stdio::from(std::fs::File::create(&stderr).unwrap()))
            .spawn()
            .unwrap(),
        stdout,
        stderr,
    };
    let reordered = finish(&mut reordered, 0).await.unwrap();
    assert_eq!(reordered["results"][0]["engine"], "yahoo");
    assert_eq!(reordered["results"][0]["query"], "two");
    assert!(
        reordered["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["sources"].as_array().unwrap().len() == 4)
    );
    assert_eq!((f.count("/bing"), f.count("/yahoo")), (6, 2));
    // Corrupt one complete unit: only that request becomes necessary.
    let damaged = std::fs::read_dir(root.join("progress"))
        .unwrap()
        .filter_map(Result::ok)
        .find(|e| e.path().extension().and_then(|s| s.to_str()) == Some("txt"))
        .unwrap()
        .path();
    std::fs::write(&damaged, b"torn").unwrap();
    let mut retry = spawn(&f, root, "corrupt", "99", false, &[]);
    finish(&mut retry, 0).await.unwrap();
    assert_eq!(f.count("/bing") + f.count("/yahoo"), 9);
    // Page interruption with completed provider work: no discovery request repeats.
    f.block_page.store(true, Ordering::SeqCst);
    let mut page = spawn(&f, root, "pages", "99", true, &[]);
    until(
        || {
            std::fs::read_dir(root.join("pages"))
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .any(|e| e.path().extension().and_then(|s| s.to_str()) == Some("txt"))
                && f.count("/slow") == 1
        },
        &mut page,
    )
    .await;
    page.child.kill().unwrap();
    assert!(!page.child.wait().unwrap().success());
    f.block_page.store(false, Ordering::SeqCst);
    let mut page_retry = spawn(&f, root, "pages-retry", "99", true, &[]);
    let pages = finish(&mut page_retry, 0).await.unwrap();
    assert!(pages["results"].as_array().unwrap().iter().all(|r| {
        r["content"]
            .as_str()
            .is_some_and(|s| s.contains("complete page evidence"))
    }));
    assert_eq!((f.count("/fast"), f.count("/slow")), (1, 2));
    assert_eq!(f.count("/bing") + f.count("/yahoo"), 9);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn graceful_signal_preserves_incomplete_units() {
    let f = Fixture::new().await;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut child = spawn(&f, root, "signal", "99", false, &[]);
    until(|| snapshots(root).len() == 4, &mut child).await;
    assert!(
        Command::new("kill")
            .args(["-TERM", &child.child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    finish(&mut child, 130).await;
    assert!(
        std::fs::read_to_string(&child.stderr)
            .unwrap()
            .contains("draining committed provider work")
    );
    let mut retry = spawn(&f, root, "signal-retry", "1", false, &[]);
    finish(&mut retry, 0).await.unwrap();
    assert_eq!((f.count("/bing"), f.count("/yahoo")), (2, 2));
    f.block_provider.store(false, Ordering::SeqCst);
    let mut complete = spawn(&f, root, "signal-complete", "99", false, &[]);
    finish(&mut complete, 0).await.unwrap();
    f.block_page.store(true, Ordering::SeqCst);
    let mut page = spawn(&f, root, "signal-page", "99", true, &[]);
    until(
        || {
            std::fs::read_dir(root.join("pages"))
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .any(|e| e.path().extension().and_then(|s| s.to_str()) == Some("txt"))
                && f.count("/slow") == 1
        },
        &mut page,
    )
    .await;
    assert!(
        Command::new("kill")
            .args(["-INT", &page.child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    finish(&mut page, 130).await;
    assert!(
        std::fs::read_to_string(&page.stderr)
            .unwrap()
            .contains("draining page work")
    );
    f.block_page.store(false, Ordering::SeqCst);
    let mut retry = spawn(&f, root, "signal-page-retry", "99", true, &[]);
    finish(&mut retry, 0).await.unwrap();
    assert_eq!((f.count("/fast"), f.count("/slow")), (1, 2));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deadline_expiry_eviction_settings_and_concurrent_processes() {
    let f = Fixture::new().await;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut deadline = spawn(
        &f,
        root,
        "deadline",
        "99",
        false,
        &["--search-budget", "0.5"],
    );
    let result = finish(&mut deadline, 0).await.unwrap();
    assert_eq!(result["results"].as_array().unwrap().len(), 2);
    assert!(snapshots(root).iter().any(|s| s["state"] == "incomplete"));
    let counts = (f.count("/bing"), f.count("/yahoo"));
    let mut retry = spawn(&f, root, "deadline-retry", "1", false, &[]);
    finish(&mut retry, 0).await.unwrap();
    assert_eq!(counts, (f.count("/bing"), f.count("/yahoo")));
    f.block_provider.store(false, Ordering::SeqCst);
    // Expiry misses all four units; fresh invocation budgets are independent.
    let mut expired = spawn(
        &f,
        root,
        "expired",
        "99",
        false,
        &["--recovery-ttl", "0.000000001"],
    );
    finish(&mut expired, 0).await.unwrap();
    assert_eq!(
        (f.count("/bing"), f.count("/yahoo")),
        (counts.0 + 2, counts.1 + 2)
    );
    assert!(
        std::fs::read_to_string(&expired.stderr)
            .unwrap()
            .contains("expired")
    );
    let before = f.count("/bing") + f.count("/yahoo");
    let mut region = spawn(&f, root, "region", "99", false, &["--region", "gb"]);
    finish(&mut region, 0).await.unwrap();
    assert_eq!(f.count("/bing") + f.count("/yahoo"), before + 4);
    // Moving recency windows deliberately never reuse an old coverage window.
    let before = f.count("/bing") + f.count("/yahoo");
    for label in ["recent-one", "recent-two"] {
        let mut recent = spawn(&f, root, label, "99", false, &["--time-filter", "d"]);
        finish(&mut recent, 0).await.unwrap();
    }
    assert_eq!(f.count("/bing") + f.count("/yahoo"), before + 8);
    // Explicit eviction is an expected miss, independently of corruption/expiry.
    for entry in std::fs::read_dir(root.join("progress"))
        .unwrap()
        .filter_map(Result::ok)
    {
        if entry.path().extension().and_then(|s| s.to_str()) == Some("txt") {
            std::fs::remove_file(entry.path()).unwrap();
        }
    }
    let before = f.count("/bing") + f.count("/yahoo");
    let mut evicted = spawn(&f, root, "evicted", "99", false, &[]);
    finish(&mut evicted, 0).await.unwrap();
    assert_eq!(f.count("/bing") + f.count("/yahoo"), before + 4);
    let concurrent = tempfile::tempdir().unwrap();
    let mut a = spawn(&f, concurrent.path(), "a", "99", false, &[]);
    let mut b = spawn(&f, concurrent.path(), "b", "99", false, &[]);
    let a = finish(&mut a, 0).await.unwrap();
    let b = finish(&mut b, 0).await.unwrap();
    assert_eq!(a["results"], b["results"]);
    assert_eq!(snapshots(concurrent.path()).len(), 4);
    assert!(
        snapshots(concurrent.path())
            .iter()
            .all(|s| s["state"] == "complete")
    );
    let before = f.count("/bing") + f.count("/yahoo");
    let mut warm = spawn(&f, concurrent.path(), "after-concurrent", "99", false, &[]);
    let warm = finish(&mut warm, 0).await.unwrap();
    assert_eq!(warm["results"], a["results"]);
    assert_eq!(f.count("/bing") + f.count("/yahoo"), before);
}
