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
    // 활성 탭 provider만 주기 갱신한다(README의 "활성 탭만 주기 조회" 동작).
    // 예전엔 매 사이클 3개 provider를 전부 force 조회해서, Codex/Gemini를 보고
    // 있어도 Claude가 계속 호출돼 usage 엔드포인트 한도를 갉아먹었다. 비활성
    // provider는 디스크 스냅샷을 보여주고, 탭 전환 시 프론트가 직접 당겨온다.
    //
    // force=true: 프론트 주기와 백엔드 TTL이 같아 발생하던 경계 스킵을 피하려
    // 신선도 캐시를 우회한다. Retry-After 냉각과 최소요청간격은 fetch_one이 존중.
    let provider = state.active_provider().await;
    let snap = state.fetch_one(provider, true).await;
    let _ = app.emit(
        "usage:provider_updated",
        ProviderUpdatedPayload { provider, snapshot: snap },
    );
}
