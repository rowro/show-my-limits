//! Read-only state of mitmproxy's macOS network extension.
//!
//! `local` capture on macOS starts "Mitmproxy Redirector", which asks macOS to
//! activate a network extension and waits without a timeout until the user
//! allows it. mitmdump prints nothing meanwhile, so the only signal is the
//! system extension list. This module never activates, approves or changes it.
use std::process::{Command, Stdio};

const EXTENSION_ID: &str = "org.mitmproxy.macos-redirector.network-extension";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtensionState {
    /// macOS waits for the user to allow the extension in System Settings.
    WaitingForUser,
    Enabled,
    /// Registered in another state (for example, being replaced or removed).
    Other,
    /// Never registered: the redirector has not requested activation yet.
    Missing,
}

pub fn parse(listing: &str) -> ExtensionState {
    let lines: Vec<&str> = listing.lines().filter(|line| line.contains(EXTENSION_ID)).collect();
    if lines.is_empty() {
        ExtensionState::Missing
    } else if lines.iter().any(|line| line.contains("waiting for user")) {
        ExtensionState::WaitingForUser
    } else if lines.iter().any(|line| line.contains("[activated enabled]")) {
        ExtensionState::Enabled
    } else {
        ExtensionState::Other
    }
}

pub fn state() -> Option<ExtensionState> {
    let output = Command::new("/usr/bin/systemextensionsctl")
        .arg("list")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output.status.success().then(|| parse(&String::from_utf8_lossy(&output.stdout)))
}

/// Open the pane where macOS lists network extensions (macOS 15+; older
/// versions open System Settings at the closest available pane).
pub fn open_settings() -> Result<(), String> {
    Command::new("/usr/bin/open")
        .arg("x-apple.systempreferences:com.apple.LoginItems-Settings.extension")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| e.to_string())
        .and_then(|status| status.success().then_some(()).ok_or_else(|| "Не удалось открыть Системные настройки.".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_extension_state_from_listing() {
        let header = "1 extension(s)\n--- com.apple.system_extension.network_extension\nenabled\tactive\tteamID\tbundleID (version)\tname\t[state]\n";
        assert_eq!(parse(header), ExtensionState::Missing);
        assert_eq!(parse(&format!("{header}\t\tS8XHQB96PW\t{EXTENSION_ID} (2.0/1)\tnetwork-extension\t[activated waiting for user]\n")), ExtensionState::WaitingForUser);
        assert_eq!(parse(&format!("{header}*\t*\tS8XHQB96PW\t{EXTENSION_ID} (2.0/1)\tnetwork-extension\t[activated enabled]\n")), ExtensionState::Enabled);
        assert_eq!(parse(&format!("{header}\t\tS8XHQB96PW\t{EXTENSION_ID} (1.0/1)\tnetwork-extension\t[terminated waiting to uninstall on reboot]\n")), ExtensionState::Other);
        // A replacement in progress: the waiting entry wins.
        assert_eq!(parse(&format!("{header}*\t*\tS8XHQB96PW\t{EXTENSION_ID} (1.0/1)\tnetwork-extension\t[activated enabled]\n\t\tS8XHQB96PW\t{EXTENSION_ID} (2.0/1)\tnetwork-extension\t[activated waiting for user]\n")), ExtensionState::WaitingForUser);
    }
}
