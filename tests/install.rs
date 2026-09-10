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
fn refuses_existing_files_and_symlinks_without_modifying_them() {
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
            .failure()
            .stderr(predicate::str::contains("nothing was replaced"));
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
        .failure();
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
