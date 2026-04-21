use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::cache::UsageCache;
use crate::errors::AppError;
use crate::providers;
use crate::settings::SettingsStore;
use crate::state_store::{PersistedState, StateStore};
use crate::types::{Provider, Status, UsageResponse};

#[derive(Serialize, Deserialize, Clone)]
pub struct ProviderSnapshot {
    #[serde(rename = "fetchedAt")]
    pub fetched_at: String,
    pub response: UsageResponse,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct SnapshotFile {
    #[serde(default)]
    pub providers: HashMap<String, ProviderSnapshot>,
}

pub struct AppState {
    pub cache: UsageCache,
    pub settings: SettingsStore,
    pub state: StateStore,
    pub persisted: Mutex<PersistedState>,
    pub per_provider: Mutex<HashMap<Provider, ProviderSnapshot>>,
    snapshot_path: PathBuf,
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

fn provider_from_str(s: &str) -> Option<Provider> {
    match s {
        "claude" => Some(Provider::Claude),
        "codex" => Some(Provider::Codex),
        "gemini" => Some(Provider::Gemini),
        _ => None,
    }
}

impl AppState {
    pub fn new(app_data_dir: PathBuf) -> Arc<Self> {
        let state = StateStore::new(app_data_dir.clone());
        let persisted = state.load();
        let snapshot_path = app_data_dir.join("usage_snapshot.json");
        let mut loaded: HashMap<Provider, ProviderSnapshot> = HashMap::new();
        if let Ok(raw) = fs::read_to_string(&snapshot_path) {
            if let Ok(file) = serde_json::from_str::<SnapshotFile>(&raw) {
                for (k, v) in file.providers {
                    if let Some(p) = provider_from_str(&k) {
                        loaded.insert(p, v);
                    }
                }
            }
        }
        Arc::new(Self {
            cache: UsageCache::new(),
            settings: SettingsStore::new(app_data_dir),
            state,
            persisted: Mutex::new(persisted),
            per_provider: Mutex::new(loaded),
            snapshot_path,
        })
    }

    pub async fn current_snapshots(&self) -> HashMap<String, ProviderSnapshot> {
        let guard = self.per_provider.lock().await;
        guard
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), v.clone()))
            .collect()
    }

    async fn persist_snapshots(&self) {
        let guard = self.per_provider.lock().await;
        let file = SnapshotFile {
            providers: guard
                .iter()
                .map(|(k, v)| (k.as_str().to_string(), v.clone()))
                .collect(),
        };
        drop(guard);
        if let Ok(bytes) = serde_json::to_vec_pretty(&file) {
            let _ = atomic_write(&self.snapshot_path, &bytes);
        }
    }

    fn fresh(&self, snap: &ProviderSnapshot, ttl_sec: i64) -> bool {
        let dt = match chrono::DateTime::parse_from_rfc3339(&snap.fetched_at) {
            Ok(d) => d,
            Err(_) => return false,
        };
        let age = chrono::Utc::now()
            .signed_duration_since(dt.with_timezone(&chrono::Utc))
            .num_seconds();
        (0..ttl_sec).contains(&age)
    }

    pub async fn fetch_one(&self, provider: Provider, force: bool) -> ProviderSnapshot {
        let ttl = self.settings.load().refresh_interval_sec as i64;
        if !force {
            let guard = self.per_provider.lock().await;
            if let Some(s) = guard.get(&provider) {
                if self.fresh(s, ttl) {
                    return s.clone();
                }
            }
        }
        let resp = match providers::fetch(provider).await {
            Ok(r) => r,
            Err(e) => error_to_response(provider, e),
        };
        let snap = ProviderSnapshot {
            fetched_at: chrono::Utc::now().to_rfc3339(),
            response: resp.clone(),
        };
        {
            let mut guard = self.per_provider.lock().await;
            guard.insert(provider, snap.clone());
        }
        if resp.status == Status::Ok {
            self.persist_snapshots().await;
        }
        snap
    }

    pub async fn fetch_all(&self, force: bool) -> Vec<(Provider, ProviderSnapshot)> {
        let providers_list = [Provider::Claude, Provider::Codex, Provider::Gemini];
        let futs = providers_list
            .iter()
            .map(|&p| async move { (p, self.fetch_one(p, force).await) });
        let results: Vec<(Provider, ProviderSnapshot)> = futures::future::join_all(futs).await;
        let responses: Vec<UsageResponse> = results.iter().map(|(_, s)| s.response.clone()).collect();
        let mut persisted = self.persisted.lock().await;
        let (new_state, _delta) = self.state.compute_and_update(&*persisted, &responses);
        *persisted = new_state.clone();
        let _ = self.state.save(&new_state);
        results
    }
}

fn error_to_response(provider: Provider, err: AppError) -> UsageResponse {
    let (status, msg) = match err {
        AppError::NotAuthenticated(m) => (Status::NotAuthenticated, m),
        AppError::Expired => (Status::Expired, "token expired".into()),
        AppError::Http(e) => (Status::NetworkError, e.to_string()),
        AppError::Api { status: 429, .. } => (Status::NetworkError, "rate limited".into()),
        AppError::Api { status, message } => (Status::UnknownError, format!("api {}: {}", status, message)),
        other => (Status::UnknownError, other.to_string()),
    };
    UsageResponse {
        provider,
        status,
        windows: vec![],
        extra_usage: None,
        error: Some(msg),
    }
}
