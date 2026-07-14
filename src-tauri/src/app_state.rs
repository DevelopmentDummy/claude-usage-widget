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
    /// rate_limited일 때 다음 네트워크 요청이 허용되는 시각(rfc3339 = 쿨다운 종료).
    /// 프론트가 "다음 요청 HH:MM"을 표시하는 데 쓴다.
    #[serde(rename = "retryAt", default, skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<String>,
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
    /// state.json에 영속화되어 재시작 후에도 존중된다.
    cooldown_until: Mutex<HashMap<Provider, i64>>,
    /// provider별 마지막 네트워크 시도 시각(unix secs) — force여도 최소 간격 하한을
    /// 두어 버튼 연타/중복 이벤트 버스트가 한도를 터뜨리는 것을 막는다.
    last_attempt: Mutex<HashMap<Provider, i64>>,
    /// 프론트에서 현재 보고 있는 활성 provider — 폴러는 이 provider만 주기 갱신한다.
    active_provider: Mutex<Provider>,
    /// provider별 in-flight 락 — 같은 순간 중복 네트워크 요청을 직렬화한다.
    fetch_locks: [Mutex<()>; 3],
    snapshot_path: PathBuf,
}

/// force=true여도 이 간격(초) 안에는 재요청하지 않는다.
const MIN_FETCH_INTERVAL_SEC: i64 = 15;

/// rate-limit 재요청 시 서버가 준 Retry-After에 더하는 여유 버퍼(초).
/// 경계 정각에 딱 재요청하면 슬라이딩 윈도우/시계 오차로 또 429를 맞는 일이
/// 많아, 조금 넘긴 뒤 재시도해 루프를 탈출할 확률을 높인다.
const RETRY_BUFFER_SEC: i64 = 60;

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
        // 영속화된 쿨다운을 복원한다(만료된 항목은 cooldown_snapshot에서 자연히 무시됨).
        let mut cooldowns_init: HashMap<Provider, i64> = HashMap::new();
        for (k, v) in &persisted.cooldowns {
            if let Some(p) = provider_from_str(k) {
                cooldowns_init.insert(p, *v);
            }
        }
        Arc::new(Self {
            cache: UsageCache::new(),
            settings: SettingsStore::new(app_data_dir),
            state,
            persisted: Mutex::new(persisted),
            per_provider: Mutex::new(loaded),
            cooldown_until: Mutex::new(cooldowns_init),
            last_attempt: Mutex::new(HashMap::new()),
            active_provider: Mutex::new(Provider::Claude),
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

    /// Ok 스냅샷만 디스크에 저장한다. 예전엔 맵 전체를 통째로 썼기 때문에
    /// Codex/Gemini가 성공하는 순간 Claude의 rate_limited 스냅샷까지 같이
    /// 디스크에 굳어, 재시작하면 "요청 제한" 배지가 그대로 살아나는 버그가 있었다.
    async fn persist_snapshots(&self) {
        let guard = self.per_provider.lock().await;
        let file = SnapshotFile {
            providers: guard
                .iter()
                .filter(|(_, v)| v.response.status == Status::Ok)
                .map(|(k, v)| (k.as_str().to_string(), v.clone()))
                .collect(),
        };
        drop(guard);
        if let Ok(bytes) = serde_json::to_vec_pretty(&file) {
            let _ = atomic_write(&self.snapshot_path, &bytes);
        }
    }

    pub async fn set_active_provider(&self, provider: Provider) {
        *self.active_provider.lock().await = provider;
    }

    pub async fn active_provider(&self) -> Provider {
        *self.active_provider.lock().await
    }

    /// 현재 쿨다운 맵(미만료 항목만)을 state.json에 반영한다.
    async fn sync_cooldowns_to_disk(&self) {
        let now = chrono::Utc::now().timestamp();
        let snapshot: HashMap<String, i64> = {
            self.cooldown_until
                .lock()
                .await
                .iter()
                .filter(|(_, &until)| until > now)
                .map(|(p, v)| (p.as_str().to_string(), *v))
                .collect()
        };
        let mut persisted = self.persisted.lock().await;
        if persisted.cooldowns != snapshot {
            persisted.cooldowns = snapshot;
            let _ = self.state.save(&persisted);
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

        // force=true여도 최소 간격 하한을 둔다: 직전 시도가 너무 최근이면 캐시를 반환한다.
        // (F5/헤더 새로고침/만료버튼 연타·중복 이벤트로 같은 provider에 요청이 몰려
        //  민감한 usage 엔드포인트의 한도를 터뜨리는 것을 백엔드에서 차단)
        let now = chrono::Utc::now().timestamp();
        if force {
            let last = self.last_attempt.lock().await.get(&provider).copied();
            if let Some(t) = last {
                if now - t < MIN_FETCH_INTERVAL_SEC {
                    if let Some(s) = self.per_provider.lock().await.get(&provider).cloned() {
                        return s;
                    }
                }
            }
        }
        self.last_attempt.lock().await.insert(provider, now);

        let result = providers::fetch(provider).await;
        // primary가 429였는지(그리고 서버 Retry-After)를 폴백/쿨다운 판단용으로 먼저 기록.
        let primary_retry_after = match &result {
            Err(AppError::RateLimited { retry_after }) => Some(*retry_after),
            _ => None,
        };

        // Claude 전용 폴백: primary(/api/oauth/usage)가 429로 튕기면 rate-limit 응답 헤더
        // 방식(/v1/messages haiku 1회)으로 신선한 5시간/7일 수치를 확보한다. primary의
        // Retry-After 쿨다운은 아래에서 그대로 설정되므로 이 폴백은 쿨다운당 최대 1회만 돈다.
        let result = if provider == Provider::Claude && primary_retry_after.is_some() {
            match providers::fetch_claude_ratelimit_headers().await {
                Ok(fb) => {
                    crate::diag::log("app_state", "claude primary 429 → ratelimit-header fallback ok");
                    Ok(fb)
                }
                Err(e) => {
                    crate::diag::log("app_state", &format!("claude fallback failed: {}", e));
                    result
                }
            }
        } else {
            result
        };

        let resp = match result {
            Ok(r) => r,
            Err(e) => error_to_response(provider, e),
        };

        // 냉각 갱신: primary가 429였으면(폴백 성공 여부와 무관하게) Retry-After+버퍼만큼 냉각.
        // 폴백이 성공해 resp가 Ok여도 primary 쿨다운은 유지해 throttle된 엔드포인트를 덜 때린다.
        // 성공이면 해제. retry_at(다음 primary 시도 시각)도 버퍼 포함 값으로 표시된다.
        let mut retry_at: Option<String> = None;
        if let Some(ra) = primary_retry_after {
            let base = ra.unwrap_or(300).clamp(1, 3600) as i64;
            let secs = base + RETRY_BUFFER_SEC;
            let until = now + secs;
            self.cooldown_until.lock().await.insert(provider, until);
            self.sync_cooldowns_to_disk().await;
            retry_at = chrono::DateTime::from_timestamp(until, 0).map(|dt| dt.to_rfc3339());
        } else if resp.status == Status::Ok {
            let removed = self.cooldown_until.lock().await.remove(&provider).is_some();
            if removed {
                self.sync_cooldowns_to_disk().await;
            }
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
            retry_at,
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
