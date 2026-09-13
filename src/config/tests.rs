use super::*;

// Concurrent process-spawning tests can briefly inherit a locked descriptor
// between fork and exec on Unix. Closing our handle alone need not release the
// lock until that child execs. Successful acquisitions use the production budget;
// reserve Duration::ZERO for assertions that contention must fail immediately.
const TEST_LOCK_TIMEOUT: Duration = Duration::from_secs(10);

#[test]
fn records_deduplicates_and_removes() {
    let _telemetry = crate::telemetry::test_export_guard();
    let directory = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(directory.path().join("config.toml"));
    let skill = directory.path().join("skills/SKILL.md");
    fs::create_dir_all(skill.parent().unwrap()).unwrap();
    fs::write(&skill, "skill").unwrap();
    store.record_installation(&skill).unwrap();
    store.record_installation(&skill).unwrap();
    assert_eq!(
        store.get_installations().unwrap(),
        [skill.canonicalize().unwrap()]
    );
    store.remove_installation(&skill).unwrap();
    assert!(store.get_installations().unwrap().is_empty());
}

#[test]
fn missing_skill_table_is_preserved_on_remove() {
    let _telemetry = crate::telemetry::test_export_guard();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    fs::write(&path, "title = 'empty'\n").unwrap();
    ConfigStore::new(path.clone())
        .remove_installation(Path::new("missing"))
        .unwrap();
    assert_eq!(fs::read_to_string(path).unwrap(), "title = 'empty'\n");
}

// Run the actual private store in independent processes without changing HOME or
// exposing a test-only CLI/environment interface in the production binary.
#[test]
fn process_worker() {
    let _telemetry = crate::telemetry::test_export_guard();
    let Some(root) = std::env::var_os("KESTREL_CONFIG_TEST_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let store = ConfigStore::new(root.join("config.toml"));
    let mode = std::env::var("KESTREL_CONFIG_TEST_MODE").unwrap();
    if mode == "hold" {
        store
            .update(|_| {
                fs::write(root.join("held"), "ready")?;
                wait_for(&root.join("release"));
                Ok(false)
            })
            .unwrap();
        return;
    }
    let id = std::env::var("KESTREL_CONFIG_TEST_ID").unwrap();
    fs::write(root.join(format!("ready-{id}")), "ready").unwrap();
    wait_for(&root.join("start"));
    for index in 0..8 {
        if mode == "mixed" {
            store
                .remove_installation(&root.join(format!("old-{id}-{index}")))
                .unwrap();
        }
        store
            .record_installation(&root.join(format!("new-{id}-{index}")))
            .unwrap();
    }
}

fn wait_for(path: &Path) {
    let start = Instant::now();
    while !path.exists() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "timed out waiting for {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

struct Worker(std::process::Child);

impl Worker {
    fn spawn(root: &Path, mode: &str, id: usize) -> Self {
        Self(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "config::tests::process_worker", "--nocapture"])
                .env("KESTREL_CONFIG_TEST_ROOT", root)
                .env("KESTREL_CONFIG_TEST_MODE", mode)
                .env("KESTREL_CONFIG_TEST_ID", id.to_string())
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        )
    }

    fn finish(mut self) {
        let start = Instant::now();
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                assert!(status.success(), "worker failed: {status}");
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "worker timed out"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn independent_processes_retain_additions_and_removals() {
    let _telemetry = crate::telemetry::test_export_guard();
    for mode in ["new", "mixed"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let store = ConfigStore::new(root.join("config.toml"));
        if mode == "mixed" {
            fs::write(&store.path, "# keep this comment\ntitle = 'keep me'\n[other]\nenabled = true\n[skill]\ncustom = 'also keep'\n").unwrap();
            for id in 0..6 {
                for index in 0..8 {
                    store
                        .record_installation(&root.join(format!("old-{id}-{index}")))
                        .unwrap();
                }
            }
        }
        let workers: Vec<_> = (0..6).map(|id| Worker::spawn(&root, mode, id)).collect();
        for id in 0..6 {
            wait_for(&root.join(format!("ready-{id}")));
        }
        fs::write(root.join("start"), "go").unwrap();
        for worker in workers {
            worker.finish();
        }
        let actual: std::collections::BTreeSet<_> =
            store.get_installations().unwrap().into_iter().collect();
        let expected = (0..6)
            .flat_map(|id| (0..8).map(move |index| format!("new-{id}-{index}")))
            .map(|name| root.join(name))
            .collect();
        assert_eq!(actual, expected);
        if mode == "mixed" {
            let text = fs::read_to_string(&store.path).unwrap();
            let document: DocumentMut = text.parse().unwrap();
            assert!(text.contains("# keep this comment"));
            assert_eq!(document["title"].as_str(), Some("keep me"));
            assert_eq!(document["other"]["enabled"].as_bool(), Some(true));
            assert_eq!(document["skill"]["custom"].as_str(), Some("also keep"));
        }
    }
}

#[test]
fn transaction_lock_times_out_and_is_released_when_process_exits() {
    let _telemetry = crate::telemetry::test_export_guard();
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let store = ConfigStore::new(root.join("config.toml"));
    let mut worker = Worker::spawn(&root, "hold", 0);
    wait_for(&root.join("held"));
    let start = Instant::now();
    let error = store.lock(Duration::from_millis(30)).unwrap_err();
    assert!(matches!(error, ConfigError::Io(ref error) if error.kind() == io::ErrorKind::TimedOut));
    assert!(start.elapsed() >= Duration::from_millis(30));
    assert!(
        error
            .to_string()
            .contains("retry after the other installation finishes")
    );
    worker.0.kill().unwrap();
    worker.0.wait().unwrap();
    store.record_installation(&root.join("after-exit")).unwrap();
    assert_eq!(
        store.get_installations().unwrap(),
        [root.join("after-exit")]
    );
    assert!(root.join("config.toml.lock").exists());
}

#[test]
fn failures_before_replacement_preserve_old_bytes_and_clean_staging() {
    let _telemetry = crate::telemetry::test_export_guard();
    let directory = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(directory.path().join("config.toml"));
    let original = "# precious\ntitle = 'original'\n";
    fs::write(&store.path, original).unwrap();
    let replacement = "title = 'replacement'\n".parse().unwrap();
    for fail_persist in [false, true] {
        let _lock = store.lock(TEST_LOCK_TIMEOUT).unwrap();
        let error = store
            .save_before_replace(&replacement, |staged| {
                assert_eq!(fs::read_to_string(&store.path)?, original);
                assert_eq!(fs::read_to_string(staged)?, "title = 'replacement'\n");
                if fail_persist {
                    fs::remove_file(staged)?;
                    Ok(()) // Force the real persist operation to fail.
                } else {
                    Err(io::Error::other("injected failure before replacement"))
                }
            })
            .unwrap_err();
        assert!(matches!(error, ConfigError::Io(_)));
        assert_eq!(fs::read_to_string(&store.path).unwrap(), original);
        store.load().unwrap();
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
    }
}

#[test]
fn invalid_toml_is_untouched_and_errors_release_lock() {
    let _telemetry = crate::telemetry::test_export_guard();
    let directory = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(directory.path().join("config.toml"));
    let original = "[invalid TOML";
    fs::write(&store.path, original).unwrap();
    assert!(matches!(
        store.record_installation(Path::new("new")),
        Err(ConfigError::Toml(_))
    ));
    assert!(matches!(
        store.remove_installation(Path::new("old")),
        Err(ConfigError::Toml(_))
    ));
    assert_eq!(fs::read_to_string(&store.path).unwrap(), original);
    let _lock = store.lock(TEST_LOCK_TIMEOUT).unwrap();
}

#[cfg(unix)]
#[test]
fn preserves_permissions_and_symlink_destination() {
    let _telemetry = crate::telemetry::test_export_guard();
    use std::os::unix::fs::{PermissionsExt, symlink};
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("real.toml");
    fs::write(&target, "title = 'keep'\n").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
    let link = directory.path().join("config.toml");
    symlink(&target, &link).unwrap();
    let store = ConfigStore::new(link.clone());
    let resolved = store.transaction_store().unwrap();
    let lock = resolved.lock(TEST_LOCK_TIMEOUT).unwrap();
    assert!(
        matches!(ConfigStore::new(target.canonicalize().unwrap()).lock(Duration::ZERO), Err(ConfigError::Io(ref e)) if e.kind() == io::ErrorKind::TimedOut)
    );
    drop(lock);
    store
        .record_installation(&directory.path().join("skill"))
        .unwrap();
    assert!(fs::symlink_metadata(link).unwrap().is_symlink());
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert_eq!(store.get_installations().unwrap().len(), 1);
    assert!(
        fs::read_to_string(target)
            .unwrap()
            .contains("title = 'keep'")
    );
}

#[cfg(unix)]
#[test]
fn dangling_symlink_is_not_replaced() {
    let _telemetry = crate::telemetry::test_export_guard();
    let directory = tempfile::tempdir().unwrap();
    let link = directory.path().join("config.toml");
    let target = directory.path().join("missing.toml");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let store = ConfigStore::new(link.clone());
    assert!(store.record_installation(Path::new("skill")).is_err());
    assert!(fs::symlink_metadata(link).unwrap().is_symlink());
    assert!(!target.exists());
}

#[test]
fn waiting_writer_succeeds_after_other_process_releases_lock() {
    let _telemetry = crate::telemetry::test_export_guard();
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let store = ConfigStore::new(root.join("config.toml"));
    let worker = Worker::spawn(&root, "hold", 0);
    wait_for(&root.join("held"));
    assert!(
        matches!(store.lock(Duration::ZERO), Err(ConfigError::Io(ref e)) if e.kind() == io::ErrorKind::TimedOut)
    );
    let release = root.join("release");
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        fs::write(release, "go").unwrap();
    });
    store
        .record_installation(&root.join("after-release"))
        .unwrap();
    releaser.join().unwrap();
    worker.finish();
    assert_eq!(
        store.get_installations().unwrap(),
        [root.join("after-release")]
    );
}
