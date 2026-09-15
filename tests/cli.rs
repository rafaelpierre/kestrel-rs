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
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
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
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
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
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
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
            .set_body_bytes("<nav>Unwanted navigation</nav><main><div class=\"download\"><h1>Article heading</h1><p>This is meaningful article content fetched directly from a known page URL.</p></div></main>"))
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
        assert_eq!(value.as_object().unwrap().len(), 4);
        let seconds = value["elapsed_seconds"].as_f64().unwrap();
        assert!(seconds.is_finite() && seconds >= 0.1);
        // Stderr rounds its later sample to milliseconds.
        assert!(seconds <= completion_seconds(&output.stderr, "Fetch") + 0.001);
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
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
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
                .set_body_bytes("PDF"),
        )
        .mount(&server)
        .await;
    Mock::given(path("/empty.md"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(" \r\n\t", "text/markdown"))
        .mount(&server)
        .await;
    let base = server.uri();
    tokio::task::spawn_blocking(move || {
        for (path, reason) in [
            ("/missing", "HTTP request failed"),
            ("/document.pdf", "expected HTML, plain text, or Markdown"),
            ("/empty.md", "no extractable page text"),
        ] {
            Command::cargo_bin("kestrel")
                .unwrap()
                .args(["fetch", &format!("{base}{path}"), "--output", "json"])
                .assert()
                .failure()
                .stdout("")
                .stderr(predicate::str::contains("Fetch failed"))
                .stderr(predicate::str::contains(reason))
                .stderr(predicate::str::contains("completed in").not());
        }
    })
    .await
    .unwrap();
}

#[test]
fn version_matches_package_version() {
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
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
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
    Command::cargo_bin("kestrel")
        .unwrap()
        .args(["search", "query", "--top-k", "0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("must be at least 1"));
}

#[test]
fn skill_install_and_uninstall_use_compatible_paths() {
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
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
    assert!(
        user_home
            .path()
            .join(".kestrelsearch/config.toml.lock")
            .exists()
    );
    for guidance in [
        "Do not delete this lock file",
        "lock contention fails after ten seconds",
        "Config writes use atomic replacement",
        "Invalid TOML returns an error without overwriting it",
        "dangling symlinks are rejected",
        "bookkeeping are separate operations",
    ] {
        assert!(
            skill.contains(guidance),
            "missing installed guidance: {guidance}"
        );
    }
    for guidance in [
        "## Formulate keyword FTS queries",
        "NEVER submit conversational questions",
        "Apply this rule to every `--query` and every recovery search",
        "| `why is the sky blue Rayleigh scattering` | `Rayleigh scattering blue sky` |",
        "| `What is the capital of Australia?` | `Australia capital` |",
        "| `Rust E0382 use of moved value how to fix` | `Rust E0382 use of moved value` |",
        "Preserve exact error messages such as `use of moved value`",
        "shortening a query must not relax its requirements",
        "Kestrel passes query text to",
        "providers unchanged",
    ] {
        assert!(skill.contains(guidance), "missing FTS guidance: {guidance}");
    }
    for guidance in [
        "clones share an aggregate parser capacity",
        "with_parser_capacity(n)",
        "larger per-call limits do not raise the shared cap",
        "runtime shutdown may still wait",
    ] {
        assert!(
            skill.contains(guidance),
            "missing parser capacity guidance: {guidance}"
        );
    }
    assert!(skill.contains("Only ordinary deadline outcomes are eligible"));
    assert!(skill.contains("unknown report outcomes must not be treated as retryable deadlines"));
    assert!(skill.contains("retained prefix"));
    assert!(skill.contains("Completed transport challenge diagnostics and HTML/JSON extraction share one worker-local representation"));
    assert!(skill.contains("JSON snapshots likewise reuse their value"));
    assert!(skill.contains("remain distinct parsing inputs"));
    assert!(skill.contains("embedded CAPTCHA markup does not by itself"));
    assert!(skill.contains("round to at least one nanosecond"));
    assert!(skill.contains("Semaphore::MAX_PERMITS"));
    assert!(skill.contains("checked `3 * top-k`"));
    assert!(skill.contains("whole class tokens"));
    for guidance in [
        "HTML text follows document order",
        "tabs between table cells",
        "`pre` preserves indentation",
        "Entities are decoded once",
        "HTML headings have no implicit ranking boost",
        "including retained whitespace and generated separators",
    ] {
        assert!(
            skill.contains(guidance),
            "missing extraction guidance: {guidance}"
        );
    }
    assert!(skill.contains("Content-quality assessment is advisory"));
    assert!(skill.contains("32,768 UTF-8 bytes"));
    assert!(skill.contains("comments-only `main`"));
    assert!(skill.contains("diagnostics.candidate_content_quality"));
    assert!(skill.contains("No quality rejection or ranking penalty"));
    assert!(skill.contains("support `text/plain`"));
    assert!(skill.contains("literal markup/entities"));
    assert!(skill.contains("whitespace-only retained plain text"));
    assert!(skill.contains("`text/markdown`"));
    assert!(skill.contains("Markdown is returned as source"));
    assert!(skill.contains("text/markdown-extra"));
    assert!(skill.contains("decode with replacement characters"));
    assert!(skill.contains("download`, `reader`, `shadow`, and `thread"));
    assert!(skill.contains("not cached"));
    assert!(skill.contains("--max-response-bytes 65536"));
    assert!(skill.contains("defaults to 1,000,000 decoded body bytes"));
    assert_eq!(skill.matches("[default: 1000000]").count(), 2);
    assert!(skill.contains("name: kestrelsearch"));
    assert!(skill.contains("elapsed_seconds"));
    assert!(skill.contains("jq '.results[]'"));
    assert!(skill.contains("before JSON serialization/output"));
    assert!(!skill.contains("returns an array"));
    assert!(skill.contains("Search completed in 1.234 seconds."));
    assert!(skill.contains("Fetch completed in 0.125 seconds."));
    assert!(skill.contains("Empty successful searches also report time"));
    assert!(skill.contains("--query-syntax"));
    for contract in [
        "--min-fetch-score",
        "score >= SCORE",
        "zero keeps zero",
        "without lexical terms",
        "fetch_score_bypassed_queries",
        "whitespace-only queries are dropped",
        "fetching AND final",
        "no page/cache work",
    ] {
        assert!(
            skill.contains(contract),
            "missing fetch score contract: {contract}"
        );
    }
    assert!(skill.contains("Defaults do not cause conflicts"));
    assert!(skill.contains("## Choosing limits: collected, fetched, returned"));
    assert!(skill.contains("## Recipes: speed, coverage and relevance"));
    assert!(skill.contains("not replenished"));
    assert!(skill.contains("not 5,000 total output characters"));
    assert!(skill.contains("does not itself stop the network download sooner"));
    assert!(skill.contains("--min-results 15 --fetch-candidates 15"));
    assert!(skill.contains("usage status 2 before requests"));
    assert!(skill.contains("Choose either `--rank` or `--ranking-policy`, never both"));
    assert!(skill.contains("Provider-native passthrough is the default"));
    assert!(skill.contains("Portable mode and --query-syntax have been removed"));
    assert!(!skill.contains("--query-syntax <"));
    assert!(!skill.contains("[default: portable]"));
    assert!(skill.contains("Query constraints apply before counting"));
    assert!(skill.contains("--min-results"));
    assert!(skill.contains("Provider quorum is ignored"));
    assert!(
        skill.contains(
            "Omitting the minimum still means five, regardless of quorum or search budget"
        )
    );
    assert!(
        skill
            .contains("[default: duckduckgo bing yahoo dogpile ecosia swisscows yep qwant mojeek]")
    );
    assert!(skill.contains("All nine supported engines are selected by default"));
    assert!(skill.contains("exclude Bing and Yahoo skip their impersonated transports"));
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
    let unrelated = project.path().join(".claude/skills/kestrelsearch/SKILL.md");
    fs::create_dir_all(unrelated.parent().unwrap()).unwrap();
    fs::write(&unrelated, "unrelated installation").unwrap();
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
    assert_eq!(
        fs::read_to_string(unrelated).unwrap(),
        "unrelated installation"
    );
}

#[test]
fn removed_query_syntax_option_fails_before_requests() {
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
    for syntax in ["portable", "native"] {
        Command::cargo_bin("kestrel")
            .unwrap()
            .args(["search", "machine learning", "--query-syntax", syntax])
            .assert()
            .code(2)
            .stdout("")
            .stderr(predicate::str::contains(
                "unexpected argument '--query-syntax'",
            ));
    }
}

#[cfg(feature = "test-fixtures")]
#[tokio::test(flavor = "multi_thread")]
async fn passthrough_accepts_provider_syntax_without_local_parser_errors() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string("<li class=\"b_no\">No results</li>"),
        )
        .mount(&server)
        .await;
    for query in ["filetype:pdf", "learning AND", "\"machine", "a|b"] {
        Command::cargo_bin("kestrel")
            .unwrap()
            .env("KESTREL_TEST_PROVIDER_ENDPOINT", server.uri())
            .args([
                "search",
                query,
                "--engine",
                "bing",
                "--no-fetch",
                "--search-budget",
                "15",
                "--output",
                "json",
            ])
            .assert()
            .success();
    }
    assert_eq!(server.received_requests().await.unwrap().len(), 4);
}

#[test]
fn contradictory_search_options_fail_before_requests_in_either_order() {
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
    let mut pairs = vec![
        (vec!["--fetch"], vec!["--no-fetch"]),
        (vec!["--rank"], vec!["--no-rank"]),
        (vec!["--no-fetch"], vec!["--rank"]),
        (vec!["--no-fetch"], vec!["--pre-rank"]),
        (vec!["--no-fetch"], vec!["--ranking-policy", "body"]),
        (vec!["--search-budget", "5"], vec!["--no-search-budget"]),
    ];
    for policy in ["provider", "snippet", "body", "hybrid", "rrf"] {
        pairs.push((vec!["--rank"], vec!["--ranking-policy", policy]));
        pairs.push((vec!["--no-rank"], vec!["--ranking-policy", policy]));
    }
    for option in [
        "--fetch-candidates",
        "--content-limit",
        "--max-response-bytes",
        "--timeout",
        "--fetch-budget",
        "--cache-ttl",
        "--cache-max-entries",
        "--concurrency",
        "--parse-concurrency",
    ] {
        pairs.push((vec!["--no-fetch"], vec![option, "10"]));
    }
    pairs.push((vec!["--no-fetch"], vec!["--cache-dir", "cache"]));
    for (left, right) in pairs {
        for (first, second) in [(&left, &right), (&right, &left)] {
            Command::cargo_bin("kestrel")
                .unwrap()
                .args(["search", "test"])
                .args(first)
                .args(second)
                .assert()
                .code(2)
                .stdout("")
                .stderr(predicate::str::contains("cannot be used with"))
                .stderr(predicate::str::contains("Searching").not());
        }
    }
}

#[test]
fn search_min_results_is_documented_and_validated() {
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
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
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
    // Runtime validation failure is deterministic and performs no provider work.
    for output in ["text", "json"] {
        Command::cargo_bin("kestrel")
            .unwrap()
            .args([
                "search",
                " ",
                "--no-fetch",
                "--search-budget",
                "0.000000001",
                "--output",
                output,
            ])
            .assert()
            .failure()
            .stdout("")
            .stderr(predicate::str::contains("At least one non-empty query"))
            .stderr(predicate::str::contains("completed in").not());
    }
}

#[tokio::test]
async fn capped_fetch_returns_successful_text_and_json_with_stderr_notice() {
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html")
                .set_body_bytes(
                    "<main><p>This readable content survives the byte cutoff.</p><p>".to_owned()
                        + &"later text ".repeat(100),
                ),
        )
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
                assert_eq!(value.as_object().unwrap().len(), 4);
                let seconds = value["elapsed_seconds"].as_f64().unwrap();
                assert!(seconds.is_finite() && seconds >= 0.0);
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
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    let server = MockServer::start().await;
    let body = "<main><p>This readable prefix is before the default cutoff.</p><!--".to_owned()
        + &"x".repeat(1_000_000)
        + "--><p>This readable tail is after the default cutoff.</p></main>";
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html")
                .set_body_bytes(body),
        )
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

#[test]
fn overflowing_numeric_arguments_are_usage_errors() {
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
    for flag in [
        "--timeout",
        "--search-budget",
        "--fetch-budget",
        "--cache-ttl",
    ] {
        for value in [
            "0",
            "NaN",
            "inf",
            "-inf",
            "1e-100",
            "1e100",
            "18446744073709551615",
        ] {
            Command::cargo_bin("kestrel")
                .unwrap()
                .args(["search", "test", &format!("{flag}={value}")])
                .assert()
                .code(2)
                .stdout("")
                .stderr(predicate::str::contains("panicked").not());
        }
    }
    for value in ["1e-100", "1e100", "NaN", "inf"] {
        Command::cargo_bin("kestrel")
            .unwrap()
            .args(["fetch", "http://127.0.0.1:9", "--timeout", value])
            .assert()
            .code(2)
            .stdout("");
    }
    for flag in [
        "--search-concurrency",
        "--concurrency",
        "--parse-concurrency",
    ] {
        for value in [0, tokio::sync::Semaphore::MAX_PERMITS + 1, usize::MAX] {
            Command::cargo_bin("kestrel")
                .unwrap()
                .args(["search", "test", flag, &value.to_string()])
                .assert()
                .code(2)
                .stdout("");
        }
    }
    Command::cargo_bin("kestrel")
        .unwrap()
        .args(["search", "test", "--top-k", &usize::MAX.to_string()])
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains("--fetch-candidates"));
}

#[tokio::test]
async fn installed_skill_reading_recipe_handles_evidence_and_capture() {
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

    let server = MockServer::start().await;
    for (route, body) in [
        (
            "/article",
            "<main><p>Ownership keeps Rust memory safe.</p></main>".to_owned(),
        ),
        ("/empty", "<script>nothing readable</script>".to_owned()),
        (
            "/capped",
            format!(
                "<main><p>{}</p></main>",
                "Ownership evidence. ".repeat(120_000)
            ),
        ),
    ] {
        Mock::given(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/html"))
            .expect(1)
            .mount(&server)
            .await;
    }
    let base = server.uri();
    tokio::task::spawn_blocking(move || {
        let project = tempfile::tempdir().unwrap();
        let user_home = tempfile::tempdir().unwrap();
        Command::cargo_bin("kestrel")
            .unwrap()
            .current_dir(project.path())
            .env("HOME", user_home.path())
            .args(["skill", "install", "--agent", "codex", "--scope", "project"])
            .assert()
            .success();
        let skill = fs::read_to_string(project.path().join(".codex/skills/kestrelsearch/SKILL.md"))
            .unwrap();
        let workflow = skill
            .split("## Task workflow:")
            .nth(1)
            .unwrap()
            .split("## `search` subcommand")
            .next()
            .unwrap();
        let recipe = workflow
            .lines()
            .find(|line| line.starts_with("kestrel fetch "))
            .unwrap();
        let recipe = shlex::split(recipe).unwrap();
        let trace = project.path().join("trace");
        let artifacts = project.path().join("artifacts");
        for route in ["/article", "/empty", "/capped"] {
            let mut args = recipe[1..].to_vec();
            args[1] = format!("{base}{route}");
            let output = Command::cargo_bin("kestrel")
                .unwrap()
                .current_dir(project.path())
                .env("HOME", user_home.path())
                .env("KESTRELSEARCH_PROVIDER_TRACE_DIR", &trace)
                .env("KESTRELSEARCH_BENCHMARK_ARTIFACT_DIR", &artifacts)
                .env("KESTRELSEARCH_BENCHMARK_RUN_ID", "lookup")
                .args(args)
                .output()
                .unwrap();
            if route == "/empty" {
                assert!(!output.status.success());
                assert!(output.stdout.is_empty());
                continue;
            }
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert!(json["content"].as_str().unwrap().contains("Ownership"));
            assert!(json["elapsed_seconds"].as_f64().unwrap() >= 0.0);
            if route == "/capped" {
                assert!(String::from_utf8_lossy(&output.stderr).contains("--max-response-bytes"));
            }
        }
        // Direct fetch can capture generated client headers, but not search artifacts.
        assert!(!artifacts.exists());
        let captures: Vec<_> = fs::read_dir(trace)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert!(!captures.is_empty());
        for file in captures {
            let json: serde_json::Value = serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
            assert!(json["headers"].is_object());
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn fetch_plain_text_and_markdown_preserve_body_in_text_and_json() {
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
    let server = MockServer::start().await;
    for (route, content_type, body) in [
        (
            "/source.txt",
            "text/plain; charset=utf-8",
            "  fn main() {\n\tprintln!(\"<p>&amp; 日本 🦀</p>\");\n}\n",
        ),
        (
            "/source.md",
            "Text/Markdown; charset=UTF-8",
            "# Heading\n\n- [link](https://example.org)\n```html\n\t<p>&amp; 日本 🦀</p>\n```\n",
        ),
    ] {
        Mock::given(path(route))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(body)
                    .insert_header("content-type", content_type),
            )
            .expect(2)
            .mount(&server)
            .await;
        let url = format!("{}{route}", server.uri());
        tokio::task::spawn_blocking(move || {
            for format in ["text", "json"] {
                let output = Command::cargo_bin("kestrel")
                    .unwrap()
                    .args(["fetch", &url, "--output", format])
                    .assert()
                    .success()
                    .get_output()
                    .clone();
                let expected = format!("Source: {url}\n\n{body}");
                if format == "json" {
                    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
                    assert_eq!(value["content"], expected);
                    assert_eq!(value["url"], url);
                } else {
                    assert_eq!(
                        String::from_utf8(output.stdout).unwrap(),
                        format!("{expected}\n")
                    );
                }
                completion_seconds(&output.stderr, "Fetch");
            }
        })
        .await
        .unwrap();
    }
}

#[test]
fn fetch_score_invalid_options_fail_before_requests() {
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
    for flags in [
        vec!["--min-fetch-score=-1"],
        vec!["--min-fetch-score=NaN"],
        vec!["--min-fetch-score=inf"],
        vec!["--min-fetch-score=-inf"],
        vec!["--min-fetch-score=1e999"],
        vec!["--min-fetch-score=abc"],
        vec!["--min-fetch-score=1", "--no-fetch"],
        vec!["--min-fetch-score=1", "--query-syntax=native"],
    ] {
        Command::cargo_bin("kestrel")
            .unwrap()
            .args(["search", "rust"])
            .args(flags)
            .assert()
            .code(2)
            .stdout("")
            .stderr(predicate::str::contains("Searching").not());
    }
    Command::cargo_bin("kestrel")
        .unwrap()
        .args(["fetch", "https://example.com", "--min-fetch-score=1"])
        .assert()
        .code(2)
        .stdout("");
}

#[tokio::test]
async fn quality_keeps_shell_fetch_successful_and_recovers_explicit_article() {
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
    let server = MockServer::start().await;
    for (route, html) in [
        (
            "/shell",
            "<main><h1>Your browser is not supported</h1></main>",
        ),
        (
            "/recover",
            "<main><h2>Post a comment</h2><p>Your email address will not be published.</p></main><article><p>The singer joined the group before its second album.</p></article>",
        ),
    ] {
        Mock::given(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_raw(html, "text/html"))
            .mount(&server)
            .await;
    }
    let base = server.uri();
    tokio::task::spawn_blocking(move || {
        let user_home = tempfile::tempdir().unwrap();
        for (route, expected) in [
            ("/shell", "Your browser is not supported"),
            (
                "/recover",
                "The singer joined the group before its second album.",
            ),
        ] {
            for json in [false, true] {
                let mut command = Command::cargo_bin("kestrel").unwrap();
                command
                    .env("HOME", user_home.path())
                    .args(["fetch", &format!("{base}{route}")]);
                if json {
                    command.args(["--output", "json"]);
                }
                let output = command.assert().success().get_output().clone();
                completion_seconds(&output.stderr, "Fetch");
                if json {
                    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
                    assert_eq!(
                        value["content"],
                        format!("Source: {base}{route}\n\n{expected}")
                    );
                    assert!(value.get("content_quality").is_none());
                } else {
                    assert!(String::from_utf8(output.stdout).unwrap().contains(expected));
                }
            }
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn structured_diagnostics_default_opt_out_and_skill_installation() {
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::method("GET"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw("Your browser is not supported.", "text/plain"),
        )
        .expect(2)
        .mount(&server)
        .await;
    let url = server.uri();
    tokio::task::spawn_blocking(move || {
        let project = tempfile::tempdir().unwrap();
        let user_home = tempfile::tempdir().unwrap();
        for opt_out in [false, true] {
            let mut command = Command::cargo_bin("kestrel").unwrap();
            command
                .current_dir(project.path())
                .env("HOME", user_home.path())
                .env_remove("HTTP_PROXY")
                .env_remove("HTTPS_PROXY")
                .env_remove("ALL_PROXY")
                .env_remove("http_proxy")
                .env_remove("https_proxy")
                .env_remove("all_proxy")
                .env("NO_PROXY", "*")
                .env("no_proxy", "*")
                .env_remove("KESTRELSEARCH_PROVIDER_TRACE_DIR")
                .env_remove("KESTRELSEARCH_BENCHMARK_ARTIFACT_DIR")
                .args(["fetch", &url, "--output", "json"]);
            if opt_out {
                command.arg("--no-diagnostics");
            }
            let output = command.assert().success().get_output().clone();
            let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            if opt_out {
                assert_eq!(value.as_object().unwrap().len(), 3);
                assert!(value.get("diagnostics").is_none());
            } else {
                assert_eq!(value["diagnostics"]["schema_version"], 1);
                assert_eq!(value["diagnostics"]["state"], "extracted");
                assert_eq!(value["diagnostics"]["quality"]["state"], "boilerplate_only");
                assert!(value["diagnostics"]["usable"].is_null());
                assert!(!value["diagnostics"].to_string().contains(&url));
            }
        }
        Command::cargo_bin("kestrel")
            .unwrap()
            .current_dir(project.path())
            .env("HOME", user_home.path())
            .args(["skill", "install", "--agent", "codex", "--scope", "project"])
            .assert()
            .success();
        let skill = fs::read_to_string(project.path().join(".codex/skills/kestrelsearch/SKILL.md"))
            .unwrap();
        for command in ["search", "fetch"] {
            Command::cargo_bin("kestrel")
                .unwrap()
                .args([command, "--help"])
                .assert()
                .success()
                .stdout(predicate::str::contains("--no-diagnostics"));
        }
        for contract in [
            "--no-diagnostics",
            "schema_version: 1",
            "returned_index",
            "timing_censored",
            "all_failed",
            "queries_omitted",
        ] {
            assert!(skill.contains(contract), "{contract}");
        }
    })
    .await
    .unwrap();
}

#[test]
fn installed_skill_documents_telemetry_configuration_and_capture_limits() {
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
    let project = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    Command::cargo_bin("kestrel")
        .unwrap()
        .current_dir(project.path())
        .env("HOME", home.path())
        .args(["skill", "install", "--agent", "codex", "--scope", "project"])
        .assert()
        .success();
    let skill =
        std::fs::read_to_string(project.path().join(".codex/skills/kestrelsearch/SKILL.md"))
            .unwrap();
    for text in [
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
        "KESTRELSEARCH_OTEL_CONTENT",
        "8192",
        "sanitized",
        "http/protobuf",
        "test_traces.py",
    ] {
        assert!(skill.contains(text), "missing {text}");
    }
}

#[tokio::test]
async fn ordered_html_fetch_matches_golden_in_text_and_json() {
    let _telemetry = kestrelsearch::telemetry::test_export_guard();
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
    let server = MockServer::start().await;
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/extraction/ordered.json")).unwrap();
    for case in cases.iter().filter(|case| case["expected"].is_string()) {
        Mock::given(path(format!("/{}", case["id"].as_str().unwrap())))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(case["html"].as_str().unwrap(), "text/html"),
            )
            .expect(2)
            .mount(&server)
            .await;
    }
    let base = server.uri();
    tokio::task::spawn_blocking(move || {
        let user_home = tempfile::tempdir().unwrap();
        for case in cases.iter().filter(|case| case["expected"].is_string()) {
            let url = format!("{}/{}", base, case["id"].as_str().unwrap());
            let expected = format!("Source: {url}\n\n{}", case["expected"].as_str().unwrap());
            for format in ["text", "json"] {
                let output = Command::cargo_bin("kestrel")
                    .unwrap()
                    .env("HOME", user_home.path())
                    .args(["fetch", &url, "--output", format])
                    .assert()
                    .success()
                    .get_output()
                    .clone();
                if format == "json" {
                    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
                    assert_eq!(value["content"], expected);
                    assert_eq!(value.as_object().unwrap().len(), 4);
                    assert_eq!(value["diagnostics"]["schema_version"], 1);
                } else {
                    assert_eq!(
                        String::from_utf8(output.stdout).unwrap(),
                        format!("{expected}\n")
                    );
                }
            }
        }
    })
    .await
    .unwrap();
}

#[cfg(feature = "test-fixtures")]
#[tokio::test(flavor = "multi_thread")]
async fn default_hybrid_matches_explicit_policy_with_and_without_fetching() {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    let server = MockServer::start().await;
    let base = server.uri();
    Mock::given(method("GET")).and(path("/bing"))
        .respond_with(ResponseTemplate::new(200).set_body_string(format!(
            r#"<ol id="b_results"><li class="b_algo"><h2><a href="{base}/generic">Python home</a></h2><div class="b_caption"><p>Python downloads</p></div></li><li class="b_algo"><h2><a href="{base}/guide">uv guide</a></h2><div class="b_caption"><p>uv package management guide</p></div></li></ol>"#)))
        .mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/generic"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/plain")
                .set_body_string("uv guide"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/guide"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let mut fetch_requests = 0;
    for no_fetch in [true, false] {
        let mut outputs = Vec::new();
        let controls: &[&[&str]] = if no_fetch {
            &[
                &[],
                &["--ranking-policy", "hybrid"],
                &["--no-rank"],
                &["--ranking-policy", "provider"],
            ]
        } else {
            &[
                &[],
                &["--ranking-policy", "hybrid"],
                &["--no-rank"],
                &["--ranking-policy", "provider"],
                &["--rank"],
                &["--ranking-policy", "body"],
            ]
        };
        for control in controls {
            let mut command = Command::cargo_bin("kestrel").unwrap();
            command
                .env("KESTREL_TEST_PROVIDER_ENDPOINT", format!("{base}/bing"))
                .args([
                    "search",
                    "uv guide",
                    "-e",
                    "bing",
                    "--search-budget",
                    "15",
                    "--output",
                    "json",
                ])
                .args(*control);
            if no_fetch {
                command.arg("--no-fetch");
            }
            let output = command.assert().success().get_output().stdout.clone();
            let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
            outputs.push(value["results"].clone());
            if !no_fetch {
                fetch_requests += 2;
            }
        }
        assert_eq!(outputs[0], outputs[1]);
        assert_eq!(outputs[0][0]["url"], format!("{base}/guide"));
        assert_eq!(outputs[0].as_array().unwrap().len(), 2);
        assert!(outputs[0][0]["content"].is_null());
        assert!(outputs[0][0]["bm25_score"].is_null());
        assert_eq!(outputs[2], outputs[3]);
        assert_eq!(outputs[2][0]["url"], format!("{base}/generic"));
        if !no_fetch {
            assert_eq!(outputs[0], outputs[4]);
            assert_eq!(outputs[5].as_array().unwrap().len(), 1);
            assert_eq!(outputs[5][0]["url"], format!("{base}/generic"));
            assert!(outputs[5][0]["bm25_score"].as_f64().unwrap() > 0.0);
        }
        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests.iter().filter(|r| r.url.path() != "/bing").count(),
            fetch_requests
        );
    }
}
