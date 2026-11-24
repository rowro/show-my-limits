use crate::installer::{self, Installation};
use crate::process_tree::OwnedChild;
use crate::quota::{Profile, ProxyState};
use serde::Serialize;
use std::{
    collections::{BTreeSet, HashMap},
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt;

pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn checked_ca_trust(path: &Path) -> Option<bool> {
    crate::certificate::os_trust(path)
}

fn checked_ca_scope(path: &Path) -> Option<String> {
    crate::certificate::os_trust_scope(path)
}

fn abandoned_store(active: &Path) -> bool {
    crate::certificate::abandoned_store(active, &|cert| (checked_ca_trust(cert), checked_ca_scope(cert)))
}

/// Whether any capture in this data directory ever recorded a quota sample.
/// Unreadable evidence counts as "yes": skipping the trial capture must never
/// break a setup that may have worked.
fn quotas_ever_received(proxy: &Path) -> bool {
    let mut files = vec![proxy.join("state.json")];
    match fs::read_dir(proxy.join("runs")) {
        Ok(entries) => {
            for entry in entries {
                let Ok(entry) = entry else { return true; };
                files.push(entry.path().join("state.json"));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return true,
    }
    files.iter().any(|path| match fs::symlink_metadata(path) {
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
        Ok(meta) if !meta.file_type().is_file() || meta.len() > 262_144 => true,
        Ok(_) => fs::read(path).ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .map_or(true, |state| state.get("profiles").and_then(|p| p.as_array()).is_some_and(|p| !p.is_empty())),
    })
}

use crate::settings::normalized_processes;
pub use crate::settings::{Settings, normalize_host, selector};

fn migrate_process_settings(settings: &mut Settings) {
    if settings.clients.iter().any(|client| client == "vscode") {
        for name in ["Code", "code", "extensionHost"] {
            if !settings
                .additional_processes
                .iter()
                .any(|process| process == name)
            {
                settings.additional_processes.push(name.into());
            }
        }
    }
    // Legacy client selection is retained only for compatibility with older settings.
    settings.clients = vec!["desktop".into(), "terminal".into()];
}

pub struct Paths {
    pub config: PathBuf,
    pub proxy: PathBuf,
    pub certificates: PathBuf,
    pub helper: PathBuf,
}
pub struct Inner {
    pub settings: Settings,
    pub child: Option<OwnedChild>,
    pub capture_lock: Option<std::fs::File>,
    pub stop_at: Option<f64>,
    pub started_at: f64,
    pub error: Option<String>,
    pub capture_issue: Arc<Mutex<Option<String>>>,
    pub run_id: Option<String>,
    pub last_state: Option<ProxyState>,
    pub rotation_trial: bool,
    /// Last macOS network-extension check while starting: (checked_at, waiting for user).
    pub extension_check: Option<(f64, bool)>,
    /// When the capture last waited for an OS approval; the start deadline counts from it.
    pub approval_seen_at: Option<f64>,
}
pub struct Shared {
    pub paths: Paths,
    pub inner: Mutex<Inner>,
    pub installation: Mutex<Installation>,
    pub certificate_installation: Mutex<()>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStatus {
    pub phase: String,
    pub pid: Option<u32>,
    pub started_at: Option<f64>,
    pub error: Option<String>,
    pub observation: Option<String>,
    /// An OS approval the helper waits for, e.g. "macos_extension".
    pub approval: Option<String>,
}
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupStatus {
    pub platform: String,
    pub helper_path: Option<String>,
    pub certificate_path: String,
    pub certificate_exists: bool,
    pub certificate_trusted: Option<bool>,
    pub certificate_trust_error: Option<String>,
    pub certificate_security_issue: Option<String>,
    pub certificate_trust_scope: Option<String>,
    pub certificate_name: Option<String>,
    pub certificate_sha256: Option<String>,
    pub certificate_expires_at: Option<i64>,
    /// "leaf": CA key never stored, limited to `certificate_hosts`; "legacy": mitmproxy CA with its key.
    pub certificate_format: Option<String>,
    pub certificate_hosts: Vec<String>,
    /// An older, never-trusted or incomplete store that the wizard replaces with a leaf-only one.
    pub certificate_replaceable: bool,
    pub installation: Installation,
    pub install_supported: bool,
    pub launch_at_login: Option<bool>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    pub settings: Settings,
    pub profiles: Vec<Profile>,
    pub capture: CaptureStatus,
    pub setup: SetupStatus,
    pub rotation: RotationStatus,
    pub connection_issue: Option<String>,
    pub now: f64,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RotationStatus {
    pub phase: String,
    pub pending_sha256: Option<String>,
    pub pending_certificate_path: Option<String>,
    pub pending_trusted: Option<bool>,
    pub pending_issue: Option<String>,
    pub pending_hosts: Vec<String>,
    /// The active legacy CA never produced quotas, so switching needs no trial capture.
    pub trial_optional: bool,
    pub previous_sha256: Option<String>,
    pub previous_certificate_path: Option<String>,
    pub previous_trust_scope: Option<String>,
    pub trial_ready: bool,
}

impl AppStatus {
    pub fn setup_complete(&self) -> bool {
        self.setup.helper_path.is_some()
            && self.setup.certificate_exists
            && !self.settings.hosts.is_empty()
            && self
                .setup
                .certificate_trusted
                .unwrap_or(self.settings.certificate_trust_confirmed || !self.profiles.is_empty())
    }
}

impl Shared {
    pub fn new(app: &AppHandle) -> Result<Self, String> {
        let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
        let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
        fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;
        let config = config_dir.join("settings.json");
        let mut settings: Settings = fs::read(&config)
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default();
        migrate_process_settings(&mut settings);
        let paths = Paths {
            proxy: root.join("proxy"),
            certificates: root.join("certificates"),
            helper: root.join("mitmproxy"),
            config,
        };
        Ok(Self {
            paths,
            installation: Mutex::new(Installation::default()),
            certificate_installation: Mutex::new(()),
            inner: Mutex::new(Inner {
                settings,
                child: None,
                capture_lock: None,
                stop_at: None,
                started_at: 0.0,
                error: None,
                capture_issue: Arc::new(Mutex::new(None)),
                run_id: None,
                last_state: None,
                rotation_trial: false,
                extension_check: None,
                approval_seen_at: None,
            }),
        })
    }

    fn find_helper(&self, app: &AppHandle, settings: &Settings) -> Option<PathBuf> {
        let executable = if cfg!(windows) {
            "mitmdump.exe"
        } else {
            "mitmdump"
        };
        let mut candidates = Vec::new();
        if !settings.helper_path.is_empty() {
            candidates.push(PathBuf::from(&settings.helper_path));
        }
        if let Some(path) = installer::installed_path(&self.paths.helper) {
            candidates.push(path);
        }
        if let Ok(resources) = app.path().resource_dir() {
            candidates.extend([
                resources.join("binaries").join(executable),
                resources.join("binaries/mitmproxy").join(executable),
                resources
                    .join("binaries/mitmproxy/mitmproxy.app/Contents/MacOS")
                    .join(executable),
            ]);
        }
        if let Some(path) = std::env::var_os("PATH") {
            candidates.extend(std::env::split_paths(&path).map(|p| p.join(executable)));
        }
        #[cfg(target_os = "macos")]
        candidates.extend([
            PathBuf::from("/opt/homebrew/bin/mitmdump"),
            PathBuf::from("/usr/local/bin/mitmdump"),
            PathBuf::from("/Applications/mitmproxy.app/Contents/MacOS/mitmdump"),
        ]);
        if let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) {
            let home = PathBuf::from(home);
            candidates.extend([
                home.join(".local/bin").join(executable),
                home.join("Applications/mitmproxy.app/Contents/MacOS")
                    .join(executable),
            ]);
        }
        // Preserve a path deliberately chosen by an existing user. For a new
        // setup, only reuse a discovered external helper if it is the pinned
        // version and can actually start; otherwise install our verified copy.
        let configured = !settings.helper_path.is_empty() || !settings.hosts.is_empty();
        candidates.into_iter().find(|p| p.is_file() &&
            (configured || p.starts_with(&self.paths.helper) || compatible_external_helper(p)))
    }

    pub fn status(&self, app: &AppHandle) -> AppStatus {
        let timestamp = now();
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if inner.stop_at.is_some_and(|at| timestamp - at > 3.0) {
            if let Some(child) = inner.child.as_mut() {
                let _ = child.kill();
            }
        }
        let exited = inner
            .child
            .as_mut()
            .and_then(|child| child.try_wait().ok().flatten());
        if let Some(status) = exited {
            if inner.stop_at.is_none() && !status.success() {
                inner.error = Some(format!(
                    "Перехватчик завершился (код {}). Повторите запуск. Если ошибка повторится, проверьте разрешения захвата вашей ОС.",
                    status
                        .code()
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "сигнал".into())
                ));
            }
            inner.child = None;
            inner.capture_lock = None;
            inner.stop_at = None;
        }
        if inner.child.is_none() && inner.error.is_some() {
            let issue = inner
                .capture_issue
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            if let Some(issue) = issue {
                inner.error = Some(issue);
            }
        }
        // Each owned capture writes to a distinct directory. A legacy helper or
        // a delayed previous worker can no longer overwrite this session's data.
        let run_id = inner.run_id.clone();
        let state = read_capture_state(&self.paths.proxy, run_id.as_deref(), &mut inner.last_state);
        let pid = inner.child.as_ref().map(|child| child.id());
        let active = current_capture_state(
            &state,
            inner.run_id.as_deref(),
            pid.is_some(),
            inner.started_at,
            timestamp,
        );
        let starting = pid.is_some() && !active && inner.stop_at.is_none();
        // mitmproxy's macOS redirector waits without a timeout until the user allows
        // its network extension; it prints nothing. Read the extension list instead.
        #[cfg(target_os = "macos")]
        let awaiting_extension = starting && timestamp - inner.started_at > 2.0 && {
            if inner.extension_check.is_none_or(|(at, _)| timestamp - at > 3.0) {
                let waiting = crate::macos_extension::state() == Some(crate::macos_extension::ExtensionState::WaitingForUser);
                inner.extension_check = Some((timestamp, waiting));
            }
            inner.extension_check.is_some_and(|(_, waiting)| waiting)
        };
        #[cfg(not(target_os = "macos"))]
        let awaiting_extension = false;
        if !starting {
            inner.extension_check = None;
            inner.approval_seen_at = None;
        } else if awaiting_extension {
            inner.approval_seen_at = Some(timestamp);
        }
        // The deadline does not run out while the OS waits for the user's approval.
        if starting
            && timestamp - inner.started_at > 90.0
            && inner.approval_seen_at.is_none_or(|seen| timestamp - seen > 60.0)
        {
            if let Some(child) = inner.child.as_mut() {
                signal_stop(child);
            }
            inner.stop_at = Some(timestamp);
            let issue = inner.capture_issue.lock().unwrap_or_else(|p| p.into_inner()).clone();
            inner.error = Some(issue.unwrap_or_else(|| if cfg!(target_os = "macos") {
                "Перехватчик не подтвердил готовность за 90 секунд, запуск отменён. Проверьте, не ждёт ли macOS разрешения: сетевое расширение Mitmproxy Redirector (Системные настройки → Основные → Объекты входа и расширения) или запрос на добавление конфигурации прокси. Затем повторите запуск.".into()
            } else {
                "Перехватчик не подтвердил готовность за 90 секунд. Запуск отменён. Проверьте запрос прав и повторите попытку.".into()
            }));
        }
        let profiles: Vec<_> = state
            .profiles
            .into_iter()
            .filter(|profile| inner.settings.hosts.contains(&profile.host))
            .collect();
        let trial_ready = inner.rotation_trial && active && profiles.iter().any(|profile|
            profile.observed_at >= inner.started_at && (0.0..120.0).contains(&(timestamp - profile.observed_at)));
        let (pending, retired) = crate::certificate::rotation_paths(&self.paths.certificates)
            .expect("application CA path has a parent");
        let pending_complete = crate::certificate::complete_store(&pending);
        // An abandoned store may be incomplete; any readable certificate still needs cleanup checks.
        let previous_cert = crate::certificate::retired_certificates(&self.paths.certificates)
            .ok().and_then(|certs| certs.into_iter().next());
        let trial_optional = pending_complete
            && crate::certificate::store_format(&self.paths.certificates) == Some(crate::certificate::StoreFormat::Legacy)
            && !quotas_ever_received(&self.paths.proxy);
        let rotation = RotationStatus {
            phase: if fs::symlink_metadata(&retired).is_ok() && !crate::certificate::complete_store(&self.paths.certificates) { "interrupted" }
                else if fs::symlink_metadata(&retired).is_ok() { "cleanup" }
                else if fs::symlink_metadata(&pending).is_ok() && inner.rotation_trial { "testing" }
                else if fs::symlink_metadata(&pending).is_ok() { "pending" }
                else { "none" }.into(),
            pending_sha256: pending_complete.then(|| crate::certificate::details(&pending.join("mitmproxy-ca-cert.cer"))).flatten().map(|d| d.sha256),
            pending_certificate_path: pending_complete.then(|| pending.join("mitmproxy-ca-cert.cer").display().to_string()),
            pending_trusted: pending_complete.then(|| checked_ca_trust(&pending.join("mitmproxy-ca-cert.cer"))).flatten(),
            pending_issue: crate::certificate::security_issue(&pending).map(str::to_owned),
            pending_hosts: crate::certificate::leaf_hosts(&pending).unwrap_or_default(),
            previous_sha256: previous_cert.as_deref().and_then(crate::certificate::details).map(|d| d.sha256),
            previous_certificate_path: previous_cert.as_ref().map(|path| path.display().to_string()),
            previous_trust_scope: previous_cert.as_deref().and_then(checked_ca_scope),
            trial_optional,
            trial_ready,
        };
        let connection_issue = state.health.as_ref().filter(|health| active && timestamp - health.observed_at < 120.0
            && profiles.iter().filter(|p| p.host == health.host).all(|p| p.observed_at < health.observed_at))
            .and_then(|health| match health.kind.as_str() {
                "certificate_client" => Some(format!("Одно из соединений клиента отклонило сертификат прокси для {}. Если ответы Claude не приходят, полностью перезапустите клиент.", health.host)),
                "certificate_server" => Some(format!("Не удалось проверить сертификат сервера {} в одном из соединений.", health.host)),
                "network_ipv6" => Some(format!("Не удалось соединиться с {} по IPv6: маршрут недоступен. Проверьте IPv6 в этой системе или выполните диагностический запрос по IPv4.", health.host)),
                "network_unreachable" => Some(format!("Сетевой маршрут к {} недоступен. Проверьте подключение и маршрутизацию.", health.host)),
                "tls_client" | "tls_server" => Some(format!("Не завершено TLS-соединение с {}. Ошибка доверия к сертификату не подтверждена.", health.host)),
                _ => None,
            });
        let phase = if inner.stop_at.is_some() {
            "stopping"
        } else if active {
            "listening"
        } else if pid.is_some() {
            "starting"
        } else {
            "stopped"
        };
        let certificate = self.paths.certificates.join("mitmproxy-ca-cert.cer");
        let identity = crate::certificate::details(&certificate);
        #[cfg(target_os = "macos")]
        let (certificate_trusted, certificate_trust_error) = if certificate.is_file() {
            match crate::macos_trust::is_trusted(&certificate) {
                Ok(trusted) => (Some(trusted), None),
                Err(error) => (
                    Some(false),
                    Some(format!("Не удалось проверить системное доверие: {error}")),
                ),
            }
        } else {
            (Some(false), None)
        };
        #[cfg(target_os = "windows")]
        let (certificate_trusted, certificate_trust_error) = if certificate.is_file() {
            match crate::windows_trust::is_trusted(&certificate) {
                Ok(trusted) => (Some(trusted), None),
                Err(error) => (Some(false), Some(error)),
            }
        } else {
            (Some(false), None)
        };
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let (certificate_trusted, certificate_trust_error) = (None, None);
        AppStatus {
            settings: inner.settings.clone(),
            profiles,
            capture: CaptureStatus {
                phase: phase.into(),
                pid,
                started_at: pid.map(|_| inner.started_at),
                error: inner.error.clone(),
                observation: state.health.as_ref()
                    .filter(|health| active && timestamp - health.observed_at < 120.0)
                    .map(|health| health.kind.clone()),
                approval: awaiting_extension.then(|| "macos_extension".into()),
            },
            setup: SetupStatus {
                platform: std::env::consts::OS.into(),
                helper_path: self
                    .find_helper(app, &inner.settings)
                    .map(|p| p.display().to_string()),
                certificate_path: certificate.display().to_string(),
                certificate_exists: certificate.is_file(),
                certificate_trusted,
                certificate_trust_error,
                certificate_security_issue: crate::certificate::security_issue(&self.paths.certificates).map(str::to_owned),
                certificate_trust_scope: {
                    #[cfg(target_os = "macos")]
                    { certificate.is_file().then(|| crate::macos_trust::trust_scope(&certificate).ok()).flatten().map(str::to_owned) }
                    #[cfg(target_os = "linux")]
                    { certificate.is_file().then(|| crate::certificate::debian_system_trust(&certificate)).flatten().map(|present| if present { "system" } else { "none" }.to_owned()) }
                    #[cfg(target_os = "windows")]
                    { certificate.is_file().then(|| crate::windows_trust::trust_scope(&certificate).ok()).flatten().map(str::to_owned) }
                    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
                    { None }
                },
                certificate_name: identity.as_ref().map(|d| d.name.clone()),
                certificate_expires_at: identity.as_ref().map(|d| d.expires_at),
                certificate_sha256: identity.map(|d| d.sha256),
                certificate_format: crate::certificate::store_format(&self.paths.certificates).map(|f| f.as_str().to_owned()),
                certificate_hosts: crate::certificate::leaf_hosts(&self.paths.certificates).unwrap_or_default(),
                certificate_replaceable: abandoned_store(&self.paths.certificates),
                installation: self
                    .installation
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clone(),
                launch_at_login: app.autolaunch().is_enabled().ok(),
                install_supported: installer::package(std::env::consts::OS, std::env::consts::ARCH)
                    .is_ok(),
            },
            rotation,
            connection_issue,
            now: timestamp,
        }
    }

    pub fn install_helper(&self, app: &AppHandle) -> Result<(), String> {
        let _setup = crate::coordination::lock(&self.paths.proxy, "setup")?;
        let settings = self
            .inner
            .lock()
            .map_err(|_| "Настройки заняты.")?
            .settings
            .clone();
        if self.find_helper(app, &settings).is_some() {
            return Ok(());
        }
        {
            let mut installation = self.installation.lock().map_err(|_| "Установка занята.")?;
            if installation.active() {
                return Err("Установка уже выполняется.".into());
            }
            *installation = Installation {
                phase: "downloading".into(),
                ..Default::default()
            };
        }
        let result = installer::install_to(&self.paths.helper, |progress| {
            *self.installation.lock().unwrap_or_else(|p| p.into_inner()) = progress;
        });
        let mut installation = self.installation.lock().unwrap_or_else(|p| p.into_inner());
        installation.phase = if result.is_ok() { "ready" } else { "error" }.into();
        installation.error = result.as_ref().err().cloned();
        result.map(|_| ())
    }

    pub fn save_settings(&self, mut settings: Settings) -> Result<bool, String> {
        if !["system", "light", "dark"].contains(&settings.theme.as_str()) {
            return Err("Неизвестная тема.".into());
        }
        if settings.hosts.len() > 32 {
            return Err("Можно выбрать до 32 серверов.".into());
        }
        settings.hosts = settings
            .hosts
            .iter()
            .map(|h| normalize_host(h))
            .collect::<Result<BTreeSet<_>, _>>()?
            .into_iter()
            .collect();
        settings.additional_processes = normalized_processes(&settings.additional_processes)?;
        settings.clients = vec!["desktop".into(), "terminal".into()];
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "Не удалось открыть настройки.")?;
        let restart = inner.child.is_some()
            && (settings.hosts != inner.settings.hosts
                || settings.additional_processes != inner.settings.additional_processes
                || settings.helper_path != inner.settings.helper_path);
        if restart && settings.hosts.is_empty() {
            return Err("Укажите хотя бы один сервер.".into());
        }
        let temporary = self.paths.config.with_extension("tmp");
        fs::write(
            &temporary,
            serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        // Settings are serialized by this mutex; replacement is only within our own app directory.
        #[cfg(windows)]
        if self.paths.config.exists() {
            fs::copy(
                &self.paths.config,
                self.paths.config.with_extension("backup.json"),
            )
            .map_err(|e| e.to_string())?;
            fs::remove_file(&self.paths.config).map_err(|e| e.to_string())?;
        }
        fs::rename(&temporary, &self.paths.config).map_err(|e| e.to_string())?;
        inner.settings = settings;
        if restart {
            if let Some(child) = inner.child.as_mut() {
                signal_stop(child);
                inner.stop_at = Some(now());
            }
        }
        Ok(restart)
    }

    pub fn save_and_apply(&self, app: &AppHandle, settings: Settings) -> Result<(), String> {
        if !self.save_settings(settings)? {
            return Ok(());
        }
        // Wait for our own helper to release capture before starting its replacement.
        let result = (|| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                self.status(app);
                if self
                    .inner
                    .lock()
                    .map_err(|_| "Перехватчик занят.")?
                    .child
                    .is_none()
                {
                    return self.start(app);
                }
                if std::time::Instant::now() >= deadline {
                    return Err("Настройки сохранены, но перехватчик ещё останавливается. Попробуйте запустить его снова.".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        })();
        if let Err(ref error) = result {
            self.inner.lock().unwrap_or_else(|p| p.into_inner()).error = Some(error.clone());
        }
        result
    }

    pub fn complete_setup(&self, app: &AppHandle) -> Result<(), String> {
        let status = self.status(app);
        if !status.setup_complete() {
            return Err("Завершите все шаги подключения.".into());
        }
        if matches!(status.capture.phase.as_str(), "starting" | "listening") {
            return Ok(());
        }
        self.start(app)
    }

    pub fn start_on_launch(&self, app: &AppHandle) {
        if !self.status(app).setup_complete() {
            return;
        }
        if let Err(error) = self.complete_setup(app) {
            self.inner.lock().unwrap_or_else(|p| p.into_inner()).error = Some(error);
        }
    }

    pub fn start(&self, app: &AppHandle) -> Result<(), String> {
        self.start_with_ca(app, false)
    }

    pub fn start_replacement_trial(&self, app: &AppHandle) -> Result<(), String> {
        let (pending, retired) = crate::certificate::rotation_paths(&self.paths.certificates)?;
        if fs::symlink_metadata(retired).is_ok() {
            return Err("Сначала завершите очистку доверия к предыдущему CA.".into());
        }
        if !crate::certificate::complete_store(&pending) {
            return Err("Сначала подготовьте новый CA.".into());
        }
        if checked_ca_trust(&pending.join("mitmproxy-ca-cert.cer")) != Some(true) {
            return Err("Доверие к новому CA ещё не подтверждено. Действующий захват не остановлен.".into());
        }
        self.stop_for_update(app)?;
        if let Err(error) = self.start_with_ca(app, true) {
            return match self.start_with_ca(app, false) {
                Ok(()) => Err(format!("Пробный захват не запустился: {error}. Захват со старым CA восстановлен.")),
                Err(restore) => Err(format!("Пробный захват не запустился: {error}. Не удалось восстановить старый захват: {restore}.")),
            };
        }
        Ok(())
    }

    fn start_with_ca(&self, app: &AppHandle, trial: bool) -> Result<(), String> {
        let ca_dir = if trial {
            crate::certificate::rotation_paths(&self.paths.certificates)?.0
        } else {
            self.paths.certificates.clone()
        };
        let ca_file = ca_dir.join("mitmproxy-ca-cert.cer");
        #[cfg(target_os = "macos")]
        if !crate::macos_trust::is_trusted(&ca_file)?
        {
            return Err(if trial { "Новый CA ещё не доверен. Подтвердите доверие перед пробным захватом." } else { "Действующий CA больше не доверен. Подтвердите доверие перед запуском." }.into());
        }
        #[cfg(target_os = "windows")]
        if trial && !crate::windows_trust::is_trusted(&ca_file)? {
            return Err("Новый CA ещё не доверен в Windows.".into());
        }
        #[cfg(target_os = "linux")]
        if trial && crate::certificate::debian_system_trust(&ca_file) != Some(true) {
            return Err("Новый CA не найден в системном наборе доверия Debian/Ubuntu. Установите его явно и повторите проверку.".into());
        }

        let mut inner = self.inner.lock().map_err(|_| "Перехватчик занят.")?;
        if inner.child.is_some() {
            return Err("Перехватчик уже запущен или ещё останавливается.".into());
        }
        if inner.settings.hosts.is_empty() {
            return Err("Добавьте сервер в настройках.".into());
        }
        let clients = selector(&inner.settings.additional_processes)?;
        let capture_lock = crate::coordination::lock(&self.paths.proxy, "capture")?;
        crate::capture_conflict::ensure_exclusive()?;
        if system_proxy_active() {
            return Err("Обнаружен другой системный прокси. Завершите его захват перед запуском Show My Limits; настройки сети Show My Limits не изменяет.".into());
        }
        if !ca_file.is_file()
        {
            return Err("Сначала создайте сертификат в мастере первоначальной настройки.".into());
        }
        // Legacy stores run as mitmproxy confdir; leaf-only stores pass the leaf via --certs.
        let tls_args = crate::certificate::capture_tls_args(&ca_dir, &self.paths.proxy, &inner.settings.hosts)?;
        let helper = self
            .find_helper(app, &inner.settings)
            .ok_or("Не найден mitmdump. Проверьте установку перехватчика.")?;
        let addon = app
            .path()
            .resolve(
                "proxy/limits_addon.py",
                tauri::path::BaseDirectory::Resource,
            )
            .map_err(|e| e.to_string())?;
        if !addon.is_file() {
            return Err("В сборке отсутствует обработчик лимитов.".into());
        }
        let hosts = inner
            .settings
            .hosts
            .iter()
            .map(|h| format!("{}(?::[0-9]+)?", regex::escape(h)))
            .collect::<Vec<_>>()
            .join("|");
        // One-file distributions spawn a Python worker with a different PID.
        // Correlate state by launch ID, while the owned launcher must remain alive.
        let run_id = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let state_dir = self.paths.proxy.join("runs").join(&run_id);
        fs::create_dir_all(&state_dir)
            .map_err(|e| format!("Не удалось подготовить состояние захвата: {e}"))?;
        crate::coordination::publish_run(&self.paths.proxy, &run_id)?;
        let mut command = helper_command(&helper);
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        command.env("SML_RUN_ID", &run_id);
        #[cfg(target_os = "linux")]
        {
            let askpass = app
                .path()
                .resolve(
                    "proxy/linux_askpass.sh",
                    tauri::path::BaseDirectory::Resource,
                )
                .map_err(|e| e.to_string())?;
            if !askpass.is_file() {
                return Err(
                    "В сборке отсутствует компонент запроса прав Linux. Переустановите приложение."
                        .into(),
                );
            }
            if !Path::new("/usr/bin/zenity").is_file() {
                return Err("Для графического запроса прав установите zenity: sudo apt install zenity. Затем повторите запуск.".into());
            }
            // sudo invokes this only when it needs authentication without a TTY.
            // The GUI never receives the password; it goes directly to sudo.
            command.env("SUDO_ASKPASS", askpass);
        }
        command
            .args([
                "--mode",
                &format!("local:{clients}"),
                "--allow-hosts",
                &format!("^(?:{hosts})$"),
                "--set",
                "flow_detail=0",
                "--set",
                "termlog_verbosity=error",
            ])
            .args(&tls_args)
            .arg("-s")
            .arg(addon)
            .env("SML_STATE_DIR", &state_dir)
            .env(
                "SML_HOSTS",
                serde_json::to_string(&inner.settings.hosts).map_err(|e| e.to_string())?,
            )
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = OwnedChild::spawn(&mut command)
            .map_err(|e| format!("Не удалось безопасно запустить перехватчик: {e}"))?;
        let issue = Arc::new(Mutex::new(None));
        if let Some(stdout) = child.stdout.take() {
            monitor_capture_output(stdout, issue.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            monitor_capture_output(stderr, issue.clone());
        }
        inner.capture_issue = issue;
        inner.run_id = Some(run_id);
        inner.last_state = None;
        inner.rotation_trial = trial;
        inner.child = Some(child);
        inner.capture_lock = Some(capture_lock);
        inner.started_at = now();
        inner.stop_at = None;
        inner.error = None;
        Ok(())
    }

    pub fn stop(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(child) = inner.child.as_mut() {
            signal_stop(child);
            inner.stop_at = Some(now());
        }
    }

    pub fn stop_for_update(&self, app: &AppHandle) -> Result<(), String> {
        self.stop();
        for _ in 0..50 {
            self.status(app);
            if self
                .inner
                .lock()
                .map_err(|_| "Перехватчик занят.")?
                .child
                .is_none()
            {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Err("Перехватчик ещё останавливается. Повторите обновление позже.".into())
    }

    pub fn shutdown(&self) {
        let child = self
            .inner
            .lock()
            .ok()
            .and_then(|mut inner| inner.child.take());
        if let Some(mut child) = child {
            signal_stop(&mut child);
            for _ in 0..30 {
                if child.try_wait().ok().flatten().is_some() { break; }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            if child.try_wait().ok().flatten().is_none() { let _ = child.kill(); }
            let _ = child.wait();
        }
        if let Ok(mut inner) = self.inner.lock() { inner.capture_lock = None; }
    }

    pub fn install_certificate(&self) -> Result<(), String> {
        let _guard = self
            .certificate_installation
            .try_lock()
            .map_err(|_| "Установка сертификата уже выполняется.")?;
        crate::certificate::install(&self.paths.certificates.join("mitmproxy-ca-cert.cer"))?;
        // Complete only after native import and current-user HTTPS trust read-back succeed.
        let mut settings = self
            .inner
            .lock()
            .map_err(|_| "Сертификат установлен, но настройки заняты.")?
            .settings
            .clone();
        settings.certificate_trust_confirmed = true;
        self.save_settings(settings)
            .map(|_| ())
            .map_err(|e| format!("Сертификат установлен, но не удалось сохранить настройки: {e}"))
    }

    pub fn install_replacement_certificate(&self) -> Result<(), String> {
        let (pending, retired) = crate::certificate::rotation_paths(&self.paths.certificates)?;
        if fs::symlink_metadata(retired).is_ok() || !crate::certificate::complete_store(&pending) {
            return Err("Новый CA не подготовлен или прежняя очистка не завершена.".into());
        }
        crate::certificate::install(&pending.join("mitmproxy-ca-cert.cer"))
    }

    pub fn promote_replacement(&self, app: &AppHandle) -> Result<(), String> {
        let status = self.status(app);
        let tested = status.rotation.phase == "testing" && status.rotation.trial_ready;
        let skippable = status.rotation.trial_optional && matches!(status.rotation.phase.as_str(), "pending" | "testing");
        if !tested && !skippable {
            return Err("Дождитесь свежего ответа клиента в пробном захвате новым CA.".into());
        }
        if status.rotation.pending_trusted != Some(true) {
            return Err("Доверие к новому CA не подтверждено.".into());
        }
        self.stop_for_update(app)?;
        let _setup = crate::coordination::lock(&self.paths.proxy, "setup")?;
        let _guard = self.certificate_installation.try_lock()
            .map_err(|_| "Замена CA уже выполняется.")?;
        let _capture = crate::coordination::lock(&self.paths.proxy, "capture")?;
        let (pending, _) = crate::certificate::rotation_paths(&self.paths.certificates)?;
        if checked_ca_trust(&pending.join("mitmproxy-ca-cert.cer")) != Some(true) {
            return Err("Доверие к новому CA исчезло; старый CA сохранён.".into());
        }
        crate::certificate::promote_replacement(&self.paths.certificates)?;
        self.inner.lock().map_err(|_| "Перехватчик занят.")?.rotation_trial = false;
        drop(_capture);
        self.start(app).map_err(|error| format!("Новый CA активен, но захват не запустился: {error}. Повторите запуск; прежний CA сохранён для очистки."))
    }

    pub fn stop_replacement_trial(&self, app: &AppHandle) -> Result<(), String> {
        if !self.inner.lock().map_err(|_| "Перехватчик занят.")?.rotation_trial {
            return Ok(());
        }
        self.stop_for_update(app)?;
        self.inner.lock().map_err(|_| "Перехватчик занят.")?.rotation_trial = false;
        self.start(app)
    }

    pub fn finish_replacement_cleanup(&self) -> Result<(), String> {
        let _setup = crate::coordination::lock(&self.paths.proxy, "setup")?;
        let (_, retired) = crate::certificate::rotation_paths(&self.paths.certificates)?;
        if !crate::certificate::complete_store(&self.paths.certificates) {
            return Err("Действующий CA повреждён; старый ключ сохранён для восстановления.".into());
        }
        if fs::symlink_metadata(&retired).is_err() {
            return Err("Прежний CA уже удалён.".into());
        }
        // An incomplete store from an older version may have no readable
        // certificate at all; then nothing could have been trusted.
        for old_cert in crate::certificate::retired_certificates(&self.paths.certificates)? {
            if checked_ca_scope(&old_cert).as_deref() != Some("none")
                || checked_ca_trust(&old_cert) != Some(false) {
                return Err("Прежнему CA всё ещё доверяет проверенное хранилище ОС или проверить доверие не удалось. Удалите доверие к точному отпечатку и повторите.".into());
            }
            #[cfg(target_os = "macos")]
            if crate::macos_trust::present_in_checked_keychains(&old_cert)? {
                return Err("Прежний CA всё ещё находится в связке Вход или Система. Удалите сертификат по точному отпечатку и повторите.".into());
            }
            #[cfg(target_os = "linux")]
            if crate::certificate::debian_source_trust(&old_cert) != Some(false) {
                return Err("Файл прежнего CA ещё находится в источниках доверия Debian/Ubuntu или их не удалось проверить. Удалите точный файл и обновите сертификаты ОС.".into());
            }
        }
        fs::remove_dir_all(&retired).map_err(|e| format!("Не удалось удалить закрытый ключ прежнего CA: {e}"))?;
        Ok(())
    }

    pub fn restore_interrupted_replacement(&self) -> Result<(), String> {
        let _setup = crate::coordination::lock(&self.paths.proxy, "setup")?;
        let _capture = crate::coordination::lock(&self.paths.proxy, "capture")?;
        crate::certificate::restore_interrupted_replacement(&self.paths.certificates)
    }

    pub fn prepare_certificate(&self, app: &AppHandle) -> Result<(), String> {
        let _setup = crate::coordination::lock(&self.paths.proxy, "setup")?;
        let _guard = self
            .certificate_installation
            .try_lock()
            .map_err(|_| "Подготовка сертификата уже выполняется.")?;
        self.prepare_certificate_at(app, &self.paths.certificates)
    }

    pub fn prepare_replacement(&self, app: &AppHandle) -> Result<(), String> {
        let _setup = crate::coordination::lock(&self.paths.proxy, "setup")?;
        let _guard = self
            .certificate_installation
            .try_lock()
            .map_err(|_| "Подготовка сертификата уже выполняется.")?;
        if !crate::certificate::complete_store(&self.paths.certificates) {
            return Err("Действующий CA повреждён. Новый CA не создавался; сначала проверьте файлы и резервную копию.".into());
        }
        let (pending, retired) = crate::certificate::rotation_paths(&self.paths.certificates)?;
        if fs::symlink_metadata(retired).is_ok() {
            return Err("Сначала завершите очистку доверия к предыдущему CA.".into());
        }
        self.prepare_certificate_at(app, &pending)?;
        let old = crate::certificate::details(&self.paths.certificates.join("mitmproxy-ca-cert.cer"))
            .ok_or("Не удалось прочитать действующий CA.")?;
        let new = crate::certificate::details(&pending.join("mitmproxy-ca-cert.cer"))
            .ok_or("Не удалось прочитать новый CA.")?;
        if old.sha256 == new.sha256 {
            return Err("Новый CA совпадает с действующим; переключение невозможно.".into());
        }
        Ok(())
    }

    fn prepare_certificate_at(&self, app: &AppHandle, destination: &Path) -> Result<(), String> {
        let _ = app;
        let settings = self
            .inner
            .lock()
            .map_err(|_| "Настройки заняты.")?
            .settings
            .clone();
        let is_active = destination == self.paths.certificates.as_path();
        if is_active && abandoned_store(destination) {
            // An older version left a CA that nobody trusts (or an incomplete one).
            // Move it aside for explicit cleanup instead of asking to trust its key.
            crate::certificate::replace_abandoned_store(destination, &settings.hosts)?;
            let mut settings = settings;
            settings.certificate_trust_confirmed = false;
            return self.save_settings(settings).map(|_| ())
                .map_err(|e| format!("Сертификат создан, но не удалось сохранить настройки: {e}"));
        }
        if destination.join(crate::certificate::PUBLIC_CA).is_file() {
            let Some(format) = crate::certificate::store_format(destination) else {
                return Err("Хранилище CA неполное, содержит ссылку или закрытый ключ не соответствует сертификату. Существующие файлы сохранены без изменений.".into());
            };
            let covered = crate::certificate::leaf_hosts(destination).unwrap_or_default();
            if format == crate::certificate::StoreFormat::Legacy
                || settings.hosts.iter().all(|host| covered.contains(host)) {
                return Ok(());
            }
            // A leaf-only store holds no CA key. Before anyone trusts it, a server
            // change may recreate it; once trusted, only the explicit replacement does.
            let certificate = destination.join(crate::certificate::PUBLIC_CA);
            if (is_active && settings.certificate_trust_confirmed)
                || checked_ca_trust(&certificate) != Some(false)
                || checked_ca_scope(&certificate).as_deref() != Some("none") {
                return Err(format!(
                    "Сертификат выпущен для {} и уже доверен. Для другого сервера создайте замену CA в настройках «Захват».",
                    covered.join(", ")));
            }
            fs::remove_dir_all(destination).map_err(|e| format!("Не удалось удалить недоверенный сертификат: {e}"))?;
        }
        crate::certificate::create_leaf_store(&settings.hosts, destination).map(|_| ())
    }
}

fn compatible_external_helper(path: &Path) -> bool {
    static CHECKS: OnceLock<Mutex<HashMap<PathBuf, (u64, Option<SystemTime>, bool)>>> = OnceLock::new();
    let Ok(metadata) = fs::metadata(path) else { return false; };
    let stamp = (metadata.len(), metadata.modified().ok());
    let checks = CHECKS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(cache) = checks.lock() {
        if let Some((length, modified, valid)) = cache.get(path) {
            if *length == stamp.0 && *modified == stamp.1 { return *valid; }
        }
    }
    let valid = probe_helper_version(path);
    if let Ok(mut cache) = checks.lock() {
        cache.insert(path.to_path_buf(), (stamp.0, stamp.1, valid));
    }
    valid
}

fn probe_helper_version(path: &Path) -> bool {
    let Ok(mut output) = tempfile::tempfile() else { return false; };
    let Ok(stdout) = output.try_clone() else { return false; };
    let Ok(stderr) = output.try_clone() else { return false; };
    let mut command = Command::new(path);
    command.arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    let Ok(mut child) = OwnedChild::spawn(&mut command) else { return false; };
    for _ in 0..100 {
        if let Ok(Some(status)) = child.try_wait() {
            drop(child);
            let mut bytes = Vec::new();
            return status.success()
                && output.seek(SeekFrom::Start(0)).is_ok()
                && output.take(64 * 1024).read_to_end(&mut bytes).is_ok()
                && helper_version_matches(&bytes);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    false
}

fn helper_version_matches(output: &[u8]) -> bool {
    String::from_utf8_lossy(output).lines()
        .find_map(|line| line.strip_prefix("Mitmproxy: "))
        .and_then(|line| line.split_whitespace().next()) == Some(installer::VERSION)
}

fn read_capture_state(
    root: &Path,
    run_id: Option<&str>,
    cache: &mut Option<ProxyState>,
) -> ProxyState {
    let Some(run_id) = run_id else {
        return ProxyState::default();
    };
    let path = root.join("runs").join(run_id).join("state.json");
    let fresh = fs::metadata(&path)
        .ok()
        .filter(|m| m.len() < 262_144)
        .and_then(|_| fs::read(&path).ok())
        .and_then(|bytes| serde_json::from_slice::<ProxyState>(&bytes).ok())
        .filter(|state| state.run_id.as_deref() == Some(run_id));
    if let Some(fresh) = fresh {
        *cache = Some(fresh);
    }
    // Preserve the last coherent sample across a short read failure. Its own
    // heartbeat still expires normally; this does not keep dead capture alive.
    cache
        .clone()
        .filter(|state| state.run_id.as_deref() == Some(run_id))
        .unwrap_or_default()
}

fn current_capture_state(
    state: &ProxyState,
    run_id: Option<&str>,
    launcher_alive: bool,
    started_at: f64,
    timestamp: f64,
) -> bool {
    launcher_alive
        && state.pid > 0
        && run_id.is_some()
        && state.run_id.as_deref() == run_id
        && state.running
        && timestamp - state.updated_at < 20.0
        && timestamp >= state.updated_at - 5.0
        && state.started_at >= started_at - 1.0
}

use crate::diagnostics::monitor_capture_output;
#[cfg(test)]
use crate::diagnostics::capture_diagnostic;

fn helper_command(path: &Path) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}

fn signal_stop(child: &mut OwnedChild) {
    #[cfg(target_os = "linux")]
    unsafe {
        // Cancel the owned launcher, Python worker and pending user-level askpass.
        if libc::kill(-(child.id() as i32), libc::SIGTERM) != 0 {
            libc::kill(child.id() as i32, libc::SIGTERM);
        }
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    #[cfg(windows)]
    {
        let _ = child.kill();
    }
}

fn system_proxy_active() -> bool {
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = Command::new("/usr/sbin/scutil").arg("--proxy").output() {
            let text = String::from_utf8_lossy(&output.stdout);
            return [
                "HTTPEnable : 1",
                "HTTPSEnable : 1",
                "SOCKSEnable : 1",
                "ProxyAutoConfigEnable : 1",
            ]
            .iter()
            .any(|key| text.contains(key));
        }
    }
    #[cfg(windows)]
    {
        if let Ok(output) = helper_command(Path::new("reg.exe"))
            .args([
                "query",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
                "/v",
                "ProxyEnable",
            ])
            .output()
        {
            let text = String::from_utf8_lossy(&output.stdout);
            if text
                .lines()
                .any(|line| line.contains("ProxyEnable") && line.trim_end().ends_with("0x1"))
            {
                return true;
            }
        }
    }
    [
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "ALL_PROXY",
        "https_proxy",
        "http_proxy",
        "all_proxy",
    ]
    .iter()
    .any(|key| std::env::var(key).is_ok_and(|v| !v.trim().is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_helper_discovery_accepts_only_the_pinned_version() {
        assert!(helper_version_matches(b"Mitmproxy: 12.3.1 binary\nPython: 3.15.1\n"));
        assert!(!helper_version_matches(b"Mitmproxy: 12.2.0 binary\n"));
        assert!(!helper_version_matches(b"unrelated program 12.3.1\n"));
    }

    #[test]
    fn capture_sessions_ignore_legacy_writers_and_keep_a_coherent_sample() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let legacy = serde_json::json!({"runId":"legacy", "pid":1, "running":true, "updatedAt":100.0, "startedAt":90.0, "profiles":[]});
        fs::write(root.join("state.json"), legacy.to_string()).unwrap();
        let mut cache = None;
        assert!(
            read_capture_state(root, Some("owned"), &mut cache)
                .run_id
                .is_none()
        );
        let current_dir = root.join("runs/owned");
        fs::create_dir_all(&current_dir).unwrap();
        let mut current = legacy.clone();
        current["runId"] = "owned".into();
        fs::write(current_dir.join("state.json"), current.to_string()).unwrap();
        assert_eq!(
            read_capture_state(root, Some("owned"), &mut cache)
                .run_id
                .as_deref(),
            Some("owned")
        );
        // A concurrent write/read interruption must not briefly show startup or fake empty quotas.
        fs::write(current_dir.join("state.json"), b"{").unwrap();
        assert_eq!(
            read_capture_state(root, Some("owned"), &mut cache).updated_at,
            100.0
        );
        fs::write(current_dir.join("state.json"), legacy.to_string()).unwrap();
        assert_eq!(
            read_capture_state(root, Some("owned"), &mut cache)
                .run_id
                .as_deref(),
            Some("owned")
        );
        assert!(
            read_capture_state(root, Some("next"), &mut cache)
                .run_id
                .is_none()
        );
        assert!(!current_capture_state(
            &cache.unwrap(),
            Some("owned"),
            true,
            90.0,
            150.0
        ));
    }
    #[test]
    fn worker_pid_may_differ_but_launch_identity_must_match() {
        let mut state = ProxyState {
            pid: 9999,
            run_id: Some("current".into()),
            running: true,
            updated_at: 105.0,
            started_at: 100.0,
            ..Default::default()
        };
        assert!(current_capture_state(
            &state,
            Some("current"),
            true,
            100.0,
            106.0
        ));
        assert!(!current_capture_state(
            &state,
            Some("old"),
            true,
            100.0,
            106.0
        ));
        assert!(!current_capture_state(
            &state,
            Some("current"),
            false,
            100.0,
            106.0
        ));
        assert!(!current_capture_state(
            &state,
            Some("current"),
            true,
            100.0,
            130.0
        ));
        state.run_id = None;
        assert!(!current_capture_state(&state, None, true, 100.0, 106.0));
    }

    #[test]
    fn capture_output_is_classified_without_retaining_raw_logs() {
        let issue = Arc::new(Mutex::new(None));
        let data = std::io::Cursor::new(
            b"GET /private?token=secret\nsudo: a terminal is required\n".to_vec(),
        );
        monitor_capture_output(data, issue.clone()).join().unwrap();
        let result = issue.lock().unwrap().clone().unwrap();
        assert!(result.contains("права администратора"));
        assert!(!result.contains("secret"));
        assert!(!result.contains("Подключение"));
        assert!(capture_diagnostic("cancelled TLS connection").is_none());
        assert!(capture_diagnostic("sudo: unable to execute askpass program /secret/path")
            .unwrap().contains("графический запрос прав"));
    }

    #[cfg(unix)]
    #[test]
    fn live_settings_validate_before_stopping_and_restart_only_for_capture_changes() {
        let directory = tempfile::tempdir().unwrap();
        let initial = Settings {
            hosts: vec!["gateway.example".into()],
            ..Default::default()
        };
        let shared = Shared {
            paths: Paths {
                config: directory.path().join("settings.json"),
                proxy: directory.path().join("proxy"),
                certificates: directory.path().join("certificates"),
                helper: directory.path().join("helper"),
            },
            inner: Mutex::new(Inner {
                settings: initial.clone(),
                capture_lock: None,
                child: Some(OwnedChild::spawn(Command::new("/bin/sleep").arg("30")).unwrap()),
                stop_at: None,
                started_at: now(),
                error: None,
                capture_issue: Arc::new(Mutex::new(None)),
                run_id: None,
                last_state: None,
                rotation_trial: false,
                extension_check: None,
                approval_seen_at: None,
            }),
            installation: Mutex::new(Installation::default()),
            certificate_installation: Mutex::new(()),
        };
        let mut next = initial.clone();
        next.theme = "dark".into();
        assert!(!shared.save_settings(next.clone()).unwrap());
        assert!(shared.inner.lock().unwrap().stop_at.is_none());
        next.hosts = vec!["https://user:secret@gateway.example".into()];
        assert!(shared.save_settings(next.clone()).is_err());
        assert!(shared.inner.lock().unwrap().stop_at.is_none());
        next.hosts.clear();
        assert!(shared.save_settings(next.clone()).is_err());
        assert!(shared.inner.lock().unwrap().stop_at.is_none());
        next.hosts = vec!["other.example".into()];
        assert!(shared.save_settings(next).unwrap());
        let mut inner = shared.inner.lock().unwrap();
        assert_eq!(inner.settings.hosts, vec!["other.example"]);
        assert!(inner.stop_at.is_some());
        let mut child = inner.child.take().unwrap();
        let _ = child.kill();
        let _ = child.wait();
        drop(inner);
        let mut next = initial;
        next.hosts = vec!["stopped.example".into()];
        assert!(!shared.save_settings(next).unwrap());
    }
    #[test]
    fn ca_creation_preserves_existing_material() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("certificates");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("mitmproxy-ca.pem"), b"existing key").unwrap();
        let hosts = vec!["ai.example.com".to_string()];
        assert!(crate::certificate::create_leaf_store(&hosts, &destination).is_err());
        assert_eq!(fs::read(destination.join("mitmproxy-ca.pem")).unwrap(), b"existing key");
        let empty = root.path().join("empty");
        fs::create_dir(&empty).unwrap();
        crate::certificate::create_leaf_store(&hosts, &empty).unwrap();
        assert_eq!(crate::certificate::leaf_hosts(&empty), Some(hosts.clone()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let dangling = root.path().join("dangling");
            symlink(root.path().join("missing"), &dangling).unwrap();
            assert!(crate::certificate::create_leaf_store(&hosts, &dangling).is_err());
            assert!(fs::symlink_metadata(&dangling).unwrap().file_type().is_symlink());
        }
    }
    #[test]
    fn quota_evidence_is_conservative() {
        let root = tempfile::tempdir().unwrap();
        let proxy = root.path();
        assert!(!quotas_ever_received(proxy));
        let run = proxy.join("runs/1-2");
        fs::create_dir_all(&run).unwrap();
        fs::write(run.join("state.json"), r#"{"runId":"1-2","profiles":[]}"#).unwrap();
        assert!(!quotas_ever_received(proxy));
        fs::create_dir_all(proxy.join("runs/empty")).unwrap();
        assert!(!quotas_ever_received(proxy), "a run that never wrote state proves nothing");
        fs::write(run.join("state.json"), b"{").unwrap();
        assert!(quotas_ever_received(proxy), "unreadable evidence must not allow skipping the trial");
        fs::write(run.join("state.json"), r#"{"profiles":[]}"#).unwrap();
        fs::write(proxy.join("state.json"), r#"{"profiles":[{"host":"corp.example.com"}]}"#).unwrap();
        assert!(quotas_ever_received(proxy), "the pre-run shared state of older versions counts");
    }

    #[test]
    fn automatic_capture_requires_completed_setup() {
        let mut state = AppStatus {
            settings: Settings {
                hosts: vec!["gateway.example".into()],
                certificate_trust_confirmed: true,
                ..Default::default()
            },
            profiles: vec![],
            capture: CaptureStatus::default(),
            setup: SetupStatus {
                helper_path: Some("/helper/mitmdump".into()),
                certificate_exists: true,
                ..Default::default()
            },
            rotation: RotationStatus::default(),
            connection_issue: None,
            now: 0.0,
        };
        assert!(state.setup_complete());
        state.settings.certificate_trust_confirmed = false;
        assert!(!state.setup_complete());
        state.settings.certificate_trust_confirmed = true;
        state.setup.certificate_exists = false;
        assert!(!state.setup_complete());
        state.setup.certificate_exists = true;
        state.setup.helper_path = None;
        assert!(!state.setup_complete());
        state.setup.helper_path = Some("/helper/mitmdump".into());
        state.settings.hosts.clear();
        assert!(!state.setup_complete());
        state.settings.hosts.push("gateway.example".into());
        state.settings.clients.clear();
        assert!(state.setup_complete());
    }
    #[test]
    fn deleted_trust_overrides_a_saved_confirmation() {
        let status = AppStatus {
            settings: Settings {
                hosts: vec!["gateway.example".into()],
                certificate_trust_confirmed: true,
                ..Default::default()
            },
            setup: SetupStatus {
                helper_path: Some("/helper".into()),
                certificate_exists: true,
                certificate_trusted: Some(false),
                ..Default::default()
            },
            profiles: vec![],
            capture: CaptureStatus::default(),
            rotation: RotationStatus::default(),
            connection_issue: None,
            now: 0.0,
        };
        assert!(!status.setup_complete());
    }
    #[test]
    fn hosts_and_client_scope() {
        assert_eq!(
            normalize_host("https://Gateway.Example/claude/v1/messages?beta=true").unwrap(),
            "gateway.example"
        );
        assert!(normalize_host("https://user:secret@gateway.example").is_err());
        assert!(normalize_host("file:///tmp/file").is_err());
        assert_eq!(selector(&[]).unwrap(), "Claude,claude");
        assert_eq!(
            selector(&["Code".into(), "Code".into(), "claude".into()]).unwrap(),
            "Claude,Code,claude"
        );
        for invalid in ["!Claude", "1234", "Code,other", "/usr/bin/node", "a\nb"] {
            assert!(selector(&[invalid.into()]).is_err());
        }
        let mut old = Settings {
            clients: vec!["vscode".into()],
            ..Default::default()
        };
        migrate_process_settings(&mut old);
        assert_eq!(
            selector(&old.additional_processes).unwrap(),
            "Claude,Code,claude,code,extensionHost"
        );
        old.additional_processes.clear();
        migrate_process_settings(&mut old);
        assert!(old.additional_processes.is_empty());
    }
}
