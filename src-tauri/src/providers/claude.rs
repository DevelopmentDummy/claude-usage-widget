use std::path::PathBuf;

use serde::Deserialize;

use crate::errors::{AppError, AppResult};
use crate::types::{Provider, Status, UsageResponse, UsageWindow};

#[derive(Deserialize)]
struct Creds {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: OauthBlock,
}

#[derive(Deserialize)]
struct OauthBlock {
    #[serde(rename = "accessToken")]
    access_token: String,
}

#[derive(Deserialize)]
pub(crate) struct RawWindow {
    utilization: f64,
    #[serde(rename = "resets_at", default)]
    resets_at: Option<String>,
}

#[derive(Deserialize, Default)]
pub(crate) struct RawScopeModel {
    pub display_name: Option<String>,
}

#[derive(Deserialize, Default)]
pub(crate) struct RawScope {
    pub model: Option<RawScopeModel>,
}

#[derive(Deserialize)]
pub(crate) struct RawLimit {
    pub kind: String,
    pub group: String,
    pub percent: f64,
    pub resets_at: Option<String>,
    pub scope: Option<RawScope>,
}

#[derive(Deserialize, Default)]
pub(crate) struct RawUsage {
    pub five_hour: Option<RawWindow>,
    pub seven_day: Option<RawWindow>,
    pub seven_day_sonnet: Option<RawWindow>,
    pub seven_day_opus: Option<RawWindow>,
    pub seven_day_cowork: Option<RawWindow>,
    pub limits: Option<Vec<RawLimit>>,
}

fn credentials_path() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".claude").join(".credentials.json")
}

fn read_token() -> AppResult<String> {
    let path = credentials_path();
    let raw = std::fs::read_to_string(&path)
        .map_err(|_| AppError::NotAuthenticated("claude credentials not found".into()))?;
    let creds: Creds = serde_json::from_str(&raw)
        .map_err(|_| AppError::NotAuthenticated("claude credentials malformed".into()))?;
    Ok(creds.claude_ai_oauth.access_token)
}

const WIN_DEFS: &[(&str, &str, u64)] = &[
    ("five_hour", "5시간", 5 * 60 * 60),
    ("seven_day", "7일", 7 * 24 * 60 * 60),
    ("seven_day_sonnet", "7일 (Sonnet)", 7 * 24 * 60 * 60),
    ("seven_day_opus", "7일 (Opus)", 7 * 24 * 60 * 60),
    ("seven_day_cowork", "7일 (Cowork)", 7 * 24 * 60 * 60),
];

fn compute_time_progress(resets_at: &str, duration_sec: u64) -> f64 {
    let reset = match chrono::DateTime::parse_from_rfc3339(resets_at) {
        Ok(dt) => dt.timestamp(),
        Err(_) => return 0.0,
    };
    let now = chrono::Utc::now().timestamp();
    let start = reset - duration_sec as i64;
    if now <= start { return 0.0; }
    if now >= reset { return 100.0; }
    ((now - start) as f64 / duration_sec as f64 * 100.0).round()
}

const HOUR5: u64 = 5 * 60 * 60;
const DAY7: u64 = 7 * 24 * 60 * 60;

fn window_from_reset(key: String, name: String, utilization: f64, resets_at: Option<&str>, dur: u64) -> UsageWindow {
    let (resets_at, tp) = match resets_at {
        Some(s) if !s.is_empty() => (s.to_string(), compute_time_progress(s, dur)),
        _ => (String::new(), 100.0),
    };
    UsageWindow { key, name, utilization, resets_at, time_progress: tp }
}

fn windows_from_limits(limits: &[RawLimit]) -> Vec<UsageWindow> {
    let mut windows = Vec::new();
    for l in limits {
        let (key, name, dur) = if l.group == "session" {
            ("five_hour".to_string(), "5시간".to_string(), HOUR5)
        } else if l.group == "weekly" {
            if l.kind == "weekly_all" {
                ("seven_day".to_string(), "7일".to_string(), DAY7)
            } else {
                let model = l
                    .scope
                    .as_ref()
                    .and_then(|s| s.model.as_ref())
                    .and_then(|m| m.display_name.as_deref());
                match model {
                    Some(m) => (
                        format!("weekly_scoped_{}", m.to_lowercase().replace(' ', "_")),
                        format!("7일 ({})", m),
                        DAY7,
                    ),
                    None => continue,
                }
            }
        } else {
            continue;
        };
        windows.push(window_from_reset(key, name, l.percent, l.resets_at.as_deref(), dur));
    }
    windows
}

pub(crate) fn map_raw_to_response(raw: &RawUsage) -> UsageResponse {
    if let Some(limits) = &raw.limits {
        let windows = windows_from_limits(limits);
        if !windows.is_empty() {
            return UsageResponse {
                provider: Provider::Claude,
                status: Status::Ok,
                windows,
                extra_usage: None,
                error: None,
            };
        }
    }
    let mut windows = Vec::new();
    for (key, label, dur) in WIN_DEFS {
        let w = match *key {
            "five_hour" => raw.five_hour.as_ref(),
            "seven_day" => raw.seven_day.as_ref(),
            "seven_day_sonnet" => raw.seven_day_sonnet.as_ref(),
            "seven_day_opus" => raw.seven_day_opus.as_ref(),
            "seven_day_cowork" => raw.seven_day_cowork.as_ref(),
            _ => None,
        };
        if let Some(w) = w {
            windows.push(window_from_reset(
                (*key).to_string(),
                (*label).to_string(),
                w.utilization,
                w.resets_at.as_deref(),
                *dur,
            ));
        }
    }
    UsageResponse {
        provider: Provider::Claude,
        status: Status::Ok,
        windows,
        extra_usage: None,
        error: None,
    }
}

pub async fn fetch() -> AppResult<UsageResponse> {
    let token = read_token()?;
    let client = reqwest::Client::new();
    let res = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .header("Authorization", format!("Bearer {}", token))
        .header("anthropic-beta", "oauth-2025-04-20")
        .send()
        .await?;

    let status = res.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        crate::diag::log("claude", "fetch: 401 unauthorized -> Expired");
        return Err(AppError::Expired);
    }
    if !status.is_success() {
        let retry_after = res
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok());
        let body = res.text().await.unwrap_or_default();
        crate::diag::log(
            "claude",
            &format!(
                "fetch: non-2xx status={} retry_after={:?} body={}",
                status.as_u16(),
                retry_after,
                body
            ),
        );
        if status.as_u16() == 429 {
            return Err(AppError::RateLimited { retry_after });
        }
        return Err(AppError::Api { status: status.as_u16(), message: body });
    }

    let body = res.text().await.map_err(|e| {
        crate::diag::log("claude", &format!("fetch: read body err={}", e));
        AppError::from(e)
    })?;
    match serde_json::from_str::<RawUsage>(&body) {
        Ok(raw) => Ok(map_raw_to_response(&raw)),
        Err(e) => {
            let preview: String = body.chars().take(400).collect();
            crate::diag::log(
                "claude",
                &format!(
                    "fetch: 2xx body decode failed err={} body_len={} preview={:?}",
                    e,
                    body.len(),
                    preview
                ),
            );
            Err(AppError::Expired)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_five_hour_and_seven_day() {
        let raw = RawUsage {
            five_hour: Some(RawWindow { utilization: 42.0, resets_at: Some("2030-01-01T00:00:00Z".into()) }),
            seven_day: Some(RawWindow { utilization: 10.0, resets_at: Some("2030-01-01T00:00:00Z".into()) }),
            ..Default::default()
        };
        let resp = map_raw_to_response(&raw);
        assert_eq!(resp.provider, Provider::Claude);
        assert_eq!(resp.windows.len(), 2);
        assert_eq!(resp.windows[0].key, "five_hour");
        assert_eq!(resp.windows[0].utilization, 42.0);
        assert_eq!(resp.windows[1].key, "seven_day");
    }

    #[test]
    fn skips_missing_windows() {
        let raw = RawUsage::default();
        let resp = map_raw_to_response(&raw);
        assert_eq!(resp.windows.len(), 0);
    }

    #[test]
    fn limits_array_takes_priority_and_maps_fable_scope() {
        // 2026-07-03 실제 응답 축약본 (limits 기반, 최상위 모델 키는 전부 null)
        let body = r#"{
            "five_hour": {"utilization": 25.0, "resets_at": "2030-01-01T00:00:00Z"},
            "seven_day": {"utilization": 20.0, "resets_at": "2030-01-01T00:00:00Z"},
            "seven_day_opus": null,
            "limits": [
                {"kind":"session","group":"session","percent":25,"severity":"normal","resets_at":"2030-01-01T00:00:00Z","scope":null,"is_active":false},
                {"kind":"weekly_all","group":"weekly","percent":20,"severity":"normal","resets_at":"2030-01-01T00:00:00Z","scope":null,"is_active":false},
                {"kind":"weekly_scoped","group":"weekly","percent":31,"severity":"normal","resets_at":"2030-01-01T00:00:00Z","scope":{"model":{"id":null,"display_name":"Fable"},"surface":null},"is_active":true}
            ]
        }"#;
        let raw: RawUsage = serde_json::from_str(body).unwrap();
        let resp = map_raw_to_response(&raw);
        assert_eq!(resp.windows.len(), 3);
        assert_eq!(resp.windows[0].key, "five_hour");
        assert_eq!(resp.windows[0].name, "5시간");
        assert_eq!(resp.windows[0].utilization, 25.0);
        assert_eq!(resp.windows[1].key, "seven_day");
        assert_eq!(resp.windows[1].name, "7일");
        assert_eq!(resp.windows[2].key, "weekly_scoped_fable");
        assert_eq!(resp.windows[2].name, "7일 (Fable)");
        assert_eq!(resp.windows[2].utilization, 31.0);
    }

    #[test]
    fn falls_back_to_top_level_keys_when_limits_missing() {
        let body = r#"{
            "five_hour": {"utilization": 42.0, "resets_at": "2030-01-01T00:00:00Z"},
            "seven_day": {"utilization": 10.0, "resets_at": "2030-01-01T00:00:00Z"},
            "limits": null
        }"#;
        let raw: RawUsage = serde_json::from_str(body).unwrap();
        let resp = map_raw_to_response(&raw);
        assert_eq!(resp.windows.len(), 2);
        assert_eq!(resp.windows[0].key, "five_hour");
        assert_eq!(resp.windows[1].key, "seven_day");
    }

    #[test]
    fn scoped_limit_without_model_name_is_skipped() {
        let body = r#"{
            "limits": [
                {"kind":"weekly_all","group":"weekly","percent":20,"resets_at":"2030-01-01T00:00:00Z","scope":null},
                {"kind":"weekly_scoped","group":"weekly","percent":31,"resets_at":"2030-01-01T00:00:00Z","scope":{"model":null,"surface":null}},
                {"kind":"mystery","group":"monthly","percent":5,"resets_at":"2030-01-01T00:00:00Z","scope":null}
            ]
        }"#;
        let raw: RawUsage = serde_json::from_str(body).unwrap();
        let resp = map_raw_to_response(&raw);
        assert_eq!(resp.windows.len(), 1);
        assert_eq!(resp.windows[0].key, "seven_day");
    }

    #[test]
    fn time_progress_zero_when_reset_far_future() {
        let far = "2099-01-01T00:00:00Z";
        let tp = compute_time_progress(far, 5 * 60 * 60);
        assert_eq!(tp, 0.0);
    }
}
