use std::fs;
use std::time::Duration;

use assert_cmd::Command;
use predicates::prelude::*;

fn completion_seconds(stderr: &[u8], command: &str) -> f64 {
    let stderr = std::str::from_utf8(stderr).unwrap();
    let prefix = format!("[kestrel] {command} completed in ");
    let lines: Vec<_> = stderr
        .lines()
        .filter_map(|line| line.strip_prefix(&prefix))
        .collect();
    assert_eq!(lines.len(), 1, "expected one completion line: {stderr}");
    let seconds = lines[0].strip_suffix(" seconds.").unwrap();
    assert_eq!(seconds.split_once('.').unwrap().1.len(), 3);
    let seconds: f64 = seconds.parse().unwrap();
    assert!(seconds.is_finite() && seconds >= 0.0);
    seconds
}

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
            .set_delay(Duration::from_millis(100))
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
            .clone();
        assert!(completion_seconds(&output.stderr, "Fetch") >= 0.1);
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 2);
        assert_eq!(value["url"], url);
        let content = value["content"].as_str().unwrap();
        assert!(content.starts_with(&format!("Source: {url}\n")));
        assert!(content.contains("meaningful article content"));
        assert!(!content.contains("Unwanted navigation"));
        let output = Command::cargo_bin("kestrel")
            .unwrap()
            .args(["fetch", &url, "--content-limit", "30"])
            .assert()
            .success()
            .stdout(predicate::str::starts_with(format!("Source: {url}\n")))
            .stdout(predicate::str::contains("Article heading"))
            .stdout(predicate::str::contains("known page URL").not())
            .get_output()
            .clone();
        assert!(completion_seconds(&output.stderr, "Fetch") >= 0.1);
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
                .stderr(predicate::str::contains("Fetch failed"))
                .stderr(predicate::str::contains("completed in").not());
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
    let skill = fs::read_to_string(&target).unwrap();
    assert!(skill.contains("retained prefix"));
    assert!(skill.contains("not cached"));
    assert!(skill.contains("--max-response-bytes 65536"));
    assert!(skill.contains("defaults to 1,000,000 decoded body bytes"));
    assert_eq!(skill.matches("[default: 1000000]").count(), 2);
    assert!(skill.contains("name: kestrelsearch"));
    assert!(skill.contains("Search completed in 1.234 seconds."));
    assert!(skill.contains("Fetch completed in 0.125 seconds."));
    assert!(skill.contains("Empty successful searches also report time"));
    assert!(skill.contains("--query-syntax"));
    assert!(skill.contains("Portable query syntax is the default for every provider"));
    assert!(skill.contains("Query constraints apply before counting"));
    assert!(skill.contains("--min-results"));
    assert!(skill.contains("Provider quorum is ignored"));
    assert!(
        skill
            .contains("[default: duckduckgo bing yahoo dogpile ecosia swisscows yep qwant mojeek]")
    );
    assert!(skill.contains("All nine supported engines are selected by default"));
    assert!(!skill.contains("Additional opt-in engines"));
    for subcommand in ["search", "fetch"] {
        let help = Command::cargo_bin("kestrel")
            .unwrap()
            .args([subcommand, "--help"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let help = String::from_utf8(help).unwrap();
        assert!(
            skill.contains(help.trim()),
            "installed skill must include live {subcommand} help"
        );
    }
    fs::write(&target, "stale skill").unwrap();
    Command::cargo_bin("kestrel")
        .unwrap()
        .current_dir(project.path())
        .env("HOME", user_home.path())
        .args([
            "skill", "install", "--agent", "codex", "--scope", "project", "--force",
        ])
        .assert()
        .success();
    assert_eq!(fs::read_to_string(&target).unwrap(), skill);

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

#[test]
fn malformed_primary_and_additional_queries_fail_before_search() {
    for args in [
        vec!["search", "\"machine"],
        vec!["search", "machine", "--query", "learning AND"],
        vec!["search", "filetype:pdf"],
    ] {
        Command::cargo_bin("kestrel")
            .unwrap()
            .args(args)
            .assert()
            .failure()
            .stderr(predicate::str::contains("Invalid portable query"))
            .stderr(predicate::str::contains("--query-syntax native"));
    }
}

#[test]
fn search_min_results_is_documented_and_validated() {
    Command::cargo_bin("kestrel")
        .unwrap()
        .args(["search", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--min-results <N>"));
    Command::cargo_bin("kestrel")
        .unwrap()
        .args(["search", "test", "--mode", "fanout", "--min-results", "0"])
        .assert()
        .failure();
    Command::cargo_bin("kestrel")
        .unwrap()
        .args(["search", "test", "--mode", "fallback", "--min-results", "5"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid value 'fallback'"));
}

#[test]
fn failed_search_has_no_success_completion_line() {
    // A one-nanosecond deadline expires before provider jobs start, avoiding live requests.
    for output in ["text", "json"] {
        Command::cargo_bin("kestrel")
            .unwrap()
            .args([
                "search",
                "query",
                "--no-fetch",
                "--search-budget",
                "0.000000001",
                "--output",
                output,
            ])
            .assert()
            .failure()
            .stdout("")
            .stderr(predicate::str::contains("Every search failed"))
            .stderr(predicate::str::contains("completed in").not());
    }
}

#[tokio::test]
async fn capped_fetch_returns_successful_text_and_json_with_stderr_notice() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "<main><p>This readable content survives the byte cutoff.</p><p>".to_owned()
                + &"later text ".repeat(100),
        ))
        .expect(2)
        .mount(&server)
        .await;
    let url = server.uri();
    tokio::task::spawn_blocking(move || {
        for format in ["text", "json"] {
            let output = Command::cargo_bin("kestrel")
                .unwrap()
                .args([
                    "fetch",
                    &url,
                    "--max-response-bytes",
                    "65",
                    "--output",
                    format,
                ])
                .assert()
                .success()
                .stderr(predicate::str::contains("page may be incomplete"))
                .get_output()
                .stdout
                .clone();
            let text = if format == "json" {
                let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
                assert_eq!(value.as_object().unwrap().len(), 2);
                assert_eq!(value["url"], url);
                value["content"].as_str().unwrap().to_owned()
            } else {
                String::from_utf8(output).unwrap()
            };
            assert!(text.contains("This readable content survives the byte cutoff."));
            assert!(!text.contains("later text"));
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn default_byte_cap_stops_at_one_mb_and_can_be_overridden() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    let server = MockServer::start().await;
    let body = "<main><p>This readable prefix is before the default cutoff.</p><!--".to_owned()
        + &"x".repeat(1_000_000)
        + "--><p>This readable tail is after the default cutoff.</p></main>";
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .expect(2)
        .mount(&server)
        .await;
    let url = server.uri();
    tokio::task::spawn_blocking(move || {
        Command::cargo_bin("kestrel")
            .unwrap()
            .args(["fetch", &url])
            .assert()
            .success()
            .stdout(predicate::str::contains("This readable prefix"))
            .stdout(predicate::str::contains("This readable tail").not())
            .stderr(predicate::str::contains("page may be incomplete"));
        Command::cargo_bin("kestrel")
            .unwrap()
            .args(["fetch", &url, "--max-response-bytes", "2000000"])
            .assert()
            .success()
            .stdout(predicate::str::contains("This readable prefix"))
            .stdout(predicate::str::contains("This readable tail"))
            .stderr(predicate::str::contains("page may be incomplete").not());
    })
    .await
    .unwrap();
}
