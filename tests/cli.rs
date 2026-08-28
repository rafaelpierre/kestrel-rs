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
        .stdout(predicate::str::contains("skill"));
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
