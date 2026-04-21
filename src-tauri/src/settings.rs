use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::errors::{AppError, AppResult};
use crate::types::Settings;

fn atomic_write(path: &Path, bytes: &[u8]) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new(app_data_dir: PathBuf) -> Self {
        Self { path: app_data_dir.join("settings.json") }
    }

    pub fn load(&self) -> Settings {
        match fs::read_to_string(&self.path) {
            Ok(s) => serde_json::from_str::<Settings>(&s).unwrap_or_default(),
            Err(_) => Settings::default(),
        }
    }

    pub fn save(&self, settings: &Settings) -> AppResult<()> {
        let bytes = serde_json::to_vec_pretty(settings)?;
        atomic_write(&self.path, &bytes).map_err(|e| AppError::Other(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn load_returns_default_when_missing() {
        let tmp = TempDir::new().unwrap();
        let store = SettingsStore::new(tmp.path().to_path_buf());
        let s = store.load();
        assert_eq!(s.refresh_interval_sec, 300);
        assert!(s.always_on_top);
    }

    #[test]
    fn save_then_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let store = SettingsStore::new(tmp.path().to_path_buf());
        let mut s = Settings::default();
        s.opacity = 0.7;
        s.refresh_interval_sec = 60;
        store.save(&s).unwrap();
        let loaded = store.load();
        assert!((loaded.opacity - 0.7).abs() < 1e-9);
        assert_eq!(loaded.refresh_interval_sec, 60);
    }

    #[test]
    fn load_recovers_from_corrupt_json() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("settings.json");
        fs::write(&path, "not json").unwrap();
        let store = SettingsStore::new(tmp.path().to_path_buf());
        let s = store.load();
        assert_eq!(s.refresh_interval_sec, 300);
    }
}
