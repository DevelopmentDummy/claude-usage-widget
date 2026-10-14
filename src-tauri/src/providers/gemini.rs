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

// === Antigravity loadCodeAssist + fetchAvailableModels schemas ===

#[derive(Deserialize)]
struct LoadCodeAssistResp {
    #[serde(default, rename = "cloudaicompanionProject")]
    project: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct AvailableModelsResp {
    #[serde(default)]
    models: std::collections::HashMap<String, ModelEntry>,
    #[serde(default, rename = "deprecatedModelIds")]
    deprecated_model_ids: Vec<String>,
}

#[derive(Deserialize, Default)]
struct ModelEntry {
    #[serde(default, rename = "displayName")]
    display_name: Option<String>,
    #[serde(default, rename = "quotaInfo")]
    quota_info: Option<QuotaInfo>,
    #[serde(default, rename = "isInternal")]
    is_internal: bool,
    #[serde(default)]
    disabled: bool,
    #[serde(default, rename = "isDisabled")]
    is_disabled: bool,
}

#[derive(Deserialize)]
struct QuotaInfo {
    #[serde(default, rename = "remainingFraction")]
    remaining_fraction: Option<f64>,
    #[serde(default, rename = "resetTime")]
    reset_time: Option<String>,
}

// === Family classification (shared) ===
// Fixed display order: Flash, Pro, (other Gemini families e.g. Argon), Claude, GPT-OSS.
// flash-lite collapses into "flash"; "other" is unmapped (e.g., agent helpers).

fn has_token(s: &str, tok: &str) -> bool {
    s.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|t| t == tok)
}

fn model_tier(model_id: &str) -> &'static str {
    if model_id.starts_with("claude-") { "claude" }
    else if model_id.starts_with("gpt-oss") || model_id.starts_with("gpt-") { "gpt" }
    else if has_token(model_id, "flash") { "flash" }
    else if has_token(model_id, "pro") { "pro" }
    else { "other" }
}

/// Antigravity tier: model_id first, then displayName (ids are aliased and may
/// not carry the family, e.g. MODEL_PLACEHOLDER_*). Gemini models outside
/// Flash/Pro (e.g. "Gemini Argon") get their own family named from displayName.
fn antigravity_tier(model_id: &str, display: &str) -> (String, String) {
    let fixed = |k: &str| (k.to_string(), tier_label(k).to_string());
    match model_tier(model_id) {
        "other" => {}
        k => return fixed(k),
    }
    let d = display.to_ascii_lowercase();
    if d.starts_with("claude") { return fixed("claude"); }
    if d.starts_with("gpt") { return fixed("gpt"); }
    if has_token(display, "flash") { return fixed("flash"); }
    if has_token(display, "pro") { return fixed("pro"); }
    if model_id.starts_with("gemini") || d.starts_with("gemini") {
        // "Gemini 3.5 Argon Lite (High)" → "Argon" (variants collapse like flash-lite)
        let base = display.split('(').next().unwrap_or("");
        let name: Vec<&str> = base
            .split_whitespace()
            .filter(|w| !["gemini", "lite", "preview"].iter().any(|s| w.eq_ignore_ascii_case(s)))
            .filter(|w| w.chars().any(|c| c.is_ascii_alphabetic()) && !w.chars().next().unwrap().is_ascii_digit())
            .collect();
        if !name.is_empty() {
            let label = name.join(" ");
            let key = label.to_ascii_lowercase().replace(' ', "-");
            return (key, label);
        }
    }
    ("other".into(), "Other".into())
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

fn tier_rank(key: &str) -> u8 {
    match key {
        "flash" => 0,
        "pro" => 1,
        "claude" => 3,
        "gpt" => 4,
        _ => 2,
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

// === Antigravity mapping (fetchAvailableModels → family windows) ===
// For each family, pick the model variant with the lowest remainingFraction
// (most consumed — conservative view). Internal (no displayName / isInternal),
// disabled and deprecated models are skipped, so a family whose every variant
// is turned off (e.g. Pro) drops out of the view.

fn map_antigravity_to_response(raw: &AvailableModelsResp) -> UsageResponse {
    use std::collections::HashMap;
    // key → (label, frac, reset)
    let mut worst_per_tier: HashMap<String, (String, f64, String)> = HashMap::new();

    for (model_id, entry) in &raw.models {
        let display = match &entry.display_name {
            Some(d) if !entry.is_internal => d,
            _ => continue,
        };
        if entry.disabled || entry.is_disabled || raw.deprecated_model_ids.iter().any(|d| d == model_id) {
            continue;
        }
        let (frac, reset) = match &entry.quota_info {
            // proto3 JSON omits zero values: resetTime without remainingFraction = exhausted.
            Some(QuotaInfo { remaining_fraction, reset_time: Some(r) }) => (remaining_fraction.unwrap_or(0.0), r.clone()),
            _ => continue,
        };
        let (key, label) = antigravity_tier(model_id, display);
        if key == "other" { continue; }
        match worst_per_tier.get_mut(&key) {
            Some(existing) if frac < existing.1 => *existing = (label, frac, reset),
            Some(_) => {}
            None => { worst_per_tier.insert(key, (label, frac, reset)); }
        }
    }

    let mut entries: Vec<_> = worst_per_tier.into_iter().collect();
    entries.sort_by(|(a, _), (b, _)| tier_rank(a).cmp(&tier_rank(b)).then_with(|| a.cmp(b)));

    let windows = entries
        .into_iter()
        .map(|(key, (label, frac, reset))| {
            // resetTime cycle is variable per-model; estimate duration from the gap between now and reset,
            // bounded between 1h (sprint) and DAY_SEC. Falls back to DAY_SEC if parse fails.
            let duration = match chrono::DateTime::parse_from_rfc3339(&reset) {
                Ok(dt) => {
                    let gap = dt.timestamp() - chrono::Utc::now().timestamp();
                    if gap > 0 { (gap as u64).max(3600).min(DAY_SEC) } else { DAY_SEC }
                }
                Err(_) => DAY_SEC,
            };
            UsageWindow {
                key,
                name: label,
                utilization: ((1.0 - frac) * 100.0).round(),
                time_progress: compute_time_progress(&reset, duration),
                resets_at: reset,
            }
        })
        .collect();

    UsageResponse {
        provider: Provider::Gemini,
        status: Status::Ok,
        windows,
        extra_usage: None,
        error: None,
    }
}

/// Logs the model roster once whenever it changes, so new/removed models
/// (e.g. a new family or Pro being turned off) can be checked in diag.log.
fn log_model_roster(raw: &AvailableModelsResp) {
    use std::sync::Mutex;
    static LAST: Mutex<String> = Mutex::new(String::new());
    let mut rows: Vec<String> = raw
        .models
        .iter()
        .filter(|(_, e)| e.display_name.is_some())
        .map(|(id, e)| {
            let q = e.quota_info.as_ref();
            format!(
                "{}={:?}{}{}{}",
                id,
                e.display_name.as_deref().unwrap_or(""),
                if q.and_then(|q| q.remaining_fraction).is_none() { " no_frac" } else { "" },
                if e.disabled || e.is_disabled { " disabled" } else { "" },
                if raw.deprecated_model_ids.contains(id) { " deprecated" } else { "" },
            )
        })
        .collect();
    rows.sort();
    let summary = rows.join(", ");
    if let Ok(mut last) = LAST.lock() {
        if *last != summary {
            crate::diag::log("gemini", &format!("antigravity models: {}", summary));
            *last = summary;
        }
    }
}

// === HTTP helpers ===

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

    // 2. fetchAvailableModels
    let body = match &project_id {
        Some(p) => serde_json::json!({ "project": p }),
        None => serde_json::json!({}),
    };
    let res = client
        .post("https://daily-cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels")
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
    let raw: AvailableModelsResp = res.json().await?;
    log_model_roster(&raw);
    Ok(map_antigravity_to_response(&raw))
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
    // and gives richer model coverage (Gemini 3.x, preview variants).
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

    fn mk_entry(display: &str, frac: f64, reset: &str) -> ModelEntry {
        ModelEntry {
            display_name: Some(display.into()),
            quota_info: Some(QuotaInfo {
                remaining_fraction: Some(frac),
                reset_time: Some(reset.into()),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn antigravity_picks_worst_per_family() {
        let mut models = std::collections::HashMap::new();
        models.insert("gemini-3-flash".into(), mk_entry("Gemini 3 Flash", 0.9, "2030-01-01T00:00:00Z"));
        models.insert("gemini-3.5-flash-low".into(), mk_entry("Gemini 3.5 Flash (Medium)", 0.4, "2030-01-01T00:00:00Z"));
        models.insert("gemini-3.1-flash-lite".into(), mk_entry("Gemini 3.1 Flash Lite", 0.7, "2030-01-01T00:00:00Z"));
        models.insert("gemini-3.1-pro-high".into(), mk_entry("Gemini 3.1 Pro (High)", 0.6, "2030-01-01T00:00:00Z"));
        models.insert("gemini-3.1-pro-low".into(), mk_entry("Gemini 3.1 Pro (Low)", 0.3, "2030-01-01T00:00:00Z"));
        models.insert("claude-sonnet-4-6".into(), mk_entry("Claude Sonnet 4.6 (Thinking)", 0.2, "2030-01-01T00:00:00Z"));
        models.insert("claude-opus-4-6-thinking".into(), mk_entry("Claude Opus 4.6 (Thinking)", 0.5, "2030-01-01T00:00:00Z"));
        models.insert("gpt-oss-120b-medium".into(), mk_entry("GPT-OSS 120B (Medium)", 0.8, "2030-01-01T00:00:00Z"));
        models.insert("chat_23310".into(), ModelEntry::default());
        let raw = AvailableModelsResp { models, deprecated_model_ids: vec![] };
        let resp = map_antigravity_to_response(&raw);

        // 4 families: Flash, Pro, Claude, GPT-OSS
        assert_eq!(resp.windows.len(), 4);
        let by_key: std::collections::HashMap<_, _> = resp.windows.iter().map(|w| (w.key.as_str(), w)).collect();
        // flash: worst among gemini-3-flash (0.9), gemini-3.5-flash-low (0.4), gemini-3.1-flash-lite (0.7)
        assert_eq!(by_key["flash"].utilization, 60.0);
        // pro: worst between gemini-3.1-pro-high (0.6), gemini-3.1-pro-low (0.3)
        assert_eq!(by_key["pro"].utilization, 70.0);
        // claude: worst between sonnet (0.2), opus (0.5)
        assert_eq!(by_key["claude"].utilization, 80.0);
        // gpt: only gpt-oss-120b-medium (0.8)
        assert_eq!(by_key["gpt"].utilization, 20.0);

        // Order respects TIER_ORDER
        assert_eq!(resp.windows[0].key, "flash");
        assert_eq!(resp.windows[1].key, "pro");
        assert_eq!(resp.windows[2].key, "claude");
        assert_eq!(resp.windows[3].key, "gpt");
    }

    #[test]
    fn antigravity_new_family_and_disabled_pro() {
        let mut models = std::collections::HashMap::new();
        models.insert("gemini-3.5-flash-low".into(), mk_entry("Gemini 3.5 Flash (Medium)", 0.4, "2030-01-01T00:00:00Z"));
        models.insert("gemini-argon-high".into(), mk_entry("Gemini Argon (High)", 0.9, "2030-01-01T00:00:00Z"));
        models.insert("gemini-argon-low".into(), mk_entry("Gemini Argon (Low)", 0.5, "2030-01-01T00:00:00Z"));
        // Aliased id: family only visible in displayName
        models.insert("MODEL_PLACEHOLDER_M40".into(), mk_entry("Gemini 3.5 Argon Lite", 0.7, "2030-01-01T00:00:00Z"));
        let mut pro_high = mk_entry("Gemini 3.1 Pro (High)", 1.0, "2030-01-01T00:00:00Z");
        pro_high.disabled = true;
        models.insert("gemini-3.1-pro-high".into(), pro_high);
        models.insert("gemini-3.1-pro-low".into(), mk_entry("Gemini 3.1 Pro (Low)", 1.0, "2030-01-01T00:00:00Z"));
        let mut opus = mk_entry("Claude Opus 4.6 (Thinking)", 0.5, "2030-01-01T00:00:00Z");
        // proto3 omits remainingFraction == 0
        opus.quota_info.as_mut().unwrap().remaining_fraction = None;
        models.insert("claude-opus-4-6-thinking".into(), opus);
        let raw = AvailableModelsResp {
            models,
            deprecated_model_ids: vec!["gemini-3.1-pro-low".into()],
        };
        let resp = map_antigravity_to_response(&raw);

        let keys: Vec<_> = resp.windows.iter().map(|w| w.key.as_str()).collect();
        // Pro: high disabled + low deprecated → dropped entirely
        assert_eq!(keys, vec!["flash", "argon", "claude"]);
        assert_eq!(resp.windows[1].name, "Argon");
        // worst among argon-high (0.9), argon-low (0.5), Argon Lite (0.7)
        assert_eq!(resp.windows[1].utilization, 50.0);
        assert_eq!(resp.windows[2].utilization, 100.0);
    }
}
