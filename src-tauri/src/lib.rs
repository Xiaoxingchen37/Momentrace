mod app_icon;
mod store;
mod tracker;

use crate::{
    store::{
        today, AllSummary, AppRule, AppRulePatch, Category, DaySummary, LiveStatus, RangeSummary,
        Settings, Store, WidgetStatus,
    },
    tracker::Tracker,
};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, RunEvent, State, WebviewUrl,
    WebviewWindowBuilder,
};
#[cfg(windows)]
use windows61::Win32::{
    Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST},
    UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, HWND_TOPMOST,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_SHOWWINDOW,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
    },
};

struct AppState {
    store: Arc<Store>,
    tracker: Arc<Tracker>,
    icons: parking_lot::Mutex<HashMap<String, String>>,
    widget_menu: parking_lot::Mutex<Option<MenuItem<tauri::Wry>>>,
    main_window_creating: AtomicBool,
}

#[tauri::command]
fn get_live_status(state: State<AppState>) -> LiveStatus {
    state.tracker.status()
}
#[tauri::command]
fn get_day_summary(date: String, state: State<AppState>) -> Result<DaySummary, String> {
    state.store.day_summary(&date)
}
#[tauri::command]
fn get_widget_status(state: State<AppState>) -> Result<WidgetStatus, String> {
    state.tracker.widget_status()
}
#[tauri::command]
fn get_range_summary(
    start_date: String,
    end_date: String,
    state: State<AppState>,
) -> Result<RangeSummary, String> {
    state.store.range_summary(&start_date, &end_date)
}
#[tauri::command]
fn get_all_summary(state: State<AppState>) -> Result<AllSummary, String> {
    state.store.all_summary()
}
#[tauri::command]
fn get_categories(state: State<AppState>) -> Result<Vec<Category>, String> {
    state.store.categories()
}
#[tauri::command]
fn create_category(name: String, color: String, state: State<AppState>) -> Result<(), String> {
    state.store.create_category(&name, &color).map(|_| ())
}
#[tauri::command]
fn delete_category(id: i64, state: State<AppState>) -> Result<(), String> {
    state.store.delete_category(id)
}
#[tauri::command]
fn get_app_rules(state: State<AppState>) -> Result<Vec<AppRule>, String> {
    state.store.rules()
}
#[tauri::command]
fn update_app_rule(
    executable: String,
    patch: AppRulePatch,
    state: State<AppState>,
) -> Result<(), String> {
    state.store.update_rule(&executable, &patch)
}
#[tauri::command]
fn get_settings(state: State<AppState>) -> Result<Settings, String> {
    state.store.settings()
}
#[tauri::command]
fn update_settings(
    settings: Settings,
    app: AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    sync_autostart(&app, settings.launch_at_login)?;
    state.store.update_settings(&settings)?;
    let saved_settings = state.store.settings()?;
    state.tracker.set_idle_minutes(settings.idle_minutes);
    set_widget_visibility(&app, settings.show_widget);
    let _ = app.emit("font-family-changed", saved_settings.font_family);
    Ok(())
}
#[tauri::command]
fn set_tracking_paused(paused: bool, state: State<AppState>) {
    state.tracker.set_manual_pause(paused)
}
#[tauri::command]
fn clear_usage_data(state: State<AppState>) -> Result<(), String> {
    state.tracker.clear_usage()
}
#[tauri::command]
fn get_today() -> String {
    today()
}
#[tauri::command]
fn get_app_icon(process_path: String, state: State<AppState>) -> Result<String, String> {
    if let Some(icon) = state.icons.lock().get(&process_path).cloned() {
        return Ok(icon);
    }
    let icon = app_icon::from_executable(&process_path)?;
    state.icons.lock().insert(process_path, icon.clone());
    Ok(icon)
}

#[tauri::command]
fn show_main_window(app: AppHandle) {
    open_main_window(&app)
}

fn open_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        return;
    }
    let state = app.state::<AppState>();
    if state
        .main_window_creating
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let handle = app.clone();
    std::thread::spawn(move || {
        let result =
            WebviewWindowBuilder::new(&handle, "main", WebviewUrl::App("index.html".into()))
                .title("时迹 · Momentrace")
                .inner_size(1200.0, 800.0)
                .min_inner_size(900.0, 650.0)
                .build();
        if let Err(err) = result {
            eprintln!("Unable to create overview window: {err}");
        }
        handle
            .state::<AppState>()
            .main_window_creating
            .store(false, Ordering::Release);
    });
}

fn sync_autostart(app: &AppHandle, enabled: bool) -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Ok(());
    }
    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    if enabled {
        manager.enable().map_err(|err| err.to_string())
    } else {
        manager.disable().map_err(|err| err.to_string())
    }
}

#[cfg(windows)]
fn widget_position(
    window: &tauri::WebviewWindow,
    height: i32,
    x_offset: i32,
    y_offset: i32,
) -> Option<PhysicalPosition<i32>> {
    let hwnd = window.hwnd().ok()?;
    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return None;
        }
        let monitor_rect = info.rcMonitor;
        let work_rect = info.rcWork;
        let bottom_taskbar = work_rect.bottom < monitor_rect.bottom;
        let top_taskbar = work_rect.top > monitor_rect.top;
        let taskbar_height = if bottom_taskbar {
            monitor_rect.bottom - work_rect.bottom
        } else if top_taskbar {
            work_rect.top - monitor_rect.top
        } else {
            0
        };
        let x = work_rect.left + x_offset;
        let vertical_offset = 12;
        let y = if bottom_taskbar {
            work_rect.bottom + ((taskbar_height - height).max(0) / 2) - vertical_offset
        } else if top_taskbar {
            monitor_rect.top + ((taskbar_height - height).max(0) / 2) + vertical_offset
        } else {
            work_rect.bottom - height - vertical_offset
        };
        Some(PhysicalPosition::new(x, y + y_offset))
    }
}

#[cfg(not(windows))]
fn widget_position(
    _: &tauri::WebviewWindow,
    _: i32,
    _: i32,
    _: i32,
) -> Option<PhysicalPosition<i32>> {
    None
}

#[cfg(windows)]
fn reinforce_widget_layer(window: &tauri::WebviewWindow) {
    let Ok(hwnd) = window.hwnd() else {
        return;
    };
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let click_through_style = style
            | WS_EX_NOACTIVATE.0 as isize
            | WS_EX_TRANSPARENT.0 as isize
            | WS_EX_TOOLWINDOW.0 as isize;
        let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, click_through_style);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_SHOWWINDOW,
        );
    }
}

#[cfg(not(windows))]
fn reinforce_widget_layer(_: &tauri::WebviewWindow) {}

fn anchor_widget_to_taskbar(app: &AppHandle) {
    let Some(window) = app.get_webview_window("widget") else {
        return;
    };
    let settings = app.state::<AppState>().store.settings().ok();
    let height = settings.as_ref().map_or(64, |value| value.widget_height);
    let x_offset = settings.as_ref().map_or(0, |value| value.widget_x_offset);
    let y_offset = settings.as_ref().map_or(0, |value| value.widget_y_offset);
    let scale = window.scale_factor().unwrap_or(1.0);
    let _ = window.set_size(LogicalSize::new(292.0, height as f64));
    let physical_height = (height as f64 * scale).round() as i32;
    let physical_x_offset = (x_offset as f64 * scale).round() as i32;
    let physical_y_offset = (y_offset as f64 * scale).round() as i32;
    if let Some(position) = widget_position(
        &window,
        physical_height,
        physical_x_offset,
        physical_y_offset,
    ) {
        let _ = window.set_position(position);
    }
    let _ = window.set_shadow(false);
    let _ = window.set_focusable(false);
    let _ = window.set_ignore_cursor_events(true);
    let _ = window.set_always_on_top(true);
    reinforce_widget_layer(&window);
}

fn set_widget_visibility(app: &AppHandle, visible: bool) {
    let Some(window) = app.get_webview_window("widget") else {
        return;
    };
    if visible {
        anchor_widget_to_taskbar(app);
        let _ = window.show();
        anchor_widget_to_taskbar(app);
    } else {
        let _ = window.hide();
    }
    let state = app.state::<AppState>();
    if let Some(item) = state.widget_menu.lock().as_ref() {
        let _ = item.set_text(if visible {
            "关闭悬浮窗"
        } else {
            "显示悬浮窗"
        });
    }
    let _ = app.emit("widget-visibility", visible);
}

fn set_widget_preference(app: &AppHandle, visible: bool) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut settings = state.store.settings()?;
    settings.show_widget = visible;
    state.store.update_settings(&settings)?;
    set_widget_visibility(app, visible);
    Ok(())
}
#[tauri::command]
fn set_widget_visible(visible: bool, app: AppHandle) -> Result<(), String> {
    set_widget_preference(&app, visible)
}

fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "打开概览", true, None::<&str>)?;
    let widget_visible = app
        .get_webview_window("widget")
        .is_some_and(|window| window.is_visible().unwrap_or(false));
    let widget = MenuItem::with_id(
        app,
        "widget",
        if widget_visible {
            "关闭悬浮窗"
        } else {
            "显示悬浮窗"
        },
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &widget, &quit])?;
    app.state::<AppState>()
        .widget_menu
        .lock()
        .replace(widget.clone());
    TrayIconBuilder::with_id("main-tray")
        .icon(
            app.default_window_icon()
                .cloned()
                .ok_or_else(|| tauri::Error::AssetNotFound("default application icon".into()))?,
        )
        .menu(&menu)
        .tooltip("时迹 · Momentrace")
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "open" => open_main_window(app),
            "widget" => {
                let visible = app
                    .get_webview_window("widget")
                    .is_some_and(|window| window.is_visible().unwrap_or(false));
                let _ = set_widget_preference(app, !visible);
            }
            "quit" => {
                let state = app.state::<AppState>();
                state.tracker.flush();
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                ..
            } = event
            {
                let _ = set_widget_preference(tray.app_handle(), true);
            }
        })
        .build(app)?;
    Ok(())
}
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            open_main_window(app)
        }))
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name("Momentrace")
                .build(),
        )
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let store = Store::open(&data_dir)?;
            let tracker = Tracker::new(Arc::clone(&store));
            tracker.start();
            let settings = store.settings().unwrap_or(Settings {
                idle_minutes: 5,
                launch_at_login: false,
                show_widget: true,
                font_family: "classic".into(),
                widget_height: 64,
                widget_x_offset: 0,
                widget_y_offset: 0,
            });
            app.manage(AppState {
                store,
                tracker,
                icons: parking_lot::Mutex::new(HashMap::new()),
                widget_menu: parking_lot::Mutex::new(None),
                main_window_creating: AtomicBool::new(false),
            });
            if let Err(err) = sync_autostart(app.handle(), settings.launch_at_login) {
                eprintln!("Unable to synchronize autostart: {err}");
            }
            if settings.show_widget {
                set_widget_visibility(app.handle(), true);
            }
            setup_tray(app.handle())?;
            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                let widget_visible = handle
                    .get_webview_window("widget")
                    .is_some_and(|window| window.is_visible().unwrap_or(false));
                let main_visible = handle
                    .get_webview_window("main")
                    .is_some_and(|window| window.is_visible().unwrap_or(false));
                if widget_visible || main_visible {
                    let state = handle.state::<AppState>();
                    let _ = handle.emit("tracker-status", state.tracker.status());
                }
                if widget_visible {
                    anchor_widget_to_taskbar(&handle);
                }
                std::thread::sleep(Duration::from_secs(5));
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_live_status,
            get_day_summary,
            get_widget_status,
            get_range_summary,
            get_all_summary,
            get_categories,
            create_category,
            delete_category,
            get_app_rules,
            update_app_rule,
            get_settings,
            update_settings,
            set_tracking_paused,
            clear_usage_data,
            get_today,
            get_app_icon,
            set_widget_visible,
            show_main_window
        ])
        .build(tauri::generate_context!())
        .expect("error while running Momentrace")
        .run(|app, event| {
            if let RunEvent::ExitRequested { .. } = event {
                app.state::<AppState>().tracker.flush();
            }
        });
}
