#![cfg(unix)]

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};

use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn installs_runnable_copy_and_repeated_self_install_is_a_noop() {
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("nested/bin");
    Command::cargo_bin("kestrel")
        .unwrap()
        .arg("install")
        .arg("--dir")
        .arg(&directory)
        .env("PATH", "")
        .assert()
        .success()
        .stdout(predicate::str::contains("Installed:"))
        .stdout(predicate::str::contains("export PATH="));
    let installed = directory.join("kestrel");
    assert_eq!(
        fs::metadata(&installed).unwrap().permissions().mode() & 0o777,
        fs::metadata(assert_cmd::cargo::cargo_bin("kestrel"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    );
    Command::new(&installed).arg("--version").assert().success();
    Command::new(&installed)
        .arg("install")
        .arg("--dir")
        .arg(&directory)
        .env("PATH", &directory)
        .assert()
        .success()
        .stdout(predicate::str::contains("Already installed:"))
        .stdout(predicate::str::contains("export PATH=").not());
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
}

#[test]
fn default_install_uses_home_and_reports_path_conflicts() {
    let temporary = tempfile::tempdir().unwrap();
    let competing = temporary.path().join("other-bin");
    fs::create_dir(&competing).unwrap();
    let other_binary = competing.join("kestrel");
    fs::write(&other_binary, "other installation").unwrap();
    fs::set_permissions(&other_binary, fs::Permissions::from_mode(0o755)).unwrap();
    Command::cargo_bin("kestrel")
        .unwrap()
        .arg("install")
        .env("HOME", temporary.path())
        .env("PATH", &competing)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "PATH currently selects another installation",
        ));
    assert!(temporary.path().join(".local/bin/kestrel").is_file());
    assert_eq!(
        fs::read_to_string(other_binary).unwrap(),
        "other installation"
    );
}

#[test]
fn preserves_existing_files_and_symlinks_without_confirmation() {
    let temporary = tempfile::tempdir().unwrap();
    let destination = temporary.path().join("kestrel");
    let target = temporary.path().join("managed-binary");
    fs::write(&target, "managed installation").unwrap();
    for link in [false, true] {
        if link {
            symlink(&target, &destination).unwrap();
        } else {
            fs::write(&destination, "existing installation").unwrap();
        }
        Command::cargo_bin("kestrel")
            .unwrap()
            .arg("install")
            .arg("--dir")
            .arg(temporary.path())
            .assert()
            .success()
            .stderr(predicate::str::contains("Replace it? [y/N]"))
            .stdout(predicate::str::contains("nothing was replaced"));
        assert_eq!(
            fs::symlink_metadata(&destination).unwrap().is_symlink(),
            link
        );
        assert_eq!(
            fs::read_to_string(&destination).unwrap(),
            if link {
                "managed installation"
            } else {
                "existing installation"
            }
        );
        fs::remove_file(&destination).unwrap();
    }
    // A dangling package-manager symlink must also be preserved.
    fs::remove_file(&target).unwrap();
    symlink(&target, &destination).unwrap();
    Command::cargo_bin("kestrel")
        .unwrap()
        .arg("install")
        .arg("--dir")
        .arg(temporary.path())
        .assert()
        .success();
    assert_eq!(fs::read_link(destination).unwrap(), target);
}

#[test]
fn rejects_conflicting_scopes_and_empty_directory() {
    Command::cargo_bin("kestrel")
        .unwrap()
        .args(["install", "--system", "--dir", "/unused"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
    Command::cargo_bin("kestrel")
        .unwrap()
        .args(["install", "--dir", ""])
        .assert()
        .failure()
        .stderr(predicate::str::contains("a value is required"));
}

#[test]
fn prompts_and_only_replaces_when_confirmed() {
    for answer in ["n\n", "\n", "maybe\n", "y\n", " YES \n"] {
        for link in [false, true] {
            let temporary = tempfile::tempdir().unwrap();
            let destination = temporary.path().join("kestrel");
            let target = temporary.path().join("managed-binary");
            fs::write(&target, "old installation").unwrap();
            if link {
                symlink(&target, &destination).unwrap();
            } else {
                fs::write(&destination, "old installation").unwrap();
            }
            let confirmed = matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes");
            Command::cargo_bin("kestrel")
                .unwrap()
                .arg("install")
                .arg("--dir")
                .arg(temporary.path())
                .write_stdin(answer)
                .assert()
                .success()
                .stderr(predicate::str::contains("Replace it? [y/N]"))
                .stdout(predicate::str::contains(if confirmed {
                    "Installed:"
                } else {
                    "Installation cancelled"
                }));
            assert_eq!(fs::read_to_string(&target).unwrap(), "old installation");
            if confirmed {
                assert!(!fs::symlink_metadata(&destination).unwrap().is_symlink());
                Command::new(&destination)
                    .arg("--version")
                    .assert()
                    .success();
            } else {
                assert_eq!(
                    fs::read_to_string(&destination).unwrap(),
                    "old installation"
                );
                assert_eq!(
                    fs::symlink_metadata(&destination).unwrap().is_symlink(),
                    link
                );
            }
            assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), 2);
        }
    }
}

#[test]
fn replaces_dangling_symlink_after_confirmation() {
    let temporary = tempfile::tempdir().unwrap();
    let destination = temporary.path().join("kestrel");
    let target = temporary.path().join("missing");
    symlink(&target, &destination).unwrap();
    Command::cargo_bin("kestrel")
        .unwrap()
        .arg("install")
        .arg("--dir")
        .arg(temporary.path())
        .write_stdin("y\n")
        .assert()
        .success();
    assert!(!target.exists());
    Command::new(&destination)
        .arg("--version")
        .assert()
        .success();
}
