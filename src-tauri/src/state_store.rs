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
    /// provider str -> 429 Retry-After 냉각 종료 시각(unix secs).
    /// 메모리 전용이던 쿨다운을 디스크에 남겨 재시작 시 유실을 막는다
    /// (유실되면 부팅 즉시 재요청 → 또 429 → 벌점 누적).
    #[serde(default)]
    pub cooldowns: HashMap<String, i64>,
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
            // 쿨다운은 fetch_one이 관리하므로 여기서는 그대로 보존한다.
            cooldowns: prev.cooldowns.clone(),
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

    #[test]
    fn cooldowns_survive_save_load() {
        let tmp = TempDir::new().unwrap();
        let store = StateStore::new(tmp.path().to_path_buf());
        let mut s = PersistedState::default();
        s.cooldowns.insert("claude".into(), 1_800_000_000);
        store.save(&s).unwrap();
        let loaded = store.load();
        assert_eq!(loaded.cooldowns.get("claude").copied(), Some(1_800_000_000));
    }

    #[test]
    fn compute_and_update_preserves_cooldowns() {
        let tmp = TempDir::new().unwrap();
        let store = StateStore::new(tmp.path().to_path_buf());
        let mut prev = PersistedState::default();
        prev.cooldowns.insert("claude".into(), 1_800_000_000);
        let current = vec![mk_resp(Provider::Codex, vec![("primary", 3.0)])];
        let (new, _delta) = store.compute_and_update(&prev, &current);
        // Codex 성공 갱신이 Claude 쿨다운을 지워버리지 않아야 한다.
        assert_eq!(new.cooldowns.get("claude").copied(), Some(1_800_000_000));
    }
}
