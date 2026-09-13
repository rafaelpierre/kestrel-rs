//! Trace an external test, suite or benchmark without changing its stdout.
use kestrelsearch::telemetry::{self, Span};
use std::{
    process::{Command, ExitCode},
    time::Instant,
};
fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let Some(name) = args.next() else {
        eprintln!("usage: trace_command NAME COMMAND [ARGS...] (COMMAND '-' records a skip)");
        return ExitCode::from(2);
    };
    let Some(program) = args.next() else {
        return ExitCode::from(2);
    };
    let enabled = match telemetry::init_from_env() {
        Ok(v) => v,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    let mut span = Span::new(if name == "test.run" {
        "kestrel.test_run"
    } else if name.to_string_lossy().starts_with("suite:") {
        "kestrel.test_suite"
    } else {
        "kestrel.test"
    });
    span.attribute("test.name", name.to_string_lossy().into_owned());
    let started = Instant::now();
    let receipts = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(_) => return ExitCode::FAILURE,
    };
    let outcome_file = receipts.path().join("test-outcome.json");
    let mut command = Command::new(&program);
    command.env("KESTRELSEARCH_OTEL_TEST_OUTCOME", &outcome_file);
    command.args(args);
    span.inject(&mut command);
    command.env("KESTRELSEARCH_OTEL_TEST_ID", &name);
    command.env("KESTRELSEARCH_OTEL_RECEIPT_DIR", receipts.path());
    let mut skipped = program == "-";
    let status = if skipped {
        None
    } else {
        Some(command.status())
    };
    let passed = status
        .as_ref()
        .is_none_or(|s| s.as_ref().is_ok_and(|s| s.success()));
    if let Ok(bytes) = std::fs::read(&outcome_file) {
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            skipped = value["skipped"].as_u64().is_some_and(|n| n > 0) && passed;
        }
        let _ = std::fs::remove_file(&outcome_file);
    }
    span.attribute(
        "test.status",
        if skipped {
            "skip"
        } else if passed {
            "pass"
        } else {
            "fail"
        },
    );
    span.attribute("test.duration_seconds", started.elapsed().as_secs_f64());
    span.finish();
    drop(span);
    let delivery = telemetry::shutdown();
    let children_delivered = std::fs::read_dir(receipts.path()).is_ok_and(|entries| {
        entries
            .flatten()
            .all(|entry| std::fs::read_to_string(entry.path()).is_ok_and(|s| s == "ok"))
    });
    if enabled && (!delivery || !children_delivered) {
        eprintln!(
            "[kestrel] test telemetry delivery incomplete (functional result: {})",
            if passed { "pass" } else { "fail" }
        );
        return ExitCode::from(3);
    }
    match status {
        None => ExitCode::SUCCESS,
        Some(Ok(s)) => ExitCode::from(s.code().unwrap_or(1).clamp(0, 255) as u8),
        Some(Err(_)) => ExitCode::FAILURE,
    }
}
