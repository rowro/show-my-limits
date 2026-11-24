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
const HELP: &str = "Show My Limits — terminal monitor\n\n  sml setup                  Install mitmproxy and prepare the shared CA\n  sml trust                  Explicitly install/trust the shared CA\n  sml run [--host DOMAIN]     Capture and display quotas (Ctrl+C stops capture)\n  sml watch                  Display an existing GUI/CLI capture (Ctrl+C exits viewer)\n  sml status [--json]         Print current quotas\n  sml doctor                 Show paths, helper and certificate identity\n  sml doctor --share         Show a report without paths, hosts or traffic\n\nrun also accepts repeatable --process NAME.\n--host/--process apply only to this run; saved GUI settings are preserved.\nThe GUI and CLI share data and certificates. No Python or desktop is required.\n";
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
        if !certificate::complete_store(&paths.ca()) {
            return Err("The CA store is incomplete, symlinked or its certificate files do not match. Existing material was preserved; inspect it before restoring from backup.".into());
        }
        println!("Reusing certificate: {}", paths.cert().display());
        return Ok(());
    }
    if fs::symlink_metadata(paths.ca()).is_ok()
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
        || !certificate::complete_store(&generated)
    {
        return Err("CA initialization produced incomplete material".into());
    }
    certificate::secure_new_store(&generated)?;
    if fs::symlink_metadata(paths.ca()).is_ok() {
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
        let hint = match state.health.as_ref().map(|health| health.kind.as_str()) {
            Some("response_received") => "A response from the selected server was captured, but it had no quota headers. Check the server and account.\n",
            Some("request_seen") => "A request to the selected server was captured, but no response has arrived yet. Check network and TLS.\n",
            Some("certificate_client" | "certificate_server") => "A TLS certificate was rejected. Check CA trust and restart the client.\n",
            Some("network_ipv6" | "network_unreachable") => "The selected server could not be reached. Check routing and IPv6.\n",
            Some("tls_client" | "tls_server") => "TLS did not complete. Check the certificate and network.\n",
            _ => "No request to the selected server has been observed. Check the configured host and client process, then send a message.\n",
        };
        out.push_str(hint);
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
    if matches!(command, "setup" | "trust") && args.len() != 1 {
        return Err(format!("Usage: sml {command}"));
    }
    if command == "doctor" && args.len() != 1 && args.as_slice() != ["doctor", "--share"] {
        return Err("Usage: sml doctor [--share]".into());
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
                    "This command only prints instructions; it has not changed trust.\nExplicit system-wide trust installation for Debian/Ubuntu (review and run). Other users and applications may also trust this CA:\n  sudo install -m 644 '{}' /usr/local/share/ca-certificates/show-my-limits.crt\n  sudo update-ca-certificates\nRun sml doctor afterward to check the Debian CA bundle. Clients may use a different trust store.\nOther distributions: install this CA using your distribution trust tools.\nRestart your client afterwards.",
                    paths.cert().display().to_string().replace('\'', "'\\''")
                );
                Ok(())
            }
            #[cfg(target_os = "macos")]
            {
                if certificate::details(&paths.cert()).is_none() {
                    return Err("Run sml setup first".into());
                }
                if !trusted(&paths.cert())? {
                    if !io::stdin().is_terminal() {
                        return Err(
                            "Run sml trust in an interactive terminal to authorize CA trust."
                                .into(),
                        );
                    }
                    let output = Command::new("/usr/bin/security")
                        .args(["default-keychain", "-d", "user"])
                        .output()
                        .map_err(|e| e.to_string())?;
                    if !output.status.success() {
                        return Err("Cannot find the current user's default keychain".into());
                    }
                    let keychain = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
                    let keychain = Path::new(keychain.trim().trim_matches('"'));
                    if !keychain.is_file() {
                        return Err("The current user's default keychain is unavailable".into());
                    }
                    // No administrator domain or sudo: trust only for the current user.
                    let status = Command::new("/usr/bin/security")
                        .args(["add-trusted-cert", "-r", "trustRoot", "-p", "ssl", "-k"])
                        .arg(keychain)
                        .arg(paths.cert())
                        .status()
                        .map_err(|e| e.to_string())?;
                    if !status.success() || !trusted(&paths.cert())? {
                        return Err("CA trust was not confirmed; capture was not started. Import the CA into your login keychain and enable SSL trust there.".into());
                    }
                }
                println!("Certificate is trusted. Fully restart your client.");
                Ok(())
            }
            #[cfg(target_os = "windows")]
            {
                certificate::install(&paths.cert())?;
                println!("Certificate installed. Fully restart your client.");
                Ok(())
            }
        }
        "doctor" => {
            if args.get(1).is_some_and(|arg| arg == "--share") {
                let session = match std::env::var("XDG_SESSION_TYPE").ok().as_deref() {
                    Some("wayland") => "wayland",
                    Some("x11") => "x11",
                    _ => "other/unknown",
                };
                let cert = paths.cert();
                let trusted = if cert.is_file() {
                    #[cfg(target_os = "linux")]
                    { match certificate::debian_system_trust(&cert) { Some(true) => "present in Debian CA bundle", Some(false) => "missing from Debian CA bundle", None => "not checked" } }
                    #[cfg(not(target_os = "linux"))]
                    { match trusted(&cert) { Ok(true) => "yes", Ok(false) => "no", Err(_) => "not checked" } }
                } else { "no" };
                #[cfg(target_os = "macos")]
                let scope = if cert.is_file() { macos_trust::trust_scope(&cert).unwrap_or("unknown") } else { "none" };
                #[cfg(target_os = "windows")]
                let scope = if cert.is_file() { windows_trust::trust_scope(&cert).unwrap_or("unknown") } else { "none" };
                #[cfg(target_os = "linux")]
                let scope = if cert.is_file() { match certificate::debian_system_trust(&cert) { Some(true) => "system-wide (Debian CA bundle)", Some(false) => "not in Debian CA bundle", None => "unknown" } } else { "none" };
                #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
                let scope = "unknown";
                let live = snapshot(&paths);
                let fresh = live.as_ref().is_some_and(|state| state.profiles.iter().any(|profile| (0.0..600.0).contains(&(now() - profile.observed_at))));
                println!("sml {}\nOS: {}\nSession: {}\nHelper: {}\nCA: {}\nCA trusted: {}\nCA scope: {}\nCA file permissions: {}\nCapture live: {}\nFresh quota: {}",
                    env!("CARGO_PKG_VERSION"), std::env::consts::OS, session,
                    if paths.helper().is_ok() { "present" } else { "missing" },
                    if cert.is_file() { "present" } else { "missing" }, trusted, scope,
                    if certificate::security_issue(&paths.ca()).is_some() { "needs review" } else { "no issue detected" },
                    if live.is_some() { "yes" } else { "no" },
                    if fresh { "yes" } else { "no" });
                return Ok(());
            }
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
                println!("CA: {}\nSHA-256: {}", c.name, c.sha256);
                if let Some(issue) = certificate::security_issue(&paths.ca()) {
                    println!("CA security: {issue}");
                }
                #[cfg(target_os = "linux")]
                println!("Debian system CA bundle: {} (client may use a different trust store).",
                    match certificate::debian_system_trust(&paths.cert()) {
                        Some(true) => "contains this CA",
                        Some(false) => "does not contain this CA",
                        None => "could not be checked",
                    });
                #[cfg(not(target_os = "linux"))]
                println!("Trusted by OS: {}", trusted(&paths.cert())?);
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
                if !certificate::complete_store(&paths.ca()) {
                    return Err("The CA store is incomplete, symlinked or its certificate files do not match. Existing material was preserved; inspect it before restoring from backup.".into());
                }
                if !trusted(&paths.cert())? {
                    return Err(
                        "The CA is not trusted. Run sml trust, then restart your client.".into(),
                    );
                }
                #[cfg(target_os = "linux")]
                if certificate::debian_system_trust(&paths.cert()) == Some(false) {
                    eprintln!("CA is absent from the Debian/Ubuntu system bundle. If this client uses that bundle, run sml trust and its printed commands, then restart the client. Other trust stores are not checked.");
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
                eprintln!("Selected server: {}", config.hosts.join(", "));
                eprintln!("Client processes: Claude, claude{}", if config.additional_processes.is_empty() { String::new() } else { format!(", {}", config.additional_processes.join(", ")) });
                eprintln!(
                    "Capture process launched. Wait for readiness, then restart your client and send a message. Ctrl+C stops only this capture."
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
    if terminal {
        print!("\x1b[2J");
    }
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
            return Err(issue.lock().unwrap_or_else(|p| p.into_inner()).clone().unwrap_or_else(||
                "Capture did not become ready in 90 seconds. Check OS capture permissions.".into()
            ));
        }
        let text = if capture.is_some() && state.is_none() {
            "Show My Limits\nStarting capture… Waiting for the helper and OS permission.\n".into()
        } else {
            display(state.as_ref())
        };
        if text != previous {
            if terminal {
                print!("\x1b[H");
            }
            print!("{text}");
            if terminal {
                print!("\x1b[J");
            }
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
        unrelated.kill().unwrap();
        unrelated.wait().unwrap();
    }
}
