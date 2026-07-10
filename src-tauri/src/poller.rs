use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter};

use crate::app_state::AppState;
use crate::commands::ProviderUpdatedPayload;

/// 주기 폴링을 백엔드 tokio 타이머에서 구동한다.
/// 웹뷰 setTimeout은 창이 비포커스/백그라운드일 때 스로틀링·freeze 되어
/// 실제 갱신 주기가 밀리므로, 갱신 cadence는 프론트가 아니라 여기서 통제한다.
pub fn spawn(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tick(&app, &state).await;
            // 설정 변경(주기)을 매 사이클 반영. 최소 30초로 하한.
            let secs = state.settings.load().refresh_interval_sec.max(30) as u64;
            tokio::time::sleep(Duration::from_secs(secs)).await;
        }
    });
}

async fn tick(app: &AppHandle, state: &Arc<AppState>) {
    // force=true: 예약 폴링은 프론트 주기와 백엔드 TTL이 같아 발생하던
    // 경계 스킵을 피하기 위해 신선도 캐시를 우회한다. 서버가 지정한
    // Retry-After 냉각(cooldown)은 fetch_one 내부에서 여전히 존중된다.
    let results = state.fetch_all(true).await;
    for (provider, snap) in results {
        let _ = app.emit(
            "usage:provider_updated",
            ProviderUpdatedPayload { provider, snapshot: snap },
        );
    }
}
