use std::collections::HashMap;
use std::sync::Arc;

use serde::Serialize;
use tauri::{Emitter, State};

use crate::app_state::{AppState, ProviderSnapshot};
use crate::autostart;
use crate::cli_refresher;
use crate::types::{Provider, Settings};

#[derive(Serialize, Clone)]
pub struct ProviderUpdatedPayload {
    pub provider: Provider,
    pub snapshot: ProviderSnapshot,
}

#[tauri::command]
pub async fn get_all_snapshots(
    state: State<'_, Arc<AppState>>,
) -> Result<HashMap<String, ProviderSnapshot>, String> {
    Ok(state.current_snapshots().await)
}

#[tauri::command]
pub async fn get_provider_usage(
    state: State<'_, Arc<AppState>>,
    app: tauri::AppHandle,
    provider: Provider,
    force: Option<bool>,
) -> Result<ProviderSnapshot, String> {
    let force = force.unwrap_or(false);
    let _ = app.emit(
        "usage:refreshing",
        serde_json::json!({ "provider": provider, "manual": force }),
    );
    let snap = state.fetch_one(provider, force).await;
    let _ = app.emit(
        "usage:provider_updated",
        ProviderUpdatedPayload { provider, snapshot: snap.clone() },
    );
    Ok(snap)
}

#[tauri::command]
pub async fn refresh_all_in_background(
    state: State<'_, Arc<AppState>>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let results = state.fetch_all(false).await;
    for (provider, snap) in results {
        let _ = app.emit(
            "usage:provider_updated",
            ProviderUpdatedPayload { provider, snapshot: snap },
        );
    }
    Ok(())
}

#[tauri::command]
pub async fn refresh_via_cli(
    provider: Provider,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    cli_refresher::refresh_via_cli(provider).await.map_err(|e| e.to_string())?;
    state.cache.invalidate(provider);
    Ok(())
}

#[tauri::command]
pub async fn get_settings(state: State<'_, Arc<AppState>>) -> Result<Settings, String> {
    Ok(state.settings.load())
}

#[tauri::command]
pub async fn save_settings(
    settings: Settings,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    state.settings.save(&settings).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn set_autostart(enabled: bool) -> Result<(), String> {
    autostart::set(enabled).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn open_url(url: String) -> Result<(), String> {
    webbrowser::open(&url).map_err(|e| e.to_string())
}
