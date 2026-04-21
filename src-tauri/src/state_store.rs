use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::errors::AppResult;
use crate::types::{Provider, UsageResponse};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersistedState {
    #[serde(default)]
    pub last_utilization: HashMap<String, f64>,
    #[serde(default)]
    pub last_updated_at: Option<String>,
}

fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
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

pub struct StateStore {
    path: PathBuf,
}

impl StateStore {
    pub fn new(app_data_dir: PathBuf) -> Self {
        Self { path: app_data_dir.join("state.json") }
    }

    pub fn load(&self) -> PersistedState {
        match fs::read_to_string(&self.path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => PersistedState::default(),
        }
    }

    pub fn save(&self, state: &PersistedState) -> AppResult<()> {
        let bytes = serde_json::to_vec_pretty(state)?;
        atomic_write(&self.path, &bytes)?;
        Ok(())
    }

    pub fn compute_and_update(
        &self,
        prev: &PersistedState,
        current: &[UsageResponse],
    ) -> (PersistedState, HashMap<String, f64>) {
        let mut new_util: HashMap<String, f64> = prev.last_utilization.clone();
        let mut delta: HashMap<String, f64> = HashMap::new();

        for resp in current {
            for w in &resp.windows {
                let key = format!("{}.{}", resp.provider.as_str(), w.key);
                let prev_v = prev.last_utilization.get(&key).copied().unwrap_or(w.utilization);
                delta.insert(key.clone(), w.utilization - prev_v);
                new_util.insert(key, w.utilization);
            }
        }

        let new_state = PersistedState {
            last_utilization: new_util,
            last_updated_at: Some(Utc::now().to_rfc3339()),
        };
        (new_state, delta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Status, UsageWindow};
    use tempfile::TempDir;

    fn mk_resp(provider: Provider, windows: Vec<(&str, f64)>) -> UsageResponse {
        UsageResponse {
            provider,
            status: Status::Ok,
            windows: windows.into_iter().map(|(k, u)| UsageWindow {
                key: k.to_string(),
                name: k.to_string(),
                utilization: u,
                resets_at: "2026-04-22T00:00:00Z".to_string(),
                time_progress: 50.0,
            }).collect(),
            extra_usage: None,
            error: None,
        }
    }

    #[test]
    fn delta_zero_when_no_prior_state() {
        let tmp = TempDir::new().unwrap();
        let store = StateStore::new(tmp.path().to_path_buf());
        let prev = PersistedState::default();
        let current = vec![mk_resp(Provider::Claude, vec![("five_hour", 42.0)])];
        let (_new, delta) = store.compute_and_update(&prev, &current);
        assert_eq!(delta.get("claude.five_hour").copied(), Some(0.0));
    }

    #[test]
    fn delta_positive_when_usage_grew() {
        let tmp = TempDir::new().unwrap();
        let store = StateStore::new(tmp.path().to_path_buf());
        let mut prev = PersistedState::default();
        prev.last_utilization.insert("claude.five_hour".into(), 40.0);
        let current = vec![mk_resp(Provider::Claude, vec![("five_hour", 55.0)])];
        let (new, delta) = store.compute_and_update(&prev, &current);
        assert_eq!(delta["claude.five_hour"], 15.0);
        assert_eq!(new.last_utilization["claude.five_hour"], 55.0);
    }

    #[test]
    fn delta_negative_on_window_reset() {
        let tmp = TempDir::new().unwrap();
        let store = StateStore::new(tmp.path().to_path_buf());
        let mut prev = PersistedState::default();
        prev.last_utilization.insert("claude.five_hour".into(), 80.0);
        let current = vec![mk_resp(Provider::Claude, vec![("five_hour", 5.0)])];
        let (_new, delta) = store.compute_and_update(&prev, &current);
        assert_eq!(delta["claude.five_hour"], -75.0);
    }

    #[test]
    fn save_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let store = StateStore::new(tmp.path().to_path_buf());
        let mut s = PersistedState::default();
        s.last_utilization.insert("claude.five_hour".into(), 42.0);
        store.save(&s).unwrap();
        let loaded = store.load();
        assert_eq!(loaded.last_utilization["claude.five_hour"], 42.0);
    }
}
