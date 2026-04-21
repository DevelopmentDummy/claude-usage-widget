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
struct RawWindow {
    utilization: f64,
    #[serde(rename = "resets_at")]
    resets_at: String,
}

#[derive(Deserialize, Default)]
pub(crate) struct RawUsage {
    pub five_hour: Option<RawWindow>,
    pub seven_day: Option<RawWindow>,
    pub seven_day_sonnet: Option<RawWindow>,
    pub seven_day_opus: Option<RawWindow>,
    pub seven_day_cowork: Option<RawWindow>,
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

pub(crate) fn map_raw_to_response(raw: &RawUsage) -> UsageResponse {
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
            windows.push(UsageWindow {
                key: (*key).to_string(),
                name: (*label).to_string(),
                utilization: w.utilization,
                resets_at: w.resets_at.clone(),
                time_progress: compute_time_progress(&w.resets_at, *dur),
            });
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
        return Err(AppError::Expired);
    }
    if !status.is_success() {
        let body = res.text().await.unwrap_or_default();
        return Err(AppError::Api { status: status.as_u16(), message: body });
    }

    let raw: RawUsage = res.json().await?;
    Ok(map_raw_to_response(&raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_five_hour_and_seven_day() {
        let raw = RawUsage {
            five_hour: Some(RawWindow { utilization: 42.0, resets_at: "2030-01-01T00:00:00Z".into() }),
            seven_day: Some(RawWindow { utilization: 10.0, resets_at: "2030-01-01T00:00:00Z".into() }),
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
    fn time_progress_zero_when_reset_far_future() {
        let far = "2099-01-01T00:00:00Z";
        let tp = compute_time_progress(far, 5 * 60 * 60);
        assert_eq!(tp, 0.0);
    }
}
