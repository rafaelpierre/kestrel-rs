//! Skill-installation state stored in `~/.kestrelsearch/config.toml`.

use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use thiserror::Error;
use toml_edit::{Array, DocumentMut, Item, Table, Value};

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("home directory is unavailable")]
    MissingHome,
    #[error("failed to read or write configuration: {0}")]
    Io(#[from] io::Error),
    #[error("invalid configuration TOML: {0}")]
    Toml(#[from] toml_edit::TomlError),
}

pub fn config_path() -> Result<PathBuf, ConfigError> {
    home::home_dir()
        .map(|home| home.join(".kestrelsearch").join("config.toml"))
        .ok_or(ConfigError::MissingHome)
}

pub fn get_installations() -> Result<Vec<PathBuf>, ConfigError> {
    ConfigStore::new(config_path()?).get_installations()
}

pub fn record_installation(path: &Path) -> Result<(), ConfigError> {
    ConfigStore::new(config_path()?).record_installation(path)
}

pub fn remove_installation(path: &Path) -> Result<(), ConfigError> {
    ConfigStore::new(config_path()?).remove_installation(path)
}

struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn load(&self) -> Result<DocumentMut, ConfigError> {
        match fs::read_to_string(&self.path) {
            Ok(contents) => return Ok(contents.parse()?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let mut document = DocumentMut::new();
        let mut skill = Table::new();
        skill["installations"] = Item::Value(Value::Array(Array::new()));
        document["skill"] = Item::Table(skill);
        Ok(document)
    }

    // Resolve existing symlinks so replacement updates their destination. Canonicalize
    // the parent for a new file so directory aliases share the same sidecar lock.
    // Dangling symlinks are rejected rather than silently replaced.
    fn transaction_store(&self) -> Result<Self, ConfigError> {
        let path = match self.path.canonicalize() {
            Ok(path) => path,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if fs::symlink_metadata(&self.path).is_ok_and(|meta| meta.is_symlink()) {
                    return Err(error.into());
                }
                let parent = self
                    .path
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."));
                fs::create_dir_all(parent)?;
                let name = self.path.file_name().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "config path needs a file name")
                })?;
                parent.canonicalize()?.join(name)
            }
            Err(error) => return Err(error.into()),
        };
        Ok(Self::new(path))
    }

    fn lock(&self, timeout: Duration) -> Result<fs::File, ConfigError> {
        // Never remove this file: waiters must continue to lock the same inode
        // even after the config itself is atomically replaced.
        let mut lock_path = self.path.as_os_str().to_owned();
        lock_path.push(".lock");
        let path = PathBuf::from(lock_path);
        let lock = fs::File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        let start = Instant::now();
        loop {
            match lock.try_lock() {
                Ok(()) => return Ok(lock),
                Err(fs::TryLockError::WouldBlock) if start.elapsed() < timeout => {
                    std::thread::sleep(
                        Duration::from_millis(10).min(timeout.saturating_sub(start.elapsed())),
                    );
                }
                Err(fs::TryLockError::WouldBlock) => {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, format!(
                        "timed out waiting for configuration lock {}; retry after the other installation finishes",
                        path.display()
                    )).into());
                }
                Err(fs::TryLockError::Error(error)) => return Err(error.into()),
            }
        }
    }

    fn update(
        &self,
        edit: impl FnOnce(&mut DocumentMut) -> Result<bool, ConfigError>,
    ) -> Result<(), ConfigError> {
        let store = self.transaction_store()?;
        let _lock = store.lock(Duration::from_secs(10))?;
        let mut document = store.load()?;
        if edit(&mut document)? {
            store.save(&document)?;
        }
        Ok(())
    }

    fn save(&self, document: &DocumentMut) -> Result<(), ConfigError> {
        self.save_before_replace(document, |_| Ok(()))
    }

    // The callback is an internal fault-injection seam, not an environment switch.
    fn save_before_replace(
        &self,
        document: &DocumentMut,
        before_replace: impl FnOnce(&Path) -> io::Result<()>,
    ) -> Result<(), ConfigError> {
        let parent = self.path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "config path needs a parent")
        })?;
        let mut staged = tempfile::NamedTempFile::new_in(parent)?;
        staged.write_all(document.to_string().as_bytes())?;
        match fs::metadata(&self.path) {
            Ok(metadata) => staged.as_file().set_permissions(metadata.permissions())?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        staged.as_file().sync_all()?;
        before_replace(staged.path())?;
        staged.persist(&self.path).map_err(|error| error.error)?;
        // A failure here is reported after commit: the new contents are visible,
        // but persistence of the directory entry across power loss is uncertain.
        #[cfg(unix)]
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!(
                        "configuration replaced, but failed to sync directory {}: {error}",
                        parent.display()
                    ),
                )
            })?;
        Ok(())
    }

    fn get_installations(&self) -> Result<Vec<PathBuf>, ConfigError> {
        let document = self.load()?;
        Ok(document
            .get("skill")
            .and_then(Item::as_table)
            .and_then(|skill| skill.get("installations"))
            .and_then(Item::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(PathBuf::from)
            .collect())
    }

    fn record_installation(&self, path: &Path) -> Result<(), ConfigError> {
        self.update(|document| {
            ensure_installations(document);
            let absolute = absolute_path(path)?;
            let rendered = absolute.to_string_lossy().into_owned();
            let array = document["skill"]["installations"]
                .as_array_mut()
                .expect("installation array initialized");
            let exists = array
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|value| absolute_path(Path::new(value)).ok())
                .any(|value| value == absolute);
            if !exists {
                array.push(rendered);
            }
            Ok(true)
        })
    }

    fn remove_installation(&self, path: &Path) -> Result<(), ConfigError> {
        self.update(|document| {
            let Some(array) = document
                .get_mut("skill")
                .and_then(Item::as_table_mut)
                .and_then(|skill| skill.get_mut("installations"))
                .and_then(Item::as_array_mut)
            else {
                return Ok(false);
            };
            let target = absolute_path(path)?;
            let retained: Vec<String> = array
                .iter()
                .filter_map(Value::as_str)
                .filter(|value| absolute_path(Path::new(value)).map_or(true, |path| path != target))
                .map(str::to_owned)
                .collect();
            array.clear();
            for value in retained {
                array.push(value);
            }
            Ok(true)
        })
    }
}

fn ensure_installations(document: &mut DocumentMut) {
    if !document.contains_key("skill") || !document["skill"].is_table() {
        document["skill"] = Item::Table(Table::new());
    }
    if !document["skill"]
        .as_table()
        .is_some_and(|skill| skill.get("installations").is_some_and(Item::is_array))
    {
        document["skill"]["installations"] = Item::Value(Value::Array(Array::new()));
    }
}

fn absolute_path(path: &Path) -> Result<PathBuf, io::Error> {
    if let Ok(canonical) = path.canonicalize() {
        return Ok(canonical);
    }
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests;
