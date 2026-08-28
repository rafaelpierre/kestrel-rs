//! Skill-installation state stored in `~/.kestrelsearch/config.toml`.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

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
        if self.path.exists() {
            return Ok(fs::read_to_string(&self.path)?.parse()?);
        }
        let mut document = DocumentMut::new();
        let mut skill = Table::new();
        skill["installations"] = Item::Value(Value::Array(Array::new()));
        document["skill"] = Item::Table(skill);
        Ok(document)
    }

    fn save(&self, document: &DocumentMut) -> Result<(), ConfigError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.path, document.to_string())?;
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
        let mut document = self.load()?;
        ensure_installations(&mut document);
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
        self.save(&document)
    }

    fn remove_installation(&self, path: &Path) -> Result<(), ConfigError> {
        let mut document = self.load()?;
        let Some(array) = document
            .get_mut("skill")
            .and_then(Item::as_table_mut)
            .and_then(|skill| skill.get_mut("installations"))
            .and_then(Item::as_array_mut)
        else {
            return Ok(());
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
        self.save(&document)
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
mod tests {
    use super::*;

    #[test]
    fn records_deduplicates_and_removes() {
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
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(&path, "title = 'empty'\n").unwrap();
        ConfigStore::new(path.clone())
            .remove_installation(Path::new("missing"))
            .unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "title = 'empty'\n");
    }
}
