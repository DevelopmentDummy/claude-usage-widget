use std::path::PathBuf;

use serde::Deserialize;

use crate::errors::{AppError, AppResult};
use crate::providers::antigravity_cred;
use crate::types::{Provider, Status, UsageResponse, UsageWindow};

// === Legacy gemini-cli token (oauth_creds.json) ===

#[derive(Deserialize)]
struct Creds {
    access_token: String,
    #[serde(default)]
    expiry_date: Option<i64>,
}

#[derive(Deserialize)]
struct ProjectsFile {
    #[serde(default)]
    projects: std::collections::HashMap<String, String>,
}

fn creds_path() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".gemini").join("oauth_creds.json")
}

fn projects_path() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".gemini").join("projects.json")
}

fn read_legacy_token() -> AppResult<String> {
    let s = std::fs::read_to_string(creds_path())
        .map_err(|_| AppError::NotAuthenticated("gemini creds not found".into()))?;
    let c: Creds = serde_json::from_str(&s)
        .map_err(|_| AppError::NotAuthenticated("gemini creds malformed".into()))?;
    Ok(c.access_token)
}

fn read_first_project_id() -> Option<String> {
    let s = std::fs::read_to_string(projects_path()).ok()?;
    let pf: ProjectsFile = serde_json::from_str(&s).ok()?;
    pf.projects.into_values().next()
}

// === Antigravity token (wincred: gemini:antigravity) ===

#[derive(Deserialize)]
struct AgyBlob {
    token: AgyToken,
}

#[derive(Deserialize)]
struct AgyToken {
    access_token: String,
}

fn read_agy_token() -> Option<String> {
    let blob = antigravity_cred::read_token_blob("gemini:antigravity")?;
    let s = String::from_utf8(blob).ok()?;
    let parsed: AgyBlob = serde_json::from_str(s.trim_end_matches('\0')).ok()?;
    Some(parsed.token.access_token)
}

// === Legacy retrieveUserQuota response (REQUESTS buckets) ===

#[derive(Deserialize)]
pub(crate) struct QuotaBucket {
    #[serde(rename = "resetTime")]
    pub resets_at: String,
    #[serde(rename = "tokenType")]
    pub token_type: String,
    #[serde(rename = "modelId")]
    pub model_id: String,
    #[serde(rename = "remainingFraction")]
    pub remaining_fraction: f64,
}

#[derive(Deserialize, Default)]
pub(crate) struct QuotaResponse {
    #[serde(default)]
    pub buckets: Vec<QuotaBucket>,
}

// === Antigravity loadCodeAssist + retrieveUserQuotaSummary schemas ===
// Since late 2026 Antigravity quota is no longer per-model: models are pooled into
// groups (Gemini / Claude+GPT), each sharing a 5-hour and a weekly limit.

#[derive(Deserialize)]
struct LoadCodeAssistResp {
    #[serde(default, rename = "cloudaicompanionProject")]
    project: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct QuotaSummaryResp {
    #[serde(default)]
    groups: Vec<SummaryGroup>,
}

#[derive(Deserialize)]
struct SummaryGroup {
    #[serde(default)]
    buckets: Vec<SummaryBucket>,
}

#[derive(Deserialize)]
struct SummaryBucket {
    #[serde(default, rename = "bucketId")]
    bucket_id: String,
    #[serde(default)]
    window: String,
    #[serde(default, rename = "resetTime")]
    reset_time: Option<String>,
    #[serde(default, rename = "remainingFraction")]
    remaining_fraction: Option<f64>,
}

// === Family classification (legacy retrieveUserQuota) ===
// Family display order: Flash, Pro (Claude/GPT-OSS never appear on this path).
// flash-lite collapses into "flash"; "other" is unmapped (e.g., agent helpers).

fn model_tier(model_id: &str) -> &'static str {
    if model_id.starts_with("claude-") { "claude" }
    else if model_id.starts_with("gpt-oss") || model_id.starts_with("gpt-") { "gpt" }
    else if model_id.contains("flash") { "flash" }
    else if model_id.contains("pro") { "pro" }
    else { "other" }
}

fn tier_label(key: &str) -> &'static str {
    match key {
        "flash" => "Flash",
        "pro" => "Pro",
        "claude" => "Claude",
        "gpt" => "GPT-OSS",
        _ => "Other",
    }
}

const TIER_ORDER: &[&str] = &["flash", "pro", "claude", "gpt"];
const DAY_SEC: u64 = 24 * 60 * 60;

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

// === Legacy mapping (retrieveUserQuota → tiered windows) ===

pub(crate) fn map_raw_to_response(raw: &QuotaResponse) -> UsageResponse {
    let mut first_per_tier: std::collections::HashMap<&str, &QuotaBucket> = Default::default();
    for b in &raw.buckets {
        if b.token_type != "REQUESTS" { continue; }
        let tier = model_tier(&b.model_id);
        first_per_tier.entry(tier).or_insert(b);
    }

    let mut windows = Vec::new();
    for key in TIER_ORDER {
        if let Some(b) = first_per_tier.get(key) {
            let util = ((1.0 - b.remaining_fraction) * 100.0).round();
            windows.push(UsageWindow {
                key: (*key).to_string(),
                name: tier_label(key).to_string(),
                utilization: util,
                resets_at: b.resets_at.clone(),
                time_progress: compute_time_progress(&b.resets_at, DAY_SEC),
            });
        }
    }

    UsageResponse {
        provider: Provider::Gemini,
        status: Status::Ok,
        windows,
        extra_usage: None,
        error: None,
    }
}

// === Antigravity mapping (retrieveUserQuotaSummary → group windows) ===
// Only explicit Gemini five-hour and weekly buckets are displayed.

const FIVE_HOUR_SEC: u64 = 5 * 60 * 60;
const WEEK_SEC: u64 = 7 * DAY_SEC;

fn map_summary_to_response(raw: &QuotaSummaryResp) -> UsageResponse {
    let mut windows = Vec::new();
    for (period, key, name, duration) in [
        ("5h", "gemini-5h", "Gemini 5시간", FIVE_HOUR_SEC),
        ("weekly", "gemini-weekly", "Gemini 7일", WEEK_SEC),
    ] {
        let bucket = raw.groups.iter().flat_map(|g| &g.buckets).find(|b| {
            b.bucket_id.starts_with("gemini-") && b.window == period
                && b.remaining_fraction.is_some_and(|f| f.is_finite())
                && b.reset_time.as_ref().is_some_and(|r| !r.is_empty())
        });
        if let Some(b) = bucket {
            let reset = b.reset_time.as_ref().unwrap();
            windows.push(UsageWindow {
                key: key.into(),
                name: name.into(),
                utilization: (1.0 - b.remaining_fraction.unwrap()) * 100.0,
                time_progress: compute_time_progress(reset, duration),
                resets_at: reset.clone(),
            });
        }
    }
    UsageResponse {
        provider: Provider::Gemini,
        status: Status::Ok,
        windows,
        extra_usage: None,
        error: None,
    }
}

// === HTTP helpers ===
// agy (Antigravity CLI) talks only to the daily host — generation, quota summary
// and fetchAvailableModels alike — so its usage is charged to the daily pool.
// The prod host (cloudcode-pa) reports a separate Gemini pool that agy usage
// never moves (verified 2026-10-20), so it must not be used here.

const AGY_USER_AGENT: &str = "antigravity/cli/1.4.12 (aidev_client; os_type=windows; arch=amd64)";

const DAILY_BASE: &str = "https://daily-cloudcode-pa.googleapis.com/v1internal";

async fn post_daily<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    method: &str,
    token: &str,
    body: &serde_json::Value,
) -> AppResult<T> {
    let res = client
        .post(format!("{}:{}", DAILY_BASE, method))
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .header("User-Agent", AGY_USER_AGENT)
        .body(body.to_string())
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
    Ok(res.json().await?)
}

async fn fetch_antigravity(token: &str) -> AppResult<UsageResponse> {
    let client = reqwest::Client::new();

    // 1. loadCodeAssist → cloudaicompanionProject
    let lca_body = serde_json::json!({
        "metadata": { "ideType": "ANTIGRAVITY", "platform": "PLATFORM_UNSPECIFIED", "pluginType": "GEMINI" }
    });
    let lca: LoadCodeAssistResp = post_daily(&client, "loadCodeAssist", token, &lca_body).await?;
    let project_id = match lca.project {
        Some(serde_json::Value::String(s)) => Some(s),
        Some(serde_json::Value::Object(o)) => o.get("id").and_then(|v| v.as_str()).map(String::from),
        _ => None,
    };

    // 2. retrieveUserQuotaSummary
    let body = match &project_id {
        Some(p) => serde_json::json!({ "project": p }),
        None => serde_json::json!({}),
    };
    let raw: QuotaSummaryResp = post_daily(&client, "retrieveUserQuotaSummary", token, &body).await?;
    // Model quotaInfo has no explicit period and may represent weekly usage.
    Ok(map_summary_to_response(&raw))
}

async fn fetch_legacy(token: &str) -> AppResult<UsageResponse> {
    let project_id = read_first_project_id();
    let body = match project_id {
        Some(p) => serde_json::json!({ "project": p }),
        None => serde_json::json!({}),
    };
    let client = reqwest::Client::new();
    let res = client
        .post("https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota")
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .body(body.to_string())
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

    let raw: QuotaResponse = res.json().await?;
    Ok(map_raw_to_response(&raw))
}

pub async fn fetch() -> AppResult<UsageResponse> {
    // Prefer agy (Antigravity CLI) token if present — covers post-2026-06-18 era
    // and reports the pooled 5-hour / weekly limits.
    if let Some(token) = read_agy_token() {
        return fetch_antigravity(&token).await;
    }
    let token = read_legacy_token()?;
    fetch_legacy(&token).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_dedupe_tier_and_compute_utilization() {
        let raw = QuotaResponse {
            buckets: vec![
                QuotaBucket {
                    resets_at: "2030-01-01T00:00:00Z".into(),
                    token_type: "REQUESTS".into(),
                    model_id: "gemini-2.0-flash-exp".into(),
                    remaining_fraction: 0.2,
                },
                QuotaBucket {
                    resets_at: "2030-01-01T00:00:00Z".into(),
                    token_type: "REQUESTS".into(),
                    model_id: "gemini-2.0-flash-002".into(),
                    remaining_fraction: 0.5,
                },
                QuotaBucket {
                    resets_at: "2030-01-01T00:00:00Z".into(),
                    token_type: "REQUESTS".into(),
                    model_id: "gemini-2.5-pro".into(),
                    remaining_fraction: 0.9,
                },
            ],
        };
        let resp = map_raw_to_response(&raw);
        // 4-family taxonomy: legacy response only fills flash + pro
        assert_eq!(resp.windows.len(), 2);
        assert_eq!(resp.windows[0].key, "flash");
        assert_eq!(resp.windows[0].utilization, 80.0);
        assert_eq!(resp.windows[1].key, "pro");
        assert_eq!(resp.windows[1].utilization, 10.0);
    }

    #[test]
    fn legacy_skips_non_requests_tokens() {
        let raw = QuotaResponse {
            buckets: vec![QuotaBucket {
                resets_at: "2030-01-01T00:00:00Z".into(),
                token_type: "INPUT_TOKENS".into(),
                model_id: "gemini-2.5-pro".into(),
                remaining_fraction: 0.5,
            }],
        };
        let resp = map_raw_to_response(&raw);
        assert_eq!(resp.windows.len(), 0);
    }

    #[test]
    fn explicit_windows_preserve_precision_reset_and_order() {
        let raw: QuotaSummaryResp = serde_json::from_str(r#"{
          "groups": [{ "buckets": [
            {"bucketId":"gemini-weekly","window":"weekly","remainingFraction":0.114,"resetTime":"2026-11-07T20:44:03Z"},
            {"bucketId":"3p-5h","window":"5h","remainingFraction":0,"resetTime":"2026-11-03T23:41:01Z"},
            {"bucketId":"gemini-5h","window":"5h","remainingFraction":0.62,"resetTime":"2026-11-03T23:41:01Z"},
            {"bucketId":"gemini-duplicate","window":"5h","remainingFraction":0.9,"resetTime":"2030-01-01T00:00:00Z"}
          ]}]
        }"#).unwrap();
        let resp = map_summary_to_response(&raw);
        assert_eq!(resp.windows.len(), 2);
        assert_eq!(resp.windows[0].key, "gemini-5h");
        assert!((resp.windows[0].utilization - 38.0).abs() < 1e-9);
        assert_eq!(resp.windows[0].resets_at, "2026-11-03T23:41:01Z");
        assert_eq!(resp.windows[1].key, "gemini-weekly");
        assert!((resp.windows[1].utilization - 88.6).abs() < 1e-9);
        assert_eq!(resp.windows[1].resets_at, "2026-11-07T20:44:03Z");
    }

    #[test]
    fn missing_windows_are_not_inferred_and_zero_remaining_is_valid() {
        let raw: QuotaSummaryResp = serde_json::from_str(r#"{
          "groups": [{ "buckets": [
            {"bucketId":"gemini-5h","window":"5h","resetTime":"2030-01-01T00:00:00Z"},
            {"bucketId":"gemini-5h","window":"5h","remainingFraction":0.5,"resetTime":""},
            {"bucketId":"gemini-daily","window":"daily","remainingFraction":0.5,"resetTime":"2030-01-01T00:00:00Z"},
            {"bucketId":"gemini-weekly","window":"weekly","remainingFraction":0,"resetTime":"2030-01-01T00:00:00Z"}
          ]}]
        }"#).unwrap();
        let resp = map_summary_to_response(&raw);
        assert_eq!(resp.windows.len(), 1);
        assert_eq!(resp.windows[0].key, "gemini-weekly");
        assert_eq!(resp.windows[0].utilization, 100.0);
        let empty = serde_json::from_str(r#"{"groups":[]}"#).unwrap();
        assert!(map_summary_to_response(&empty).windows.is_empty());
    }
}
