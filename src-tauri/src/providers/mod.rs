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
