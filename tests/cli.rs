use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_lists_search_and_skill_commands() {
    Command::cargo_bin("kestrel")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("search"))
        .stdout(predicate::str::contains("fetch"))
        .stdout(predicate::str::contains("skill"));
}

#[test]
fn fetch_rejects_invalid_urls_and_limits() {
    for url in [
        "test",
        "site:https://example.com/page",
        "file:///tmp/page",
        "ftp://example.com",
    ] {
        Command::cargo_bin("kestrel")
            .unwrap()
            .args(["fetch", url])
            .assert()
            .failure()
            .stderr(predicate::str::contains("full HTTP or HTTPS URL"));
    }
    for option in ["--content-limit", "--max-response-bytes", "--timeout"] {
        Command::cargo_bin("kestrel")
            .unwrap()
            .args(["fetch", "https://example.com", option, "0"])
            .assert()
            .failure();
    }
}

#[tokio::test]
async fn fetch_extracts_a_known_url_as_text_or_json() {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/article"))
        .respond_with(ResponseTemplate::new(200)
            .insert_header("content-type", "text/html")
            .set_body_string("<nav>Unwanted navigation</nav><main><h1>Article heading</h1><p>This is meaningful article content fetched directly from a known page URL.</p></main>"))
        .expect(2)
        .mount(&server).await;
    let url = format!("{}/article", server.uri());
    tokio::task::spawn_blocking(move || {
        let output = Command::cargo_bin("kestrel")
            .unwrap()
            .args(["fetch", &url, "--output", "json"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(value["url"], url);
        let content = value["content"].as_str().unwrap();
        assert!(content.starts_with(&format!("Source: {url}\n")));
        assert!(content.contains("meaningful article content"));
        assert!(!content.contains("Unwanted navigation"));
        Command::cargo_bin("kestrel")
            .unwrap()
            .args(["fetch", &url, "--content-limit", "30"])
            .assert()
            .success()
            .stdout(predicate::str::starts_with(format!("Source: {url}\n")))
            .stdout(predicate::str::contains("Article heading"))
            .stdout(predicate::str::contains("known page URL").not());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn fetch_reports_http_and_unsupported_content_failures() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
    let server = MockServer::start().await;
    Mock::given(path("/missing"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(path("/document.pdf"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/pdf")
                .set_body_string("PDF"),
        )
        .mount(&server)
        .await;
    let base = server.uri();
    tokio::task::spawn_blocking(move || {
        for path in ["/missing", "/document.pdf"] {
            Command::cargo_bin("kestrel")
                .unwrap()
                .args(["fetch", &format!("{base}{path}"), "--output", "json"])
                .assert()
                .failure()
                .stdout("")
                .stderr(predicate::str::contains("Fetch failed"));
        }
    })
    .await
    .unwrap();
}

#[test]
fn version_matches_package_version() {
    Command::cargo_bin("kestrel")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(concat!(
            "kestrel ",
            env!("CARGO_PKG_VERSION")
        )));
}

#[test]
fn invalid_positive_limits_are_rejected() {
    Command::cargo_bin("kestrel")
        .unwrap()
        .args(["search", "query", "--top-k", "0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("must be at least 1"));
}

#[test]
fn skill_install_and_uninstall_use_compatible_paths() {
    let project = tempfile::tempdir().unwrap();
    let user_home = tempfile::tempdir().unwrap();
    Command::cargo_bin("kestrel")
        .unwrap()
        .current_dir(project.path())
        .env("HOME", user_home.path())
        .args(["skill", "install", "--agent", "codex", "--scope", "project"])
        .assert()
        .success();
    let target = project.path().join(".codex/skills/kestrelsearch/SKILL.md");
    assert!(target.exists());
    assert!(
        fs::read_to_string(&target)
            .unwrap()
            .contains("name: kestrelsearch")
    );

    Command::cargo_bin("kestrel")
        .unwrap()
        .current_dir(project.path())
        .env("HOME", user_home.path())
        .args(["skill", "uninstall"])
        .write_stdin("all\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed:"));
    assert!(!target.exists());
}
