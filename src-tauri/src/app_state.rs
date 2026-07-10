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
    /// 서버가 지정한 rate-limit 냉각 종료 시각(unix secs) — provider별.
    cooldown_until: Mutex<HashMap<Provider, i64>>,
    /// provider별 in-flight 락 — 같은 순간 중복 네트워크 요청을 직렬화한다.
    fetch_locks: [Mutex<()>; 3],
    snapshot_path: PathBuf,
}

fn lock_index(p: Provider) -> usize {
    match p {
        Provider::Claude => 0,
        Provider::Codex => 1,
        Provider::Gemini => 2,
    }
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
            cooldown_until: Mutex::new(HashMap::new()),
            fetch_locks: [Mutex::new(()), Mutex::new(()), Mutex::new(())],
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

    /// 냉각 중이면 캐시 스냅샷을 반환(네트워크 skip). 냉각이 아니거나 캐시가 없으면 None.
    async fn cooldown_snapshot(&self, provider: Provider, now: i64) -> Option<ProviderSnapshot> {
        let until = self.cooldown_until.lock().await.get(&provider).copied();
        match until {
            Some(until) if now < until => {
                self.per_provider.lock().await.get(&provider).cloned()
            }
            _ => None,
        }
    }

    /// 캐시가 TTL 내로 신선하면 반환, 아니면 None.
    async fn fresh_snapshot(&self, provider: Provider, ttl: i64) -> Option<ProviderSnapshot> {
        let guard = self.per_provider.lock().await;
        match guard.get(&provider) {
            Some(s) if self.fresh(s, ttl) => Some(s.clone()),
            _ => None,
        }
    }

    pub async fn fetch_one(&self, provider: Provider, force: bool) -> ProviderSnapshot {
        let ttl = self.settings.load().refresh_interval_sec as i64;
        let now = chrono::Utc::now().timestamp();

        // 서버가 지정한 냉각(Retry-After) 중이면 네트워크를 절대 치지 않는다.
        // force(수동 새로고침)여도 존중한다 — 냉각 중 재요청은 429만 늘리고 서버 타이머를
        // 리셋시켜 오히려 복구를 지연시키기 때문.
        if let Some(s) = self.cooldown_snapshot(provider, now).await {
            return s;
        }
        if !force {
            if let Some(s) = self.fresh_snapshot(provider, ttl).await {
                return s;
            }
        }

        // provider별 in-flight 락 — 동시에 들어온 호출을 직렬화해 같은 순간 2번 나가는 것을 막는다.
        let _flight = self.fetch_locks[lock_index(provider)].lock().await;
        // 락을 기다리는 사이 다른 호출이 이미 갱신/냉각시켰을 수 있으니 재확인.
        if let Some(s) = self.cooldown_snapshot(provider, now).await {
            return s;
        }
        if !force {
            if let Some(s) = self.fresh_snapshot(provider, ttl).await {
                return s;
            }
        }

        let result = providers::fetch(provider).await;
        let retry_after = match &result {
            Err(AppError::RateLimited { retry_after }) => *retry_after,
            _ => None,
        };
        let resp = match result {
            Ok(r) => r,
            Err(e) => error_to_response(provider, e),
        };

        // 냉각 갱신: 레이트리밋이면 retry_after(없으면 5분, 1s~1h로 clamp)만큼 냉각, 성공이면 해제.
        match resp.status {
            Status::RateLimited => {
                let secs = retry_after.unwrap_or(300).clamp(1, 3600) as i64;
                self.cooldown_until.lock().await.insert(provider, now + secs);
            }
            Status::Ok => {
                self.cooldown_until.lock().await.remove(&provider);
            }
            _ => {}
        }

        // 일시적 오류(레이트리밋/네트워크)면 마지막 정상 수치와 시각을 유지해
        // 게이지가 비지 않고 "제한됨" 배지만 뜨도록 한다.
        let mut fetched_at = chrono::Utc::now().to_rfc3339();
        let resp = if matches!(resp.status, Status::RateLimited | Status::NetworkError) {
            let guard = self.per_provider.lock().await;
            match guard.get(&provider) {
                Some(prev) if !prev.response.windows.is_empty() => {
                    let mut merged = resp;
                    merged.windows = prev.response.windows.clone();
                    merged.extra_usage = prev.response.extra_usage.clone();
                    fetched_at = prev.fetched_at.clone();
                    merged
                }
                _ => resp,
            }
        } else {
            resp
        };
        let snap = ProviderSnapshot {
            fetched_at,
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
        AppError::RateLimited { .. } => (Status::RateLimited, "요청 제한 (429)".into()),
        AppError::Api { status: 429, .. } => (Status::RateLimited, "요청 제한 (429)".into()),
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
