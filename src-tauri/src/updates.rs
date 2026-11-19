use crate::engine::Shared;
use serde::Serialize;
use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub phase: String,
    pub version: Option<String>,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub error: Option<String>,
    pub manual_install: bool,
}
#[derive(Default)]
pub struct Updates {
    status: Mutex<Status>,
    pending: Mutex<Option<Update>>,
    busy: AtomicBool,
}
struct Busy<'a>(&'a AtomicBool);
impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
fn publish(app: &AppHandle, update: impl FnOnce(&mut Status)) -> Status {
    let state = app.state::<Updates>();
    let mut status = state.status.lock().unwrap_or_else(|p| p.into_inner());
    update(&mut status);
    let result = status.clone();
    let _ = app.emit("show-my-limits://update", &result);
    result
}
fn manual_install() -> bool {
    cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_none()
}
#[tauri::command]
pub fn update_status(app: AppHandle) -> Status {
    let state = app.state::<Updates>();
    let mut status = state
        .status
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    status.manual_install = manual_install();
    status
}
#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<Status, String> {
    let state = app.state::<Updates>();
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err("Обновление уже выполняется.".into());
    }
    let _busy = Busy(&state.busy);
    publish(&app, |s| {
        s.phase = "checking".into();
        s.error = None;
        s.manual_install = manual_install();
    });
    let result = async {
        app.updater_builder()
            .timeout(Duration::from_secs(20))
            .build()?
            .check()
            .await
    }
    .await;
    match result {
        Ok(update) => {
            let version = update.as_ref().map(|u| u.version.clone());
            *state.pending.lock().unwrap_or_else(|p| p.into_inner()) = update;
            Ok(publish(&app, |s| {
                s.phase = if version.is_some() {
                    "available"
                } else {
                    "current"
                }
                .into();
                s.version = version;
            }))
        }
        Err(tauri_plugin_updater::Error::ReleaseNotFound) => Ok(publish(&app, |s| {
            s.phase = "unpublished".into();
        })),
        Err(error) => {
            let message = format!("Не удалось проверить обновления: {error}");
            publish(&app, |s| {
                s.phase = "error".into();
                s.error = Some(message.clone());
            });
            Err(message)
        }
    }
}
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    if manual_install() {
        return Err("Для этой установки Linux скачайте новый пакет на странице выпусков.".into());
    }
    let state = app.state::<Updates>();
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err("Обновление уже выполняется.".into());
    }
    let _busy = Busy(&state.busy);
    let update = state
        .pending
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .ok_or("Сначала проверьте обновления.")?;
    publish(&app, |s| {
        s.phase = "downloading".into();
        s.downloaded = 0;
        s.total = None;
        s.error = None;
    });
    // Download verifies the updater signature before capture is stopped or files are changed.
    let result = async {
        let bytes = update
            .download(
                |chunk, total| {
                    publish(&app, |s| {
                        s.downloaded += chunk as u64;
                        s.total = total;
                    });
                },
                || {},
            )
            .await
            .map_err(|e| e.to_string())?;
        publish(&app, |s| s.phase = "installing".into());
        let handle = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            handle.state::<Shared>().stop_for_update(&handle)
        })
        .await
        .map_err(|e| e.to_string())??;
        let handle = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let result = update.install(bytes).map_err(|e| e.to_string());
            if result.is_ok() {
                handle.restart();
            }
            result
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    if let Err(ref error) = result {
        publish(&app, |s| {
            s.phase = "error".into();
            s.error = Some(format!(
                "Обновление не установлено: {error}. Перехват можно запустить вручную."
            ));
        });
    }
    result
}
#[tauri::command]
pub fn open_releases(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(
            "https://github.com/rowro/show-my-limits/releases",
            None::<&str>,
        )
        .map_err(|e| e.to_string())
}
