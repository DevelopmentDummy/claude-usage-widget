pub mod antigravity_cred;
pub mod claude;
pub mod codex;
pub mod gemini;

use crate::errors::AppResult;
use crate::types::{Provider, UsageResponse};

pub async fn fetch(provider: Provider) -> AppResult<UsageResponse> {
    match provider {
        Provider::Claude => claude::fetch().await,
        Provider::Codex => codex::fetch().await,
        Provider::Gemini => gemini::fetch().await,
    }
}

/// Claude primary(/api/oauth/usage)가 429일 때 쓰는 rate-limit 헤더 폴백.
pub async fn fetch_claude_ratelimit_headers() -> AppResult<UsageResponse> {
    claude::fetch_ratelimit_headers().await
}
