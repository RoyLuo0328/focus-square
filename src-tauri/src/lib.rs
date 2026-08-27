mod ai;
mod database;
mod models;
mod report;
mod timer;

use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use chrono::Utc;
use models::{
    AiAnalysis, AiConfig, AppSettings, ReportData, ReportPeriod, TimerSnapshot, TimerStatus,
};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, PhysicalPosition, RunEvent, State,
    WebviewUrl, WebviewWindowBuilder, WindowEvent,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};
use tauri_plugin_notification::NotificationExt;

use crate::{database::Database, timer::TimerEngine};

struct AppState {
    database: Database,
    timer: Mutex<TimerEngine>,
    saved_position: Mutex<Option<PhysicalPosition<i32>>>,
    exiting: AtomicBool,
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn snapshot(state: &AppState) -> Result<TimerSnapshot, String> {
    state
        .timer
        .lock()
        .map_err(|error| error.to_string())
        .map(|timer| timer.snapshot(now_ms()))
}

fn emit_snapshot(app: &AppHandle, value: &TimerSnapshot) {
    let _ = app.emit("timer://state", value);
}

fn emit_error(app: &AppHandle, error: impl Into<String>) {
    let _ = app.emit("app://error", error.into());
}

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn restore_compact(app: &AppHandle) {
    let state = app.state::<AppState>();
    let always_on_top = state
        .timer
        .lock()
        .map(|timer| timer.settings().always_on_top)
        .unwrap_or(false);
    let position = state
        .saved_position
        .lock()
        .ok()
        .and_then(|mut value| value.take());
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_size(LogicalSize::new(260.0, 260.0));
        if let Some(position) = position {
            let _ = window.set_position(position);
        }
        let _ = window.set_always_on_top(always_on_top);
    }
}

fn show_completion(app: &AppHandle, completion: timer::Completion) {
    let state = app.state::<AppState>();
    let settings = state
        .timer
        .lock()
        .map(|timer| timer.settings().clone())
        .unwrap_or_default();
    if let Some(window) = app.get_webview_window("main") {
        if let Ok(position) = window.outer_position() {
            if let Ok(mut saved) = state.saved_position.lock() {
                *saved = Some(position);
            }
        }
        let _ = window.set_size(LogicalSize::new(420.0, 420.0));
        let _ = window.center();
        let _ = window.set_always_on_top(true);
        let _ = window.show();
        let _ = window.set_focus();
    }

    let chinese = settings.locale == "zh-CN"
        || (settings.locale == "system"
            && std::env::var("LANG")
                .unwrap_or_default()
                .to_lowercase()
                .starts_with("zh"));
    let finished_focus = completion.finished_mode == models::TimerMode::Focus;
    let title = if chinese {
        if finished_focus {
            "专注结束"
        } else {
            "休息结束"
        }
    } else if finished_focus {
        "Focus complete"
    } else {
        "Break complete"
    };
    let body = if chinese {
        if finished_focus {
            "该休息一下了。"
        } else {
            "准备好开始下一轮专注了吗？"
        }
    } else if finished_focus {
        "Time for a break."
    } else {
        "Ready for another focus session?"
    };

    if settings.notifications_enabled {
        let _ = app.notification().builder().title(title).body(body).show();
    }
    if settings.sound_enabled {
        let _ = app.emit("timer://sound", ());
    }
    let _ = app.emit("timer://completed", completion.next_mode);
}

fn schedule_completion(app: AppHandle, generation: u64) {
    tauri::async_runtime::spawn(async move {
        loop {
            let wait_ms = {
                let state = app.state::<AppState>();
                let Ok(timer) = state.timer.lock() else {
                    return;
                };
                if timer.generation() != generation {
                    return;
                }
                let Some(end_at) = timer.end_at_epoch_ms() else {
                    return;
                };
                end_at.saturating_sub(now_ms()).max(0) as u64
            };
            if wait_ms > 0 {
                tokio::time::sleep(Duration::from_millis(wait_ms)).await;
            }
            let result = {
                let state = app.state::<AppState>();
                let Ok(mut timer) = state.timer.lock() else {
                    return;
                };
                timer.complete_if_due(&state.database, now_ms(), generation)
            };
            match result {
                Ok(Some(completion)) => {
                    if let Ok(value) = snapshot(&app.state::<AppState>()) {
                        emit_snapshot(&app, &value);
                    }
                    show_completion(&app, completion);
                    return;
                }
                Ok(None) => continue,
                Err(error) => {
                    emit_error(&app, error);
                    return;
                }
            }
        }
    });
}

#[tauri::command]
fn get_timer(state: State<'_, AppState>) -> Result<TimerSnapshot, String> {
    snapshot(&state)
}

#[tauri::command]
fn start_timer(app: AppHandle, state: State<'_, AppState>) -> Result<TimerSnapshot, String> {
    let (generation, value) = {
        let mut timer = state.timer.lock().map_err(|error| error.to_string())?;
        let generation = timer.start(&state.database, now_ms())?;
        (generation, timer.snapshot(now_ms()))
    };
    emit_snapshot(&app, &value);
    schedule_completion(app, generation);
    Ok(value)
}

#[tauri::command]
fn pause_timer(app: AppHandle, state: State<'_, AppState>) -> Result<TimerSnapshot, String> {
    let value = {
        let mut timer = state.timer.lock().map_err(|error| error.to_string())?;
        timer.pause(&state.database, now_ms())?;
        timer.snapshot(now_ms())
    };
    emit_snapshot(&app, &value);
    Ok(value)
}

#[tauri::command]
fn reset_timer(app: AppHandle, state: State<'_, AppState>) -> Result<TimerSnapshot, String> {
    let value = {
        let mut timer = state.timer.lock().map_err(|error| error.to_string())?;
        timer.reset(&state.database, now_ms())?;
        timer.snapshot(now_ms())
    };
    restore_compact(&app);
    emit_snapshot(&app, &value);
    Ok(value)
}

#[tauri::command]
fn advance_timer(app: AppHandle, state: State<'_, AppState>) -> Result<TimerSnapshot, String> {
    let (generation, value) = {
        let mut timer = state.timer.lock().map_err(|error| error.to_string())?;
        let generation = timer.advance(&state.database, now_ms())?;
        (generation, timer.snapshot(now_ms()))
    };
    restore_compact(&app);
    emit_snapshot(&app, &value);
    schedule_completion(app, generation);
    Ok(value)
}

#[tauri::command]
fn defer_timer(app: AppHandle, state: State<'_, AppState>) -> Result<TimerSnapshot, String> {
    let value = {
        let mut timer = state.timer.lock().map_err(|error| error.to_string())?;
        timer.defer();
        timer.snapshot(now_ms())
    };
    restore_compact(&app);
    emit_snapshot(&app, &value);
    Ok(value)
}

#[tauri::command]
fn update_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<TimerSnapshot, String> {
    settings.validate()?;
    state.database.save_settings(&settings)?;
    let value = {
        let mut timer = state.timer.lock().map_err(|error| error.to_string())?;
        timer.update_settings(settings.clone())?;
        timer.snapshot(now_ms())
    };
    if value.status != TimerStatus::Completed {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.set_always_on_top(settings.always_on_top);
        }
    }
    emit_snapshot(&app, &value);
    Ok(value)
}

#[tauri::command]
fn open_analytics(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("analytics") {
        window.show().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        return Ok(());
    }
    WebviewWindowBuilder::new(
        &app,
        "analytics",
        WebviewUrl::App("index.html?view=analytics".into()),
    )
    .title("Focus Square · Reports")
    .inner_size(900.0, 680.0)
    .min_inner_size(760.0, 560.0)
    .resizable(true)
    .center()
    .build()
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn requested_locale(configured: &str, requested: Option<String>) -> String {
    requested
        .filter(|locale| matches!(locale.as_str(), "zh-CN" | "en"))
        .unwrap_or_else(|| configured.to_string())
}

#[tauri::command]
fn build_report(
    state: State<'_, AppState>,
    period: ReportPeriod,
    locale: Option<String>,
) -> Result<ReportData, String> {
    let configured = state
        .timer
        .lock()
        .map_err(|error| error.to_string())?
        .settings()
        .locale
        .clone();
    let locale = requested_locale(&configured, locale);
    report::build_report(&state.database, period, &locale)
}

#[tauri::command]
fn delete_focus_session(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let timer = state.timer.lock().map_err(|error| error.to_string())?;
    if timer.active_session_id() == Some(id.as_str()) {
        return Err("Reset the active focus session before deleting it".into());
    }
    drop(timer);
    state.database.delete_session(&id)
}

#[tauri::command]
fn clear_focus_sessions(state: State<'_, AppState>) -> Result<(), String> {
    let timer = state.timer.lock().map_err(|error| error.to_string())?;
    if timer.active_session_id().is_some() {
        return Err("Reset the active focus session before clearing history".into());
    }
    drop(timer);
    state.database.clear_history()
}

#[tauri::command]
fn get_ai_config(state: State<'_, AppState>) -> Result<AiConfig, String> {
    let timer = state.timer.lock().map_err(|error| error.to_string())?;
    Ok(ai::config(timer.settings()))
}

#[tauri::command]
fn save_ai_key(state: State<'_, AppState>, api_key: String) -> Result<AiConfig, String> {
    ai::save_api_key(&api_key)?;
    let timer = state.timer.lock().map_err(|error| error.to_string())?;
    Ok(ai::config(timer.settings()))
}

#[tauri::command]
fn delete_ai_key(state: State<'_, AppState>) -> Result<AiConfig, String> {
    ai::delete_api_key()?;
    let timer = state.timer.lock().map_err(|error| error.to_string())?;
    Ok(ai::config(timer.settings()))
}

#[tauri::command]
async fn test_ai_connection(state: State<'_, AppState>) -> Result<String, String> {
    let settings = state
        .timer
        .lock()
        .map_err(|error| error.to_string())?
        .settings()
        .clone();
    ai::test_connection(&settings).await
}

#[tauri::command]
async fn generate_ai_analysis(
    state: State<'_, AppState>,
    period: ReportPeriod,
    locale: Option<String>,
) -> Result<AiAnalysis, String> {
    let mut settings = state
        .timer
        .lock()
        .map_err(|error| error.to_string())?
        .settings()
        .clone();
    settings.locale = requested_locale(&settings.locale, locale);
    let report = report::build_report(&state.database, period, &settings.locale)?;
    ai::generate(&state.database, &settings, &report).await
}

fn tray_toggle(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let status = state
        .timer
        .lock()
        .map_err(|error| error.to_string())?
        .snapshot(now_ms())
        .status;
    match status {
        TimerStatus::Running => pause_timer(app.clone(), state).map(|_| ()),
        TimerStatus::Completed => advance_timer(app.clone(), state).map(|_| ()),
        TimerStatus::Idle | TimerStatus::Paused => start_timer(app.clone(), state).map(|_| ()),
    }
}

fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示 Focus Square / Show", true, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", "开始 / 暂停", true, None::<&str>)?;
    let reset = MenuItem::with_id(app, "reset", "重置 / Reset", true, None::<&str>)?;
    let reports = MenuItem::with_id(app, "reports", "报告 / Reports", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 / Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &toggle, &reset, &reports, &quit])?;
    let mut builder = TrayIconBuilder::with_id("focus-square")
        .menu(&menu)
        .tooltip("Focus Square")
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main(app),
            "toggle" => {
                if let Err(error) = tray_toggle(app) {
                    emit_error(app, error);
                }
            }
            "reset" => {
                let state = app.state::<AppState>();
                if let Err(error) = reset_timer(app.clone(), state) {
                    emit_error(app, error);
                }
            }
            "reports" => {
                if let Err(error) = open_analytics(app.clone()) {
                    emit_error(app, error);
                }
            }
            "quit" => {
                let state = app.state::<AppState>();
                state.exiting.store(true, Ordering::SeqCst);
                if let Ok(mut timer) = state.timer.lock() {
                    let _ = timer.finalize_on_exit(&state.database, now_ms());
                }
                app.exit(0);
            }
            _ => {}
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    Ok(())
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let database_path = app
                .path()
                .app_data_dir()
                .map_err(std::io::Error::other)?
                .join("focus-square.db");
            let database = Database::open(&database_path).map_err(std::io::Error::other)?;
            database
                .finalize_running_sessions(now_ms())
                .map_err(std::io::Error::other)?;
            let settings = database.load_settings().map_err(std::io::Error::other)?;
            let always_on_top = settings.always_on_top;
            app.manage(AppState {
                database,
                timer: Mutex::new(TimerEngine::new(settings)),
                saved_position: Mutex::new(None),
                exiting: AtomicBool::new(false),
            });
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_always_on_top(always_on_top);
                let _ = window.set_position(LogicalPosition::new(80.0, 80.0));
            }
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            setup_tray(app.handle())?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                if !state.exiting.load(Ordering::SeqCst) {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_timer,
            start_timer,
            pause_timer,
            reset_timer,
            advance_timer,
            defer_timer,
            update_settings,
            open_analytics,
            build_report,
            delete_focus_session,
            clear_focus_sessions,
            get_ai_config,
            save_ai_key,
            delete_ai_key,
            test_ai_connection,
            generate_ai_analysis
        ])
        .build(tauri::generate_context!())
        .expect("error while building Focus Square");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            let state = app.state::<AppState>();
            if let Ok(mut timer) = state.timer.lock() {
                let _ = timer.finalize_on_exit(&state.database, now_ms());
            }
        }
    });
}
