use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tokio::process::Command;
use tokio::time::timeout;

use crate::errors::{AppError, AppResult};
use crate::types::Provider;

const SPAWN_TIMEOUT: Duration = Duration::from_secs(15);

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
    (light, full)
}

async fn run_with_timeout(mut cmd: Command) -> AppResult<std::process::ExitStatus> {
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let child = cmd.spawn().map_err(|e| AppError::Other(format!("spawn failed: {}", e)))?;
    let mut child = child;
    let status = timeout(SPAWN_TIMEOUT, child.wait())
        .await
        .map_err(|_| AppError::Other("cli spawn timed out".into()))??;
    Ok(status)
}

pub async fn refresh_via_cli(provider: Provider) -> AppResult<()> {
    let path = token_path(provider);
    let before = mtime(&path);

    let (light, full) = commands(provider);

    let _ = run_with_timeout(light).await.ok();
    let after_light = mtime(&path);
    if after_light != before && after_light.is_some() {
        return Ok(());
    }

    let status = run_with_timeout(full).await?;
    let after_full = mtime(&path);
    if after_full != before && after_full.is_some() {
        return Ok(());
    }
    if !status.success() {
        return Err(AppError::Other(format!(
            "cli exited non-zero and token file did not change (provider: {})",
            provider.as_str()
        )));
    }
    Ok(())
}
