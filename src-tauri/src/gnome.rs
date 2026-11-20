//! GNOME session-bus bridge. No TCP listener, traffic logs or credentials.
use crate::desktop_state::DesktopState;
use gtk::{gio, glib, prelude::*};
use serde::Serialize;
use std::{fs, process::Command};
use tauri::{Emitter, Manager};

const UUID: &str = "show-my-limits@rowro.github.io";
const BUS: &str = "io.github.rowro.ShowMyLimits";
const PATH: &str = "/io/github/rowro/ShowMyLimits";
const XML: &str = r#"<node><interface name="io.github.rowro.ShowMyLimits1">
<method name="GetState"><arg direction="out" type="s" name="json"/></method>
<method name="Heartbeat"><arg direction="in" type="u" name="protocol"/><arg direction="out" type="b" name="accepted"/></method>
<method name="Detach"/>
<method name="OpenWindow"><arg direction="in" type="s" name="view"/></method>
<method name="Quit"/>
</interface></node>"#;

pub fn start(app: &tauri::AppHandle) {
    let handle = app.clone();
    gio::bus_own_name(
        gio::BusType::Session,
        BUS,
        gio::BusNameOwnerFlags::NONE,
        move |connection, _| {
            let info = gio::DBusNodeInfo::for_xml(XML).expect("static bridge interface");
            let handle = handle.clone();
            let registered = connection.register_object(
                PATH,
                &info.interfaces()[0],
                move |_, _, _, _, method, parameters, invocation| {
                    let state = handle.state::<DesktopState>();
                    match method {
                        "GetState" => {
                            invocation.return_value(Some(&(state.snapshot(),).to_variant()))
                        }
                        "Heartbeat" => {
                            let accepted = parameters
                                .get::<(u32,)>()
                                .is_some_and(|(p,)| state.heartbeat(p));
                            invocation.return_value(Some(&(accepted,).to_variant()));
                        }
                        "Detach" => {
                            state.detach();
                            invocation.return_value(None);
                        }
                        "OpenWindow" => {
                            let view = parameters.get::<(String,)>().map(|(v,)| v);
                            if !matches!(view.as_deref(), Some("settings" | "overview")) {
                                invocation.return_dbus_error(
                                    "io.github.rowro.ShowMyLimits.InvalidView",
                                    "Unknown view",
                                );
                                return;
                            }
                            let view = view.unwrap();
                            let app = handle.clone();
                            let _ = handle.run_on_main_thread(move || {
                                crate::show_popover(&app, None);
                                let _ = app.emit("show-my-limits://navigate", view);
                            });
                            invocation.return_value(None);
                        }
                        "Quit" => {
                            invocation.return_value(None);
                            handle.exit(0);
                        }
                        _ => invocation.return_dbus_error(
                            "org.freedesktop.DBus.Error.UnknownMethod",
                            "Unknown method",
                        ),
                    }
                },
                |_, _, _, _, _| false.to_variant(),
                |_, _, _, _, _, _| false,
            );
            if registered.is_err() {
                eprintln!("GNOME integration: could not register session object");
            }
        },
        |_, _| {},
        |_, _| {},
    );
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationStatus {
    supported: bool,
    installed: bool,
    active: bool,
    message: String,
}
fn extension_dir() -> Result<std::path::PathBuf, String> {
    Ok(glib::user_data_dir()
        .join("gnome-shell/extensions")
        .join(UUID))
}
#[tauri::command]
pub async fn gnome_status(app: tauri::AppHandle) -> Result<IntegrationStatus, String> {
    tauri::async_runtime::spawn_blocking(move || status(&app))
        .await
        .map_err(|e| e.to_string())?
}
fn status(app: &tauri::AppHandle) -> Result<IntegrationStatus, String> {
    let version = Command::new("gnome-shell")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let supported = version
        .split_whitespace()
        .last()
        .is_some_and(|v| v.split('.').next() == Some("46"));
    let installed = extension_dir()?.join("metadata.json").is_file();
    let active = app.state::<DesktopState>().active();
    let message = if active {
        "Расширение подключено. Левый клик — лимиты, правый — меню."
    } else if !supported {
        "Первая версия расширения поддерживает GNOME 46 (Ubuntu 24.04). На других оболочках работает обычный трей."
    } else if installed {
        "Расширение установлено. Включи Show My Limits в приложении «Расширения». После первой установки может понадобиться выйти из сеанса Ubuntu и войти снова."
    } else {
        "Проценты и прогресс в панели GNOME. Левый клик — лимиты, правый — меню."
    };
    Ok(IntegrationStatus {
        supported,
        installed,
        active,
        message: message.into(),
    })
}
#[tauri::command]
pub async fn install_gnome_extension(app: tauri::AppHandle) -> Result<IntegrationStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if !status(&app)?.supported {
            return Err("Эта версия расширения рассчитана на GNOME 46.".into());
        }
        let dir = extension_dir()?;
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let resources = app
            .path()
            .resource_dir()
            .map_err(|e| e.to_string())?
            .join("gnome");
        let source = if resources.join("extension.js").is_file() {
            resources
        } else if cfg!(debug_assertions) {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../integrations/gnome")
        } else {
            return Err("В сборке отсутствует расширение GNOME.".into());
        };
        // Only our extension files; never write GNOME's enabled-extension list.
        for name in [
            "extension.js",
            "model.js",
            "stylesheet.css",
            "metadata.json",
        ] {
            let bytes = fs::read(source.join(name)).map_err(|e| e.to_string())?;
            let temporary = dir.join(format!(".{name}.tmp"));
            fs::write(&temporary, bytes).map_err(|e| e.to_string())?;
            fs::rename(temporary, dir.join(name)).map_err(|e| e.to_string())?;
        }
        // GNOME may not discover a new extension until the next login. An enable
        // failure is reported as installed, not as a failed file installation.
        let _ = Command::new("gnome-extensions")
            .args(["enable", UUID])
            .output();
        status(&app)
    })
    .await
    .map_err(|e| e.to_string())?
}
