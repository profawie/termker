//! Persists `Data` as JSON in the per-user config folder.
//!
//! This file never contains passwords or private keys.

use std::{fs, io, path::PathBuf};

use crate::model::Data;

pub struct Store {
    path: Option<PathBuf>,
}

impl Store {
    /// Store at the OS default location, e.g. `%APPDATA%\termker\config`
    /// on Windows and `~/Library/Application Support/termker` on macOS.
    pub fn open_default() -> Self {
        let path = directories::ProjectDirs::from("", "", crate::APP_NAME)
            .map(|dirs| dirs.config_dir().join("data.json"));
        Self { path }
    }

    #[cfg(test)]
    pub fn at(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    pub fn path(&self) -> Option<&PathBuf> {
        self.path.as_ref()
    }

    /// Load the data. A file that does not exist yet yields empty data.
    pub fn load(&self) -> io::Result<Data> {
        let Some(path) = &self.path else {
            return Ok(Data::default());
        };
        match fs::read_to_string(path) {
            Ok(text) => serde_json::from_str::<Data>(&text)
                .map(|mut data| {
                    data.sync_groups();
                    data
                })
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Data::default()),
            Err(err) => Err(err),
        }
    }

    /// Write to a temporary file first, then rename, so the existing file
    /// is not damaged if the write is interrupted.
    pub fn save(&self, data: &Data) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(data).map_err(io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, text)?;
        fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Host;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("termker-test-{}-{name}", uuid::Uuid::new_v4()))
            .join("data.json")
    }

    #[test]
    fn missing_file_loads_as_empty() {
        let store = Store::at(temp_path("missing"));
        assert_eq!(store.load().unwrap(), Data::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let store = Store::at(temp_path("roundtrip"));
        let mut data = Data::default();
        data.upsert_host(Host {
            label: "web".into(),
            address: "10.0.0.5".into(),
            ..Host::new()
        })
        .unwrap();
        data.settings.terminal_theme = "Amber".into();
        store.save(&data).unwrap();
        assert_eq!(store.load().unwrap(), data);
        fs::remove_dir_all(store.path().unwrap().parent().unwrap()).unwrap();
    }

    #[test]
    fn corrupt_file_is_an_error_not_silent_data_loss() {
        let store = Store::at(temp_path("corrupt"));
        fs::create_dir_all(store.path().unwrap().parent().unwrap()).unwrap();
        fs::write(store.path().unwrap(), "{ not json").unwrap();
        assert!(store.load().is_err());
        fs::remove_dir_all(store.path().unwrap().parent().unwrap()).unwrap();
    }
}
