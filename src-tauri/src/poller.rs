use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter};
use tokio::time::interval;

use crate::app_state::AppState;
use crate::commands::ProviderUpdatedPayload;

pub fn spawn(app: AppHandle, state: Arc<AppState>, interval_sec: u64) {
    tauri::async_runtime::spawn(async move {
        tick(&app, &state).await;

        let mut t = interval(Duration::from_secs(interval_sec.max(30)));
        t.tick().await;
        loop {
            t.tick().await;
            tick(&app, &state).await;
        }
    });
}

async fn tick(app: &AppHandle, state: &Arc<AppState>) {
    let results = state.fetch_all(false).await;
    for (provider, snap) in results {
        let _ = app.emit(
            "usage:provider_updated",
            ProviderUpdatedPayload { provider, snapshot: snap },
        );
    }
}
