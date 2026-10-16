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
    #[serde(default, rename = "displayName")]
    display_name: String,
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

// fetchAvailableModels — only used as a fallback for the Gemini row when the
// summary omits the Gemini group (seen 2026-10-16: summary returned only 3p-*).
// Every Gemini model carries the same pooled fraction, and its resetTime
// matched the summary's gemini-5h bucket, so it maps to the 5-hour row.

#[derive(Deserialize)]
struct AvailableModelsResp {
    #[serde(default)]
    models: std::collections::HashMap<String, ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    #[serde(default, rename = "displayName")]
    display_name: Option<String>,
    #[serde(default, rename = "quotaInfo")]
    quota_info: Option<ModelQuotaInfo>,
}

#[derive(Deserialize)]
struct ModelQuotaInfo {
    #[serde(default, rename = "remainingFraction")]
    remaining_fraction: Option<f64>,
    #[serde(default, rename = "resetTime")]
    reset_time: Option<String>,
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
// One row per (group, window): Gemini 5h/weekly, then Claude·GPT 5h/weekly.
// The bucketId ("gemini-5h", "3p-weekly", ...) is used as the window key.
// Unknown groups fall back to the server's group displayName.

const FIVE_HOUR_SEC: u64 = 5 * 60 * 60;
const WEEK_SEC: u64 = 7 * DAY_SEC;

fn group_label(bucket_id: &str, group_name: &str) -> String {
    if bucket_id.starts_with("gemini-") { "Gemini".into() }
    else if bucket_id.starts_with("3p-") { "Claude·GPT".into() }
    else { group_name.to_string() }
}

fn map_summary_to_response(raw: &QuotaSummaryResp) -> UsageResponse {
    let mut windows = Vec::new();
    for group in &raw.groups {
        // 5h before weekly within each group, regardless of server order.
        let mut buckets: Vec<&SummaryBucket> = group.buckets.iter().collect();
        buckets.sort_by_key(|b| if b.window == "5h" { 0 } else { 1 });
        for b in buckets {
            let (frac, reset) = match (b.remaining_fraction, &b.reset_time) {
                (Some(f), Some(r)) => (f, r.clone()),
                _ => continue,
            };
            let (suffix, duration) = match b.window.as_str() {
                "5h" => ("5시간", FIVE_HOUR_SEC),
                "weekly" => ("7일", WEEK_SEC),
                _ => continue,
            };
            windows.push(UsageWindow {
                key: b.bucket_id.clone(),
                name: format!("{} {}", group_label(&b.bucket_id, &group.display_name), suffix),
                utilization: ((1.0 - frac) * 100.0).round(),
                time_progress: compute_time_progress(&reset, duration),
                resets_at: reset,
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

fn has_gemini_group(resp: &UsageResponse) -> bool {
    resp.windows.iter().any(|w| w.key.starts_with("gemini-"))
}

// Most-consumed Gemini model (with a displayName) → "Gemini 5시간" row.
fn gemini_row_from_models(raw: &AvailableModelsResp) -> Option<UsageWindow> {
    let (frac, reset) = raw
        .models
        .iter()
        .filter(|(id, e)| id.starts_with("gemini-") && e.display_name.is_some())
        .filter_map(|(_, e)| {
            let q = e.quota_info.as_ref()?;
            Some((q.remaining_fraction?, q.reset_time.clone()?))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))?;
    Some(UsageWindow {
        key: "gemini-5h".into(),
        name: "Gemini 5시간".into(),
        utilization: ((1.0 - frac) * 100.0).round(),
        time_progress: compute_time_progress(&reset, FIVE_HOUR_SEC),
        resets_at: reset,
    })
}

// Prefer whichever summary carries the Gemini group; otherwise any successful
// one (prod first). Both failed → prod's error.
fn select_summary(prod: AppResult<UsageResponse>, daily: AppResult<UsageResponse>) -> AppResult<UsageResponse> {
    match (prod, daily) {
        (Ok(p), _) if has_gemini_group(&p) => Ok(p),
        (_, Ok(d)) if has_gemini_group(&d) => Ok(d),
        (Ok(p), _) => Ok(p),
        (Err(_), Ok(d)) => Ok(d),
        (Err(e), Err(_)) => Err(e),
    }
}

// === HTTP helpers ===

const SUMMARY_PROD_URL: &str = "https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary";
const SUMMARY_DAILY_URL: &str = "https://daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary";

async fn fetch_summary(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    body: &serde_json::Value,
) -> AppResult<UsageResponse> {
    let res = client
        .post(url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .header("User-Agent", "antigravity")
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
    let raw: QuotaSummaryResp = res.json().await?;
    Ok(map_summary_to_response(&raw))
}

async fn fetch_antigravity(token: &str) -> AppResult<UsageResponse> {
    let client = reqwest::Client::new();

    // 1. loadCodeAssist → cloudaicompanionProject
    let lca_body = serde_json::json!({
        "metadata": { "ideType": "ANTIGRAVITY", "platform": "PLATFORM_UNSPECIFIED", "pluginType": "GEMINI" }
    });
    let lca_res = client
        .post("https://daily-cloudcode-pa.googleapis.com/v1internal:loadCodeAssist")
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .header("User-Agent", "antigravity")
        .body(lca_body.to_string())
        .send()
        .await?;
    let lca_status = lca_res.status();
    if lca_status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(AppError::Expired);
    }
    if !lca_status.is_success() {
        let body = lca_res.text().await.unwrap_or_default();
        return Err(AppError::Api { status: lca_status.as_u16(), message: body });
    }
    let lca: LoadCodeAssistResp = lca_res.json().await?;
    let project_id = match lca.project {
        Some(serde_json::Value::String(s)) => Some(s),
        Some(serde_json::Value::Object(o)) => o.get("id").and_then(|v| v.as_str()).map(String::from),
        _ => None,
    };

    // 2. retrieveUserQuotaSummary — prod host first. Since 2026-10-16 the daily
    //    host drops the Gemini group while prod still returns it (matching the
    //    agy console), so daily is only a backup.
    let body = match &project_id {
        Some(p) => serde_json::json!({ "project": p }),
        None => serde_json::json!({}),
    };
    let prod = fetch_summary(&client, SUMMARY_PROD_URL, token, &body).await;
    let mut resp = match prod {
        Ok(r) if has_gemini_group(&r) => r,
        prod => {
            let daily = fetch_summary(&client, SUMMARY_DAILY_URL, token, &body).await;
            select_summary(prod, daily)?
        }
    };

    // 3. Neither summary had a Gemini group → fill it from per-model quota.
    //    A failure here keeps the summary rows rather than failing the whole fetch.
    if !has_gemini_group(&resp) {
        let models = client
            .post("https://daily-cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels")
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .header("User-Agent", "antigravity")
            .body(body.to_string())
            .send()
            .await;
        if let Ok(r) = models {
            if r.status().is_success() {
                if let Ok(raw) = r.json::<AvailableModelsResp>().await {
                    if let Some(row) = gemini_row_from_models(&raw) {
                        resp.windows.insert(0, row);
                    }
                }
            }
        }
    }
    Ok(resp)
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
    fn summary_maps_groups_to_5h_and_weekly_rows() {
        let raw: QuotaSummaryResp = serde_json::from_str(r#"{
          "groups": [
            { "displayName": "Gemini Models", "buckets": [
              { "bucketId": "gemini-weekly", "window": "weekly", "resetTime": "2030-01-08T00:00:00Z", "remainingFraction": 0.982088 },
              { "bucketId": "gemini-5h", "window": "5h", "resetTime": "2030-01-01T05:00:00Z", "remainingFraction": 0.9383521 }
            ]},
            { "displayName": "Claude and GPT models", "buckets": [
              { "bucketId": "3p-weekly", "window": "weekly", "resetTime": "2030-01-08T00:00:00Z", "remainingFraction": 1 },
              { "bucketId": "3p-5h", "window": "5h", "resetTime": "2030-01-01T05:00:00Z", "remainingFraction": 0.5 }
            ]}
          ]
        }"#).unwrap();
        let resp = map_summary_to_response(&raw);

        let rows: Vec<_> = resp.windows.iter().map(|w| (w.key.as_str(), w.name.as_str(), w.utilization)).collect();
        assert_eq!(rows, vec![
            ("gemini-5h", "Gemini 5시간", 6.0),
            ("gemini-weekly", "Gemini 7일", 2.0),
            ("3p-5h", "Claude·GPT 5시간", 50.0),
            ("3p-weekly", "Claude·GPT 7일", 0.0),
        ]);
    }

    #[test]
    fn summary_skips_incomplete_and_unknown_window_buckets() {
        let raw: QuotaSummaryResp = serde_json::from_str(r#"{
          "groups": [
            { "displayName": "New Group", "buckets": [
              { "bucketId": "new-5h", "window": "5h", "resetTime": "2030-01-01T05:00:00Z", "remainingFraction": 0.25 },
              { "bucketId": "new-daily", "window": "daily", "resetTime": "2030-01-02T00:00:00Z", "remainingFraction": 0.5 },
              { "bucketId": "new-weekly", "window": "weekly" }
            ]}
          ]
        }"#).unwrap();
        let resp = map_summary_to_response(&raw);
        assert_eq!(resp.windows.len(), 1);
        assert_eq!(resp.windows[0].name, "New Group 5시간");
        assert_eq!(resp.windows[0].utilization, 75.0);
    }

    #[test]
    fn gemini_row_falls_back_to_most_consumed_gemini_model() {
        let raw: AvailableModelsResp = serde_json::from_str(r#"{
          "models": {
            "gemini-3-flash": { "displayName": "Gemini 3 Flash", "quotaInfo": { "remainingFraction": 0.6937189, "resetTime": "2030-01-01T05:00:00Z" } },
            "gemini-2.5-pro": { "displayName": "Gemini 2.5 Pro", "quotaInfo": { "remainingFraction": 0.8, "resetTime": "2030-01-01T05:00:00Z" } },
            "gemini-3.8-flash-tiered": { "quotaInfo": { "remainingFraction": 0.1, "resetTime": "2030-01-01T05:00:00Z" } },
            "claude-opus-5-5-high": { "displayName": "Claude Opus 5.5 (High)", "quotaInfo": { "remainingFraction": 0.0, "resetTime": "2030-01-01T05:00:00Z" } }
          }
        }"#).unwrap();
        let row = gemini_row_from_models(&raw).unwrap();
        assert_eq!(row.key, "gemini-5h");
        assert_eq!(row.name, "Gemini 5시간");
        assert_eq!(row.utilization, 31.0);
    }

    #[test]
    fn gemini_row_absent_without_gemini_quota() {
        let raw: AvailableModelsResp = serde_json::from_str(r#"{
          "models": { "gemini-3-flash": { "displayName": "Gemini 3 Flash" } }
        }"#).unwrap();
        assert!(gemini_row_from_models(&raw).is_none());
    }

    #[test]
    fn summary_without_gemini_group_is_detected() {
        let raw: QuotaSummaryResp = serde_json::from_str(r#"{
          "groups": [ { "displayName": "Claude and GPT models", "buckets": [
            { "bucketId": "3p-5h", "window": "5h", "resetTime": "2030-01-01T05:00:00Z", "remainingFraction": 1 }
          ]} ]
        }"#).unwrap();
        assert!(!has_gemini_group(&map_summary_to_response(&raw)));
    }

    fn summary_with(bucket_ids: &[&str]) -> UsageResponse {
        let buckets: Vec<String> = bucket_ids
            .iter()
            .map(|id| format!(r#"{{ "bucketId": "{id}", "window": "5h", "resetTime": "2030-01-01T05:00:00Z", "remainingFraction": 1 }}"#))
            .collect();
        let json = format!(r#"{{ "groups": [ {{ "displayName": "G", "buckets": [ {} ] }} ] }}"#, buckets.join(","));
        map_summary_to_response(&serde_json::from_str(&json).unwrap())
    }

    #[test]
    fn select_summary_prefers_one_with_gemini_group() {
        let r = select_summary(Ok(summary_with(&["3p-5h"])), Ok(summary_with(&["gemini-5h", "3p-5h"]))).unwrap();
        assert!(has_gemini_group(&r));
        let r = select_summary(Ok(summary_with(&["gemini-5h"])), Ok(summary_with(&["3p-5h"]))).unwrap();
        assert!(has_gemini_group(&r));
    }

    #[test]
    fn select_summary_falls_back_to_any_success_then_prod_error() {
        let r = select_summary(Err(AppError::Expired), Ok(summary_with(&["3p-5h"]))).unwrap();
        assert_eq!(r.windows[0].key, "3p-5h");
        let r = select_summary(Ok(summary_with(&["3p-5h"])), Err(AppError::Expired)).unwrap();
        assert_eq!(r.windows[0].key, "3p-5h");
        let e = select_summary(Err(AppError::Expired), Err(AppError::Api { status: 500, message: String::new() }));
        assert!(matches!(e, Err(AppError::Expired)));
    }
}
