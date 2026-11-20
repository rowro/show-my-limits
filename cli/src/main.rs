//! A headless frontend. Shared modules intentionally have no Tauri/WebView dependency.
#[path = "../../src-tauri/src/capture_conflict.rs"]
mod capture_conflict;
#[path = "../../src-tauri/src/certificate.rs"]
mod certificate;
#[path = "../../src-tauri/src/coordination.rs"]
mod coordination;
#[path = "../../src-tauri/src/installer.rs"]
mod installer;
#[cfg(target_os = "macos")]
#[path = "../../src-tauri/src/macos_trust.rs"]
mod macos_trust;
#[path = "../../src-tauri/src/process_tree.rs"]
mod process_tree;
#[path = "../../src-tauri/src/quota.rs"]
mod quota;
#[path = "../../src-tauri/src/settings.rs"]
mod settings;
#[cfg(target_os = "windows")]
#[path = "../../src-tauri/src/windows_trust.rs"]
mod windows_trust;

#[path = "../../src-tauri/src/diagnostics.rs"]
mod diagnostics;
use process_tree::OwnedChild;
use std::{
    fs,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
const ID: &str = "io.github.rowro.showmylimits";
const HELP: &str = "Show My Limits — terminal monitor\n\n  sml setup                  Install mitmproxy and prepare the shared CA\n  sml trust                  Explicitly install/trust the shared CA\n  sml run [--host DOMAIN]     Capture and display quotas (Ctrl+C stops capture)\n  sml watch                  Display an existing GUI/CLI capture (Ctrl+C exits viewer)\n  sml status [--json]         Print current quotas\n  sml doctor                 Show paths, helper and certificate identity\n\nrun also accepts repeatable --process NAME.\n--host/--process apply only to this run; saved GUI settings are preserved.\nThe GUI and CLI share data and certificates. No Python or desktop is required.\n";
fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
struct Paths {
    data: PathBuf,
    config: PathBuf,
}
impl Paths {
    fn new() -> Result<Self, String> {
        Ok(Self {
            data: dirs::data_dir().ok_or("No user data directory")?.join(ID),
            config: dirs::config_dir()
                .ok_or("No user config directory")?
                .join(ID)
                .join("settings.json"),
        })
    }
    fn proxy(&self) -> PathBuf {
        self.data.join("proxy")
    }
    fn ca(&self) -> PathBuf {
        self.data.join("certificates")
    }
    fn cert(&self) -> PathBuf {
        self.ca().join("mitmproxy-ca-cert.cer")
    }
    fn settings(&self) -> Result<settings::Settings, String> {
        match fs::read(&self.config) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|e| format!("Cannot read settings: {e}"))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(settings::Settings::default()),
            Err(e) => Err(e.to_string()),
        }
    }
    fn helper(&self) -> Result<PathBuf, String> {
        let s = self.settings()?;
        if !s.helper_path.is_empty() && Path::new(&s.helper_path).is_file() {
            return Ok(s.helper_path.into());
        }
        if let Some(path) = installer::installed_path(&self.data.join("mitmproxy")) {
            return Ok(path);
        }
        let exe = if cfg!(windows) {
            "mitmdump.exe"
        } else {
            "mitmdump"
        };
        for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
            let path = dir.join(exe);
            if path.is_file() {
                return Ok(path);
            }
        }
        Err("mitmproxy is missing. Run sml setup.".into())
    }
}
fn trusted(cert: &Path) -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        macos_trust::is_trusted(cert)
    }
    #[cfg(target_os = "windows")]
    {
        windows_trust::is_trusted(cert)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Ok(cert.is_file())
    }
}
fn setup(paths: &Paths) -> Result<(), String> {
    let _guard = coordination::lock(&paths.proxy(), "setup")?;
    let helper = match paths.helper() {
        Ok(p) => p,
        Err(_) => installer::install_to(&paths.data.join("mitmproxy"), |p| {
            eprintln!("mitmproxy: {}", p.phase);
        })?,
    };
    if paths.cert().is_file() {
        println!("Reusing certificate: {}", paths.cert().display());
        return Ok(());
    }
    if paths.ca().exists()
        && fs::read_dir(paths.ca())
            .map_err(|e| e.to_string())?
            .next()
            .is_some()
    {
        return Err("Existing CA material is incomplete; preserved without changes. Repair it before setup.".into());
    }
    let stage = tempfile::tempdir_in(&paths.data).map_err(|e| e.to_string())?;
    let addon = stage.path().join("initialize_ca.py");
    fs::write(&addon, include_str!("../../proxy/initialize_ca.py")).map_err(|e| e.to_string())?;
    let generated = stage.path().join("generated");
    let mut cmd = Command::new(helper);
    cmd.args([
        "--mode",
        "regular",
        "--listen-host",
        "127.0.0.1",
        "--listen-port",
        "0",
        "--set",
        "flow_detail=0",
        "--set",
        "termlog_verbosity=error",
        "--set",
    ])
    .arg(format!(
        "confdir={}",
        stage.path().join("bootstrap").display()
    ))
    .arg("-s")
    .arg(addon)
    .env("SML_CA_OUTPUT", &generated)
    .env("PYTHONDONTWRITEBYTECODE", "1")
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null());
    let mut child = Managed::spawn(&mut cmd)?;
    let deadline = now() + 15.0;
    loop {
        if let Some(status) = child.child.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                return Err("CA initialization failed".into());
            }
            break;
        }
        if now() > deadline {
            return Err("CA initialization timed out".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if !certificate::details(&generated.join("mitmproxy-ca-cert.cer"))
        .is_some_and(|c| c.name == "limits-mitmproxy")
        || !generated.join("mitmproxy-ca.pem").is_file()
        || !generated.join("mitmproxy-dhparam.pem").is_file()
    {
        return Err("CA initialization produced incomplete material".into());
    }
    if paths.ca().exists() {
        fs::remove_dir(paths.ca()).map_err(|_| "Existing CA material was preserved")?;
    }
    fs::rename(generated, paths.ca()).map_err(|e| e.to_string())?;
    println!("CA prepared. Run sml trust, then restart the client.");
    Ok(())
}
// Normal exits, errors and Ctrl+C terminate only our own helper tree.
struct Managed {
    child: OwnedChild,
}
impl Managed {
    fn spawn(cmd: &mut Command) -> Result<Self, String> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        Ok(Self {
            child: OwnedChild::spawn(cmd).map_err(|e| e.to_string())?,
        })
    }
}
impl Drop for Managed {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_some() {
            return;
        }
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGTERM);
        }
        #[cfg(windows)]
        {
            let _ = self.child.kill();
        }
        for _ in 0..30 {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn snapshot(paths: &Paths) -> Option<quota::ProxyState> {
    coordination::active_state(&paths.proxy(), now())
}
fn display(state: Option<&quota::ProxyState>) -> String {
    let mut out = String::from("Show My Limits\n");
    let Some(state) = state else {
        return out + "Waiting for a live capture. Start the app or sml run.\n";
    };
    if state.profiles.is_empty() {
        out.push_str(
            "Waiting for Claude: restart the client, send a message and wait for its response.\n",
        );
    }
    for profile in &state.profiles {
        out.push_str(&format!("\n{}\n", profile.host));
        for key in ["5h", "7d"] {
            let Some(window) = profile.windows.get(key) else {
                continue;
            };
            let remaining = window.remaining(now());
            let title = profile.window_title(key, now());
            let used = window
                .utilization
                .map(|v| format!("{:.0}%", v * 100.0))
                .unwrap_or("—".into());
            let bar = remaining
                .map(|v| {
                    let n = (v.clamp(0.0, 1.0) * 20.0).round() as usize;
                    format!("[{}{}]", "#".repeat(n), "-".repeat(20 - n))
                })
                .unwrap_or_default();
            let reset = window
                .reset
                .map(|t| {
                    let mins = ((t - now()).max(0.0) / 60.0).ceil() as u64;
                    format!("reset in {}h {}m (Unix {:.0})", mins / 60, mins % 60, t)
                })
                .unwrap_or_default();
            out.push_str(&format!(
                "  {key}: {title} remaining {bar} | used {used} | {reset}\n"
            ));
        }
        out.push_str(&format!(
            "  Updated {:.0}s ago\n",
            (now() - profile.observed_at).max(0.0)
        ));
    }
    if let Some(health) = state.health.as_ref().filter(|h| h.kind != "quota_received") {
        out.push_str(&format!("Diagnostic: {} ({})\n", health.kind, health.host));
    }
    out
}
fn main() {
    if let Err(e) = execute() {
        eprintln!("sml: {e}");
        std::process::exit(1);
    }
}
fn execute() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("help");
    if matches!(command, "help" | "--help" | "-h") {
        print!("{HELP}");
        return Ok(());
    }
    if matches!(command, "--version" | "-V") {
        println!("sml {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if matches!(command, "setup" | "trust" | "doctor") && args.len() != 1 {
        return Err(format!("Usage: sml {command}"));
    }
    let paths = Paths::new()?;
    match command {
        "setup" => setup(&paths),
        "trust" => {
            #[cfg(target_os = "linux")]
            {
                if !paths.cert().is_file() {
                    return Err("Run sml setup first".into());
                }
                println!(
                    "Explicit trust installation for Debian/Ubuntu (review and run):\n  sudo install -m 644 '{}' /usr/local/share/ca-certificates/show-my-limits.crt\n  sudo update-ca-certificates\nOther distributions: install this CA using your distribution trust tools.\nRestart your client afterwards.",
                    paths.cert().display().to_string().replace('\'', "'\\''")
                );
                Ok(())
            }
            #[cfg(not(target_os = "linux"))]
            {
                certificate::install(&paths.cert())?;
                println!("Certificate installed. Fully restart your client.");
                Ok(())
            }
        }
        "doctor" => {
            println!(
                "Data: {}\nSettings: {}\nHelper: {}",
                paths.data.display(),
                paths.config.display(),
                paths
                    .helper()
                    .map(|p| p.display().to_string())
                    .unwrap_or("missing".into())
            );
            if let Some(c) = certificate::details(&paths.cert()) {
                println!(
                    "CA: {}\nSHA-256: {}\nTrusted: {}",
                    c.name,
                    c.sha256,
                    trusted(&paths.cert())?
                );
            } else {
                println!("CA: missing");
            }
            Ok(())
        }
        "status" => {
            if args.iter().skip(1).any(|a| a != "--json") {
                return Err("Usage: sml status [--json]".into());
            }
            let state = snapshot(&paths);
            if args.iter().any(|a| a == "--json") {
                println!("{}",serde_json::to_string(&state.map(|s|serde_json::json!({"runId":s.run_id,"running":s.running,"updatedAt":s.updated_at,"profiles":s.profiles,"health":s.health}))).map_err(|e|e.to_string())?);
            } else {
                print!("{}", display(state.as_ref()));
            }
            Ok(())
        }
        "run" | "watch" => run(&paths, &args),
        _ => Err(format!("Unknown command: {command}\n{HELP}")),
    }
}
fn run(paths: &Paths, args: &[String]) -> Result<(), String> {
    let watch = args[0] == "watch";
    let mut config = paths.settings()?;
    let mut i = 1;
    let mut hosts = vec![];
    let mut extra = vec![];
    while i < args.len() {
        match args[i].as_str() {
            "--host" | "--process" if !watch => {
                let flag = &args[i];
                i += 1;
                let value = args.get(i).ok_or("Missing option value")?;
                if flag == "--host" {
                    hosts.push(settings::normalize_host(value)?);
                } else {
                    extra.push(value.clone());
                }
            }
            _ => return Err(format!("Unknown option: {}", args[i])),
        }
        i += 1;
    }
    if !hosts.is_empty() {
        config.hosts = hosts;
    }
    config.additional_processes.extend(extra);
    let processes = settings::selector(&config.additional_processes)?;
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    ctrlc::set_handler(move || {
        stopped.store(true, Ordering::SeqCst);
    })
    .map_err(|e| e.to_string())?;
    let mut owner = None;
    let mut capture = None;
    let issue = Arc::new(std::sync::Mutex::new(None));
    if !watch {
        match coordination::lock(&paths.proxy(), "capture") {
            Ok(lock) => {
                if config.hosts.is_empty() {
                    return Err("Provide --host DOMAIN, or configure a server in the app.".into());
                }
                for host in &mut config.hosts {
                    *host = settings::normalize_host(host)?;
                }
                if !paths.cert().is_file() {
                    return Err("Run sml setup and sml trust first".into());
                }
                if !trusted(&paths.cert())? {
                    return Err(
                        "The CA is not trusted. Run sml trust, then restart your client.".into(),
                    );
                }
                capture_conflict::ensure_exclusive()?;
                let helper = paths.helper()?;
                let run = format!(
                    "{}-{}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos()
                );
                let state_dir = paths.proxy().join("runs").join(&run);
                fs::create_dir_all(&state_dir).map_err(|e| e.to_string())?;
                let addon = state_dir.join("limits_addon.py");
                fs::write(&addon, include_str!("../../proxy/limits_addon.py"))
                    .map_err(|e| e.to_string())?;
                coordination::publish_run(&paths.proxy(), &run)?;
                let hosts = config
                    .hosts
                    .iter()
                    .map(|h| format!("{}(?::[0-9]+)?", regex::escape(h)))
                    .collect::<Vec<_>>()
                    .join("|");
                let mut cmd = Command::new(helper);
                cmd.args([
                    "--mode",
                    &format!("local:{processes}"),
                    "--allow-hosts",
                    &format!("^(?:{hosts})$"),
                    "--set",
                    &format!("confdir={}", paths.ca().display()),
                    "--set",
                    "flow_detail=0",
                    "--set",
                    "termlog_verbosity=error",
                    "-s",
                ])
                .arg(addon)
                .env("SML_RUN_ID", &run)
                .env("SML_STATE_DIR", &state_dir)
                .env(
                    "SML_HOSTS",
                    serde_json::to_string(&config.hosts).map_err(|e| e.to_string())?,
                )
                .env("PYTHONDONTWRITEBYTECODE", "1")
                .stdout(Stdio::null())
                .stderr(Stdio::piped());
                // sudo uses the actual terminal; never pipe or store passwords.
                let mut child = Managed::spawn(&mut cmd)?;
                if let Some(stderr) = child.child.stderr.take() {
                    diagnostics::monitor_capture_output(stderr, issue.clone());
                }
                capture = Some(child);
                owner = Some(lock);
                eprintln!(
                    "Capture started. Restart your client and send a message. Ctrl+C stops only this capture."
                );
            }
            Err(e) => {
                if snapshot(paths).is_none() {
                    return Err(e);
                }
                eprintln!(
                    "Using an existing capture. Ctrl+C exits this viewer; capture keeps running."
                );
            }
        }
    }
    let terminal = io::stdout().is_terminal();
    if terminal { print!("\x1b[2J"); }
    let mut previous = String::new();
    let deadline = now() + 90.0;
    while !stop.load(Ordering::SeqCst) {
        if let Some(c) = capture.as_mut() {
            if let Some(status) = c.child.try_wait().map_err(|e| e.to_string())? {
                return Err(issue.lock().unwrap_or_else(|p|p.into_inner()).clone().unwrap_or_else(|| format!("Capture exited ({status}). Check capture permissions and certificate trust.")));
            }
        }
        let state = snapshot(paths);
        if capture.is_some() && now() > deadline && state.is_none() {
            return Err(
                "Capture did not become ready in 90 seconds. Check OS capture permissions.".into(),
            );
        }
        let text = if capture.is_some() && state.is_none() {
            "Show My Limits\nStarting capture… Waiting for the helper and OS permission.\n".into()
        } else { display(state.as_ref()) };
        if text != previous {
            if terminal {
                print!("\x1b[H");
            }
            print!("{text}");
            if terminal { print!("\x1b[J"); }
            io::stdout().flush().map_err(|e| e.to_string())?;
            previous = text;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    drop(capture);
    drop(owner);
    Ok(())
}

#[cfg(test)]
mod cli_tests {
    use super::*;
    #[test]
    fn live_snapshot_has_remaining_spent_and_reset() {
        let time = now();
        let state: quota::ProxyState = serde_json::from_value(serde_json::json!({
            "runId":"1-2", "pid":1, "running":true, "updatedAt":time, "startedAt":time,
            "health":null, "profiles":[{"id":"mock", "host":"mock.example", "observedAt":time,
                "windows":{"5h":{"utilization":0.35,"reset":time+3600.0},"7d":{"utilization":0.42,"reset":time+7200.0}}}]
        })).unwrap();
        let text = display(Some(&state));
        assert!(text.contains("5h: 65% remaining"));
        assert!(text.contains("7d: 58% remaining"));
        assert!(text.contains("used 42%"));
        assert!(text.contains("reset in"));
        assert!(!text.contains("Diagnostic:"));
    }
    #[cfg(unix)]
    #[test]
    fn dropping_owned_helper_does_not_stop_an_unrelated_process() {
        let mut unrelated = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let mut owned = Managed::spawn(Command::new("/bin/sleep").arg("30")).unwrap();
        assert!(owned.child.try_wait().unwrap().is_none());
        drop(owned);
        assert!(unrelated.try_wait().unwrap().is_none());
        unrelated.kill().unwrap(); unrelated.wait().unwrap();
    }
}
