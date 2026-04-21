mod app_state;
mod autostart;
mod cache;
mod cli_refresher;
mod commands;
mod errors;
mod poller;
mod providers;
mod settings;
mod state_store;
mod types;

use app_state::AppState;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};

fn save_window_rect(window: &tauri::Window) {
    use std::sync::Arc;
    let app = window.app_handle();
    let state = match app.try_state::<Arc<AppState>>() {
        Some(s) => s,
        None => return,
    };
    let scale = window.scale_factor().unwrap_or(1.0);
    let pos = match window.outer_position() {
        Ok(p) => p,
        Err(_) => return,
    };
    let size = match window.inner_size() {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut settings = state.settings.load();
    settings.window.x = (pos.x as f64 / scale).round() as i32;
    settings.window.y = (pos.y as f64 / scale).round() as i32;
    settings.window.width = (size.width as f64 / scale).round() as u32;
    settings.window.height = (size.height as f64 / scale).round() as u32;
    let _ = state.settings.save(&settings);
}

fn toggle_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        if w.is_visible().unwrap_or(false) {
            let _ = w.hide();
        } else {
            let _ = w.show();
            let _ = w.set_focus();
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("app_data_dir resolution");
            std::fs::create_dir_all(&data_dir).ok();

            let state = AppState::new(data_dir);
            app.manage(state.clone());

            let settings = state.settings.load();
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.set_always_on_top(settings.always_on_top);
                let _ = win.set_size(tauri::Size::Logical(tauri::LogicalSize {
                    width: settings.window.width as f64,
                    height: settings.window.height as f64,
                }));
                let _ = win.set_position(tauri::Position::Logical(tauri::LogicalPosition {
                    x: settings.window.x as f64,
                    y: settings.window.y as f64,
                }));
            }

            let show_i = MenuItem::with_id(app, "show", "보이기", true, None::<&str>)?;
            let hide_i = MenuItem::with_id(app, "hide", "숨기기", true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", "종료", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_i, &hide_i, &quit_i])?;

            TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Claude Usage Widget")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "hide" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.hide();
                        }
                    }
                    "quit" => {
                        if let Some(w) = app.get_webview_window("main") {
                            save_window_rect(&w.as_ref().window());
                        }
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_window(tray.app_handle());
                    }
                })
                .build(app)?;

            let _ = state;
            let _ = settings;
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut t = tokio::time::interval(std::time::Duration::from_secs(3));
                t.tick().await;
                loop {
                    t.tick().await;
                    if let Some(w) = app_handle.get_webview_window("main") {
                        if w.is_visible().unwrap_or(false) {
                            save_window_rect(&w.as_ref().window());
                        }
                    }
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" { return; }
            match event {
                WindowEvent::CloseRequested { api, .. } => {
                    save_window_rect(window);
                    use std::sync::Arc;
                    let to_tray = window
                        .app_handle()
                        .try_state::<Arc<AppState>>()
                        .map(|s| s.settings.load().close_to_tray)
                        .unwrap_or(false);
                    if to_tray {
                        let _ = window.hide();
                        api.prevent_close();
                    }
                }
                WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
                    save_window_rect(window);
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_all_snapshots,
            commands::get_provider_usage,
            commands::refresh_all_in_background,
            commands::refresh_via_cli,
            commands::get_settings,
            commands::save_settings,
            commands::set_autostart,
            commands::open_url,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
