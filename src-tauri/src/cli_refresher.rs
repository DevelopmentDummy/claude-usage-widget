use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tokio::process::Command;
use tokio::time::timeout;

use crate::errors::{AppError, AppResult};
use crate::types::Provider;

const SPAWN_TIMEOUT: Duration = Duration::from_secs(30);

macro_rules! logln {
    ($($arg:tt)*) => {{
        crate::diag::log("cli_refresher", &format!($($arg)*));
    }};
}

fn augmented_path() -> Option<String> {
    let mut parts: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs::home_dir() {
        if cfg!(windows) {
            if let Some(appdata) = std::env::var_os("APPDATA") {
                parts.push(PathBuf::from(appdata).join("npm"));
            }
            if let Some(localappdata) = std::env::var_os("LOCALAPPDATA") {
                parts.push(PathBuf::from(localappdata).join("agy").join("bin"));
            }
            parts.push(home.join("AppData").join("Roaming").join("npm"));
            parts.push(home.join("AppData").join("Local").join("agy").join("bin"));
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

/// Snapshot of all token sources for a provider. agy refresh only mutates the
/// wincred entry (Windows), so file mtime alone misses successful refreshes.
#[derive(PartialEq, Eq, Debug)]
struct TokenState {
    mtime: Option<SystemTime>,
    wincred_hash: Option<u64>,
}

fn token_state(provider: Provider) -> TokenState {
    let mtime = mtime(&token_path(provider));
    let wincred_hash = if matches!(provider, Provider::Gemini) && cfg!(windows) {
        crate::providers::antigravity_cred::read_blob_hash("gemini:antigravity")
    } else {
        None
    };
    TokenState { mtime, wincred_hash }
}

fn token_refreshed(before: &TokenState, after: &TokenState) -> bool {
    if after.mtime != before.mtime && after.mtime.is_some() {
        return true;
    }
    if after.wincred_hash != before.wincred_hash && after.wincred_hash.is_some() {
        return true;
    }
    false
}

fn resolve_bin(base: &str, path_override: Option<&str>) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) {
        &[".cmd", ".exe", ".bat", ".ps1", ""]
    } else {
        &[""]
    };
    let path_str = match path_override {
        Some(s) => s.to_string(),
        None => std::env::var("PATH").unwrap_or_default(),
    };
    let sep = if cfg!(windows) { ';' } else { ':' };
    for dir in path_str.split(sep) {
        if dir.is_empty() { continue; }
        for ext in exts {
            let candidate = PathBuf::from(dir).join(format!("{}{}", base, ext));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn build_cmd(bin: &Path, args: &[&str], path_env: Option<&str>) -> Command {
    if cfg!(windows) {
        let ext = bin
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase());
        if matches!(ext.as_deref(), Some("cmd") | Some("bat")) {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(bin);
            for a in args { c.arg(a); }
            if let Some(p) = path_env { c.env("PATH", p); }
            return c;
        }
    }
    let mut c = Command::new(bin);
    for a in args { c.arg(a); }
    if let Some(p) = path_env { c.env("PATH", p); }
    c
}

fn commands(provider: Provider) -> (Option<Command>, Option<Command>, String) {
    let prompt = "Reply with exactly: hi. No other text.";
    let bases: &[&str] = match provider {
        Provider::Claude => &["claude"],
        Provider::Gemini => &["agy", "gemini"],
        Provider::Codex => &["codex"],
    };
    let (light_args, full_args): (Vec<&str>, Vec<&str>) = match provider {
        // 토큰 리프레시는 인증된 호출 한 번만 트리거하면 되므로 가장 싼 모델(haiku)로 스폰한다.
        Provider::Claude => (vec!["--version"], vec!["-p", prompt, "--model", "haiku"]),
        Provider::Gemini => (vec!["--version"], vec!["-p", prompt]),
        Provider::Codex => (vec!["--version"], vec!["exec", prompt]),
    };
    let path_env = augmented_path();
    let resolved = bases
        .iter()
        .find_map(|b| resolve_bin(b, path_env.as_deref()));
    let resolved_str = match &resolved {
        Some(p) => p.display().to_string(),
        None => format!("<not found: {}>", bases.join(",")),
    };
    let (light, full) = match resolved {
        Some(bin) => (
            Some(build_cmd(&bin, &light_args, path_env.as_deref())),
            Some(build_cmd(&bin, &full_args, path_env.as_deref())),
        ),
        None => (None, None),
    };
    (light, full, resolved_str)
}

async fn run_with_timeout(mut cmd: Command) -> AppResult<(std::process::ExitStatus, String)> {
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    logln!("spawning: {:?}", cmd.as_std());
    let child = cmd
        .spawn()
        .map_err(|e| {
            logln!("spawn failed: {}", e);
            AppError::Other(format!("spawn failed: {}", e))
        })?;
    let output = timeout(SPAWN_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| {
            logln!("cli spawn timed out after {:?}", SPAWN_TIMEOUT);
            AppError::Other("cli spawn timed out".into())
        })??;
    let stderr_full = String::from_utf8_lossy(&output.stderr).into_owned();
    let stdout_full = String::from_utf8_lossy(&output.stdout).into_owned();
    logln!(
        "exit={:?} stdout_len={} stderr_len={}",
        output.status.code(),
        stdout_full.len(),
        stderr_full.len()
    );
    if !stdout_full.trim().is_empty() {
        logln!("stdout: {}", stdout_full.trim());
    }
    if !stderr_full.trim().is_empty() {
        logln!("stderr: {}", stderr_full.trim());
    }
    let mut tail = stderr_full;
    if tail.trim().is_empty() {
        tail = stdout_full;
    }
    let tail = tail.trim().chars().rev().take(240).collect::<String>();
    let tail: String = tail.chars().rev().collect();
    Ok((output.status, tail))
}

pub async fn refresh_via_cli(provider: Provider) -> AppResult<()> {
    let path = token_path(provider);
    let before = token_state(provider);
    logln!(
        "refresh_via_cli start provider={} token_path={} exists={} state={:?} PATH_aug={:?}",
        provider.as_str(),
        path.display(),
        path.exists(),
        before,
        augmented_path()
    );

    let (light, full, resolved) = commands(provider);
    logln!("resolved bin: {}", resolved);
    let (light, full) = match (light, full) {
        (Some(l), Some(f)) => (l, f),
        _ => {
            logln!("FAIL provider={} — bin not found on PATH", provider.as_str());
            return Err(AppError::Other(format!(
                "cli not found on PATH (provider={}; resolved={})",
                provider.as_str(),
                resolved
            )));
        }
    };

    logln!("running light probe (--version)");
    let light_result = run_with_timeout(light).await;
    if let Err(ref e) = light_result {
        logln!("light probe err: {}", e);
    }
    let after_light = token_state(provider);
    if token_refreshed(&before, &after_light) {
        logln!("token updated by light probe — done (after={:?})", after_light);
        return Ok(());
    }
    logln!("light probe did not refresh token; running full prompt");

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
    let after_full = token_state(provider);
    let changed = token_refreshed(&before, &after_full);
    logln!(
        "full run exit={:?} after={:?} changed={}",
        status.code(),
        after_full,
        changed
    );
    if changed {
        return Ok(());
    }
    if !status.success() {
        logln!(
            "FAIL provider={} exit={:?} tail={}",
            provider.as_str(),
            status.code(),
            tail
        );
        return Err(AppError::Other(format!(
            "cli exit={:?} tail={} (provider={})",
            status.code(),
            tail,
            provider.as_str()
        )));
    }
    logln!(
        "FAIL provider={} — cli exit 0 but token unchanged",
        provider.as_str()
    );
    Err(AppError::Other(format!(
        "cli ran ok but token file unchanged (provider={}, tail={})",
        provider.as_str(),
        tail
    )))
}
