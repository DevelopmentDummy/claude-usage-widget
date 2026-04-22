use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tokio::process::Command;
use tokio::time::timeout;

use crate::errors::{AppError, AppResult};
use crate::types::Provider;

const SPAWN_TIMEOUT: Duration = Duration::from_secs(30);

fn augmented_path() -> Option<String> {
    let mut parts: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs::home_dir() {
        if cfg!(windows) {
            if let Some(appdata) = std::env::var_os("APPDATA") {
                parts.push(PathBuf::from(appdata).join("npm"));
            }
            parts.push(home.join("AppData").join("Roaming").join("npm"));
            parts.push(home.join(".bun").join("bin"));
            parts.push(home.join(".volta").join("bin"));
        } else {
            parts.push(home.join(".npm-global").join("bin"));
            parts.push(home.join(".bun").join("bin"));
            parts.push(home.join(".volta").join("bin"));
            parts.push(PathBuf::from("/usr/local/bin"));
            parts.push(PathBuf::from("/opt/homebrew/bin"));
        }
    }
    let existing = std::env::var_os("PATH").unwrap_or_default();
    let sep = if cfg!(windows) { ";" } else { ":" };
    let extras: Vec<String> = parts
        .into_iter()
        .filter(|p| p.exists())
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    if extras.is_empty() {
        return None;
    }
    let mut s = extras.join(sep);
    if !existing.is_empty() {
        s.push_str(sep);
        s.push_str(&existing.to_string_lossy());
    }
    Some(s)
}

fn token_path(provider: Provider) -> PathBuf {
    let home = dirs::home_dir().unwrap_or_default();
    match provider {
        Provider::Claude => home.join(".claude").join(".credentials.json"),
        Provider::Codex => home.join(".codex").join("auth.json"),
        Provider::Gemini => home.join(".gemini").join("oauth_creds.json"),
    }
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn bin_name(base: &str) -> String {
    if cfg!(windows) { format!("{}.cmd", base) } else { base.to_string() }
}

fn commands(provider: Provider) -> (Command, Command) {
    let prompt = "Reply with exactly: hi. No other text.";
    let base = match provider {
        Provider::Claude => "claude",
        Provider::Gemini => "gemini",
        Provider::Codex => "codex",
    };
    let name = bin_name(base);
    let (light_args, full_args): (&[&str], Vec<&str>) = match provider {
        Provider::Claude | Provider::Gemini => (&["--version"], vec!["-p", prompt]),
        Provider::Codex => (&["--version"], vec!["exec", prompt]),
    };
    let mut light = Command::new(&name);
    light.args(light_args);
    let mut full = Command::new(&name);
    full.args(&full_args);
    if let Some(p) = augmented_path() {
        light.env("PATH", &p);
        full.env("PATH", &p);
    }
    (light, full)
}

async fn run_with_timeout(mut cmd: Command) -> AppResult<(std::process::ExitStatus, String)> {
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let child = cmd
        .spawn()
        .map_err(|e| AppError::Other(format!("spawn failed: {}", e)))?;
    let output = timeout(SPAWN_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| AppError::Other("cli spawn timed out".into()))??;
    let mut tail = String::from_utf8_lossy(&output.stderr).into_owned();
    if tail.trim().is_empty() {
        tail = String::from_utf8_lossy(&output.stdout).into_owned();
    }
    let tail = tail.trim().chars().rev().take(240).collect::<String>();
    let tail: String = tail.chars().rev().collect();
    Ok((output.status, tail))
}

pub async fn refresh_via_cli(provider: Provider) -> AppResult<()> {
    let path = token_path(provider);
    let before = mtime(&path);

    let (light, full) = commands(provider);

    let light_result = run_with_timeout(light).await;
    let after_light = mtime(&path);
    if after_light != before && after_light.is_some() {
        return Ok(());
    }

    let (status, tail) = match run_with_timeout(full).await {
        Ok(v) => v,
        Err(e) => {
            let light_msg = match light_result {
                Ok((s, t)) => format!("light exit={:?} tail={}", s.code(), t),
                Err(e2) => format!("light err={}", e2),
            };
            return Err(AppError::Other(format!(
                "{} ({}; provider={})",
                e,
                light_msg,
                provider.as_str()
            )));
        }
    };
    let after_full = mtime(&path);
    if after_full != before && after_full.is_some() {
        return Ok(());
    }
    if !status.success() {
        return Err(AppError::Other(format!(
            "cli exit={:?} tail={} (provider={})",
            status.code(),
            tail,
            provider.as_str()
        )));
    }
    Err(AppError::Other(format!(
        "cli ran ok but token file unchanged (provider={}, tail={})",
        provider.as_str(),
        tail
    )))
}
