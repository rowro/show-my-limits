mod capture_conflict;
mod certificate;
mod coordination;
#[cfg(any(target_os = "linux", test))]
mod desktop_state;
mod engine;
mod diagnostics;
#[cfg(target_os = "linux")]
mod gnome;
mod installer;
#[cfg(target_os = "linux")]
mod linux_panel;
#[cfg(target_os = "macos")]
mod macos_trust;
mod process_tree;
mod quota;
mod settings;
#[cfg(any(target_os = "linux", target_os = "windows", test))]
mod tray_image;
mod updates;
#[cfg(target_os = "windows")]
mod windows_taskbar;
#[cfg(target_os = "windows")]
mod windows_tray_text;
#[cfg(target_os = "windows")]
mod windows_trust;

use engine::{AppStatus, Settings, Shared};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

#[derive(Default)]
struct PopoverState {
    last_blur: Mutex<Option<Instant>>,
    anchor: Mutex<Option<tauri::Rect>>,
    system_dialog: AtomicBool,
    #[cfg(target_os = "linux")]
    dragging: AtomicBool,
}
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, State};

#[tauri::command]
fn start_window_drag(window: tauri::WebviewWindow) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        linux_panel::start_drag(window)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = window;
        Err("Перетаскивание доступно только на Linux.".into())
    }
}

#[cfg(not(target_os = "linux"))]
#[tauri::command]
fn gnome_status() -> Result<(), String> {
    Err("GNOME доступен на Linux.".into())
}
#[cfg(not(target_os = "linux"))]
#[tauri::command]
fn install_gnome_extension() -> Result<(), String> {
    Err("GNOME доступен на Linux.".into())
}
#[cfg(not(target_os = "linux"))]
#[tauri::command]
fn logout_gnome_session() -> Result<(), String> {
    Err("GNOME доступен на Linux.".into())
}
#[cfg(target_os = "linux")]
use gnome::{gnome_status, install_gnome_extension, logout_gnome_session};

#[tauri::command]
fn get_status(app: tauri::AppHandle, state: State<'_, Shared>) -> AppStatus {
    state.status(&app)
}

#[tauri::command]
async fn save_settings(app: tauri::AppHandle, settings: Settings) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Shared>().save_and_apply(&app, settings)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn set_launch_at_login(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    if enabled {
        manager.enable()
    } else {
        manager.disable()
    }
    .map_err(|e| e.to_string())?;
    manager.is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
fn complete_setup(app: tauri::AppHandle, state: State<'_, Shared>) -> Result<(), String> {
    state.complete_setup(&app)
}

#[tauri::command]
fn start_capture(app: tauri::AppHandle, state: State<'_, Shared>) -> Result<(), String> {
    state.start(&app)
}

#[tauri::command]
fn stop_capture(state: State<'_, Shared>) {
    state.stop();
}

#[tauri::command]
async fn prepare_certificate(app: tauri::AppHandle) -> Result<(), String> {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        handle.state::<Shared>().prepare_certificate(&handle)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn install_helper(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || app.state::<Shared>().install_helper(&app))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn install_certificate(app: tauri::AppHandle) -> Result<(), String> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let handle = app.clone();
    app.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window("main") {
            let _ = window.show();
            let _ = window.set_focus();
        }
        handle
            .state::<PopoverState>()
            .system_dialog
            .store(true, Ordering::SeqCst);
        let result = handle.state::<Shared>().install_certificate();
        handle
            .state::<PopoverState>()
            .system_dialog
            .store(false, Ordering::SeqCst);
        if let Some(window) = handle.get_webview_window("main") {
            if !window.is_focused().unwrap_or(false) {
                let _ = window.hide();
            }
        }
        let _ = sender.send(result);
    })
    .map_err(|e| e.to_string())?;
    let result = tauri::async_runtime::spawn_blocking(move || receiver.recv())
        .await
        .map_err(|e| e.to_string())?;
    result.map_err(|_| "Не удалось получить результат установки сертификата.".to_string())?
}

#[tauri::command]
fn open_certificate(app: tauri::AppHandle, state: State<'_, Shared>) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let path = state.paths.certificates.join("mitmproxy-ca-cert.cer");
    if !path.is_file() {
        return Err("Сначала создайте сертификат.".into());
    }
    app.opener()
        .open_path(path.display().to_string(), None::<String>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn reveal_certificate(app: tauri::AppHandle, state: State<'_, Shared>) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let path = state.paths.certificates.join("mitmproxy-ca-cert.cer");
    if !path.is_file() {
        return Err("Сначала создайте сертификат.".into());
    }
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

fn toggle_window(app: &tauri::AppHandle, rect: Option<tauri::Rect>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        return;
    }
    // Clicking the tray first blurs/hides the panel. Do not reopen it on the same click.
    let state = app.state::<PopoverState>();
    if state
        .last_blur
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .is_some_and(|at| at.elapsed() < Duration::from_millis(250))
    {
        return;
    }
    show_popover(app, rect);
}

fn show_popover(app: &tauri::AppHandle, rect: Option<tauri::Rect>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if let Some(anchor) = rect {
        *app.state::<PopoverState>()
            .anchor
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(anchor);
    }
    let anchor = *app
        .state::<PopoverState>()
        .anchor
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    position_popover(&window, anchor);
    let _ = window.show();
    let _ = window.set_focus();
}

fn contains_physical_point(origin: (i32, i32), size: (u32, u32), point: (f64, f64)) -> bool {
    point.0 >= origin.0 as f64
        && point.1 >= origin.1 as f64
        && point.0 < origin.0 as f64 + size.0 as f64
        && point.1 < origin.1 as f64 + size.1 as f64
}

fn popover_monitor(
    window: &tauri::WebviewWindow,
    rect: Option<tauri::Rect>,
) -> Option<tauri::Monitor> {
    if let Some(rect) = rect {
        if let Ok(monitors) = window.available_monitors() {
            for monitor in monitors {
                let scale = monitor.scale_factor();
                let point = rect.position.to_physical::<f64>(scale);
                let size = rect.size.to_physical::<f64>(scale);
                let origin = monitor.position();
                let screen = monitor.size();
                // Keep tray coordinates and monitor bounds in physical pixels.
                // monitor_from_point uses Cocoa points on macOS, unlike these APIs.
                if contains_physical_point(
                    (origin.x, origin.y),
                    (screen.width, screen.height),
                    (point.x + size.width / 2.0, point.y + size.height / 2.0),
                ) {
                    return Some(monitor);
                }
            }
        }
    }
    window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten())
}

fn panel_position(
    anchor: (f64, f64, f64, f64),
    area: (f64, f64, f64, f64),
    panel: (f64, f64),
) -> (i32, i32) {
    let (ax, ay, aw, ah) = anchor;
    let (left, top, width, height) = area;
    let x = (ax + aw / 2.0 - panel.0 / 2.0)
        .clamp(left + 8.0, (left + width - panel.0 - 8.0).max(left + 8.0));
    let below = ay + ah + 8.0;
    let y = if below + panel.1 <= top + height - 8.0 {
        below
    } else {
        ay - panel.1 - 8.0
    };
    let y = y.clamp(top + 8.0, (top + height - panel.1 - 8.0).max(top + 8.0));
    (x.round() as i32, y.round() as i32)
}

fn position_popover(window: &tauri::WebviewWindow, rect: Option<tauri::Rect>) {
    // Wayland compositors own placement; tray coordinates may be unavailable.
    // Preserve the user's chosen Linux position on resize and subsequent opens.
    if cfg!(target_os = "linux") {
        return;
    }
    let Some(rect) = rect else {
        return;
    };
    let Some(monitor) = popover_monitor(window, Some(rect)) else {
        return;
    };
    let Ok(window_size) = window.outer_size() else {
        return;
    };
    let scale = monitor.scale_factor();
    let position = rect.position.to_physical::<f64>(scale);
    let size = rect.size.to_physical::<f64>(scale);
    let area = monitor.work_area();
    let (x, y) = panel_position(
        (position.x, position.y, size.width, size.height),
        (
            area.position.x as f64,
            area.position.y as f64,
            area.size.width as f64,
            area.size.height as f64,
        ),
        (window_size.width as f64, window_size.height as f64),
    );
    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
}

fn bounded_panel_height(requested: f64, available: f64) -> f64 {
    requested.max(240.0).min(available.max(160.0)).ceil()
}

#[tauri::command]
fn resize_popover(app: tauri::AppHandle, height: f64) -> Result<f64, String> {
    if !height.is_finite() || height <= 0.0 {
        return Err("Некорректная высота окна.".into());
    }
    let window = app.get_webview_window("main").ok_or("Окно недоступно.")?;
    let anchor = *app
        .state::<PopoverState>()
        .anchor
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let monitor = popover_monitor(&window, anchor);
    let scale = monitor
        .as_ref()
        .map(|m| m.scale_factor())
        .unwrap_or_else(|| window.scale_factor().unwrap_or(1.0));
    let available = monitor
        .map(|m| m.work_area().size.height as f64 / scale - 16.0)
        .unwrap_or(720.0);
    let height = bounded_panel_height(height, available);
    window
        .set_size(tauri::LogicalSize::new(344.0, height))
        .map_err(|e| e.to_string())?;
    position_popover(&window, anchor);
    Ok(height)
}

#[cfg(test)]
mod panel_tests {
    use super::{bounded_panel_height, contains_physical_point, panel_position};
    #[test]
    fn retina_tray_uses_physical_monitor_bounds_and_stays_below_menu_bar() {
        assert!(contains_physical_point(
            (0, 0),
            (3024, 1964),
            (2800.0, 24.0)
        ));
        assert!(!contains_physical_point(
            (0, 0),
            (1512, 982),
            (2800.0, 24.0)
        ));
        assert_eq!(
            panel_position(
                (2780.0, 0.0, 40.0, 48.0),
                (0.0, 48.0, 3024.0, 1850.0),
                (688.0, 800.0)
            ),
            (2328, 56)
        );
    }
    #[test]
    fn resizing_keeps_anchor_and_bottom_tray_opens_upwards() {
        let small = panel_position(
            (900.0, 0.0, 30.0, 24.0),
            (0.0, 24.0, 1920.0, 1056.0),
            (344.0, 300.0),
        );
        let large = panel_position(
            (900.0, 0.0, 30.0, 24.0),
            (0.0, 24.0, 1920.0, 1056.0),
            (344.0, 600.0),
        );
        assert_eq!(small, large);
        assert_eq!(
            panel_position(
                (900.0, 1040.0, 30.0, 40.0),
                (0.0, 0.0, 1920.0, 1040.0),
                (344.0, 300.0)
            ),
            (743, 732)
        );
        assert!(contains_physical_point(
            (-1920, 0),
            (1920, 1080),
            (-100.0, 20.0)
        ));
    }
    #[test]
    fn content_height_respects_minimum_and_screen_space() {
        assert_eq!(bounded_panel_height(180.0, 800.0), 240.0);
        assert_eq!(bounded_panel_height(440.2, 800.0), 441.0);
        assert_eq!(bounded_panel_height(1200.0, 640.0), 640.0);
        assert_eq!(bounded_panel_height(300.0, 200.0), 200.0);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|_, _, _| {}))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            install_helper,
            gnome_status,
            logout_gnome_session,
            install_gnome_extension,
            get_status,
            updates::update_status,
            updates::check_update,
            updates::install_update,
            updates::open_releases,
            resize_popover,
            start_window_drag,
            set_launch_at_login,
            complete_setup,
            save_settings,
            start_capture,
            stop_capture,
            prepare_certificate,
            install_certificate,
            open_certificate,
            reveal_certificate,
            quit_app
        ])
        .setup(|app| {
            app.manage(PopoverState::default());
            app.manage(updates::Updates::default());
            #[cfg(target_os = "linux")]
            {
                app.manage(desktop_state::DesktopState::default());
                gnome::start(app.handle());
                gnome::finish_pending_setup(app.handle());
            }
            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window("main") {
                let native = window.ns_window()?.cast::<objc2::runtime::AnyObject>();
                // SAFETY: the setup callback runs on the GUI thread; native is this live NSWindow.
                unsafe {
                    let _: () = objc2::msg_send![native, setMovable: objc2::runtime::Bool::NO];
                    let _: () = objc2::msg_send![native, setMovableByWindowBackground: objc2::runtime::Bool::NO];
                }
            }
            app.manage(Shared::new(app.handle()).map_err(std::io::Error::other)?);
            if !cfg!(debug_assertions) {
                let handle=app.handle().clone();
                tauri::async_runtime::spawn(async move { let _=updates::check_update(handle).await; });
            }
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let show = tauri::menu::MenuItem::with_id(
                app,
                "show",
                "Открыть Show My Limits",
                true,
                None::<&str>,
            )?;
            let quit = tauri::menu::MenuItem::with_id(app, "quit", "Выйти", true, None::<&str>)?;
            let menu = tauri::menu::Menu::with_items(app, &[&show, &quit])?;
            let builder = TrayIconBuilder::with_id("show-my-limits")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .tooltip("Show My Limits · ожидаем лимиты")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        let rect = app.tray_by_id("show-my-limits").and_then(|tray| tray.rect().ok().flatten());
                        show_popover(app, rect);
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        rect,
                        ..
                    } = event
                    {
                        toggle_window(tray.app_handle(), Some(rect));
                    }
                });
            #[cfg(target_os = "macos")]
            let builder = builder.title("—");
            #[cfg(target_os = "windows")]
            let app_icon = app.default_window_icon().cloned().map(|icon| icon.to_owned());
            #[cfg(target_os = "windows")]
            let builder = builder.icon(app_icon.unwrap_or_else(|| tray_image::render("—", false)));
            #[cfg(target_os = "linux")]
            let builder = builder.icon(tray_image::render("—", false));
            builder.build(app)?;
            #[cfg(target_os = "windows")]
            windows_taskbar::init(app.handle().clone());
            let startup = app.handle().clone();
            tauri::async_runtime::spawn_blocking(move || startup.state::<Shared>().start_on_launch(&startup));
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                #[cfg(not(target_os = "windows"))]
                let mut previous_title = String::new();
                #[cfg(target_os = "linux")]
                let mut tray_visible = true;
                loop {
                    let status = handle.state::<Shared>().status(&handle);
                    let profile = status
                        .profiles
                        .iter()
                        .find(|p| Some(&p.id) == status.settings.selected_profile.as_ref())
                        .or_else(|| status.profiles.first());
                    let profile = profile.filter(|_| {
                        status.capture.phase == "listening" && status.connection_issue.is_none()
                    });
                    let title = profile.map(|p| p.title(status.now)).unwrap_or("—".into());
                    #[cfg(target_os = "windows")]
                    {
                        let weekly = profile
                            .map(|p| p.window_title("7d", status.now))
                            .unwrap_or("—".into());
                        let values = [title.clone(), weekly];
                        let controls = tauri::async_runtime::spawn_blocking(windows_taskbar::occupied_controls)
                            .await
                            .ok()
                            .flatten();
                        let _ = handle.run_on_main_thread(move || {
                            windows_taskbar::update(&values, controls.as_ref());
                        });
                    }
                    if let Some(tray) = handle.tray_by_id("show-my-limits") {
                        #[cfg(target_os = "linux")]
                        {
                            let visible = !handle.state::<desktop_state::DesktopState>().active();
                            if visible != tray_visible {
                                if tray.set_visible(visible).is_ok() { tray_visible = visible; }
                            }
                        }
                        #[cfg(not(target_os = "windows"))]
                        if title != previous_title {
                            #[cfg(target_os = "macos")]
                            let _ = tray.set_title(Some(&title));
                            #[cfg(target_os = "linux")]
                            let _ = tray.set_icon(Some(tray_image::render(&title, false)));
                            previous_title.clone_from(&title);
                        }
                        let tooltip = format!("Show My Limits · остаток за 5 часов: {title}");
                        let _ = tray.set_tooltip(Some(tooltip));
                    }
                    let _ = handle.emit("show-my-limits://status", &status);
                    // Pace status polling without blocking the async worker.
                    tauri::async_runtime::spawn_blocking(|| {
                        std::thread::sleep(std::time::Duration::from_secs(1))
                    })
                    .await
                    .ok();
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Focused(false) = event {
                if let Some(state) = window.try_state::<PopoverState>() {
                    #[cfg(target_os = "linux")]
                    if state.dragging.load(Ordering::SeqCst) {
                        return;
                    }
                    // Keep the parent GUI available while its own certificate authorization runs.
                    if !state.system_dialog.load(Ordering::SeqCst) && window.is_visible().unwrap_or(false) {
                        *state.last_blur.lock().unwrap_or_else(|p| p.into_inner()) = Some(Instant::now());
                        let _ = window.hide();
                    }
                }
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("Could not initialize Show My Limits");
    // Normal OS/session termination must use the same owned-helper cleanup as Quit.
    #[cfg(unix)]
    {
        let handle = app.handle().clone();
        ctrlc::set_handler(move || handle.exit(0)).expect("Could not register shutdown handler");
    }
    app.run(|app, event| {
        if let tauri::RunEvent::Exit = event {
            #[cfg(target_os = "windows")]
            windows_taskbar::close();
            app.state::<Shared>().shutdown();
        }
    });
}
