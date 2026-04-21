use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdout, Command};
use tokio::time::timeout;

use crate::errors::{AppError, AppResult};
use crate::types::{Provider, Status, UsageResponse, UsageWindow};

#[derive(Deserialize, Debug)]
pub(crate) struct PrimaryOrSecondary {
    #[serde(rename = "usedPercent")]
    pub used_percent: Option<f64>,
    #[serde(rename = "windowDurationMins")]
    pub window_duration_mins: Option<u64>,
    #[serde(rename = "resetsAt")]
    pub resets_at: Option<i64>,
}

#[derive(Deserialize, Debug)]
pub(crate) struct Bucket {
    #[serde(rename = "limitId")]
    pub limit_id: Option<String>,
    #[serde(rename = "limitName")]
    pub limit_name: Option<String>,
    pub primary: Option<PrimaryOrSecondary>,
    pub secondary: Option<PrimaryOrSecondary>,
}

#[derive(Deserialize, Debug)]
pub(crate) struct RateLimitsResult {
    #[serde(rename = "rateLimitsByLimitId", default)]
    pub rate_limits_by_limit_id: std::collections::HashMap<String, Bucket>,
}

fn window_name(dur_mins: u64) -> String {
    let hours = dur_mins / 60;
    if hours >= 24 {
        format!("{}일", hours / 24)
    } else {
        format!("{}시간", hours)
    }
}

fn iso_from_epoch(sec: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(sec, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
}

fn compute_time_progress(resets_epoch: i64, duration_sec: u64) -> f64 {
    let now = chrono::Utc::now().timestamp();
    let start = resets_epoch - duration_sec as i64;
    if now <= start { return 0.0; }
    if now >= resets_epoch { return 100.0; }
    ((now - start) as f64 / duration_sec as f64 * 100.0).round()
}

fn push_window(out: &mut Vec<UsageWindow>, key: &str, pw: &PrimaryOrSecondary) {
    let (util, dur_mins, reset_sec) = match (pw.used_percent, pw.window_duration_mins, pw.resets_at) {
        (Some(u), Some(d), Some(r)) => (u, d, r),
        _ => return,
    };
    let dur_sec = dur_mins * 60;
    out.push(UsageWindow {
        key: key.to_string(),
        name: window_name(dur_mins),
        utilization: util,
        resets_at: iso_from_epoch(reset_sec),
        time_progress: compute_time_progress(reset_sec, dur_sec),
    });
}

pub(crate) fn map_to_response(result: &RateLimitsResult) -> UsageResponse {
    let mut windows = Vec::new();
    let mut buckets: Vec<&Bucket> = result.rate_limits_by_limit_id.values().collect();
    buckets.sort_by_key(|b| b.limit_id.clone().unwrap_or_default());
    for b in buckets {
        let base = b.limit_id.clone().unwrap_or_else(|| "unknown".into());
        if let Some(p) = &b.primary { push_window(&mut windows, &format!("{}_primary", base), p); }
        if let Some(s) = &b.secondary { push_window(&mut windows, &format!("{}_secondary", base), s); }
    }
    UsageResponse {
        provider: Provider::Codex,
        status: Status::Ok,
        windows,
        extra_usage: None,
        error: None,
    }
}

async fn read_response_by_id(
    reader: &mut tokio::io::Lines<BufReader<ChildStdout>>,
    target_id: u64,
    deadline: Duration,
) -> AppResult<serde_json::Value> {
    let fut = async {
        loop {
            let line = reader
                .next_line()
                .await
                .map_err(|e| AppError::Other(format!("read stdout: {}", e)))?;
            let line = match line {
                Some(l) => l,
                None => return Err(AppError::Other("codex stdout closed".into())),
            };
            let trimmed = line.trim();
            if trimmed.is_empty() { continue; }
            let msg: serde_json::Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if msg.get("id").and_then(|v| v.as_u64()) == Some(target_id) {
                if let Some(err) = msg.get("error") {
                    return Err(AppError::Other(format!("codex rpc error: {}", err)));
                }
                if let Some(result) = msg.get("result") {
                    return Ok(result.clone());
                }
                return Err(AppError::Other("codex response missing result".into()));
            }
        }
    };
    timeout(deadline, fut)
        .await
        .map_err(|_| AppError::Other("codex rpc timed out".into()))?
}

pub async fn fetch() -> AppResult<UsageResponse> {
    let codex_bin = if cfg!(windows) { "codex.cmd" } else { "codex" };
    let mut cmd = Command::new(codex_bin);
    cmd.args(["app-server", "-c", "sandbox=\"danger-full-access\""])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let mut child = cmd.spawn().map_err(|e| {
        AppError::NotAuthenticated(format!("codex CLI not found on PATH: {}", e))
    })?;

    let mut stdin = child.stdin.take().ok_or_else(|| AppError::Other("no stdin".into()))?;
    let stdout = child.stdout.take().ok_or_else(|| AppError::Other("no stdout".into()))?;
    let mut reader = BufReader::new(stdout).lines();

    let init_req = r#"{"method":"initialize","params":{"clientInfo":{"name":"claude-usage-widget","version":"0.1.0"}},"id":1}"#;
    let init_notif = r#"{"method":"initialized","params":{}}"#;
    let rl_req = r#"{"method":"account/rateLimits/read","params":{},"id":2}"#;

    stdin.write_all(format!("{}\n", init_req).as_bytes()).await.map_err(AppError::Io)?;
    // wait for init response before sending initialized (matches bridge flow)
    let _ = read_response_by_id(&mut reader, 1, Duration::from_secs(10)).await?;
    stdin.write_all(format!("{}\n", init_notif).as_bytes()).await.map_err(AppError::Io)?;
    stdin.write_all(format!("{}\n", rl_req).as_bytes()).await.map_err(AppError::Io)?;

    let result = read_response_by_id(&mut reader, 2, Duration::from_secs(10)).await?;

    let _ = child.start_kill();
    let _ = child.wait().await;

    let parsed: RateLimitsResult = serde_json::from_value(result.clone()).map_err(|e| {
        AppError::Other(format!("codex rateLimits parse failed: {} | raw: {}", e, result))
    })?;

    if parsed.rate_limits_by_limit_id.is_empty() {
        return Ok(UsageResponse {
            provider: Provider::Codex,
            status: Status::Ok,
            windows: vec![],
            extra_usage: None,
            error: None,
        });
    }

    Ok(map_to_response(&parsed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_primary_and_secondary() {
        let mut map = std::collections::HashMap::new();
        map.insert(
            "plan-a".to_string(),
            Bucket {
                limit_id: Some("plan-a".into()),
                limit_name: Some("Plan A".into()),
                primary: Some(PrimaryOrSecondary {
                    used_percent: Some(20.0),
                    window_duration_mins: Some(300),
                    resets_at: Some(4_000_000_000),
                }),
                secondary: Some(PrimaryOrSecondary {
                    used_percent: Some(55.0),
                    window_duration_mins: Some(10_080),
                    resets_at: Some(4_000_000_000),
                }),
            },
        );
        let result = RateLimitsResult { rate_limits_by_limit_id: map };
        let resp = map_to_response(&result);
        assert_eq!(resp.windows.len(), 2);
        assert_eq!(resp.windows[0].name, "5시간");
        assert_eq!(resp.windows[1].name, "7일");
    }
}
