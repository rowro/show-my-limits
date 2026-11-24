//! Deliberately small, read-only quota projection for native desktop integrations.
use crate::engine::AppStatus;
use serde_json::{Value, json};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const PROTOCOL: u32 = 1;
#[derive(Default)]
pub struct DesktopState {
    snapshot: Mutex<Value>,
    lease: Mutex<Option<Instant>>,
}
impl DesktopState {
    pub fn update(&self, status: &AppStatus) {
        let profile = status
            .profiles
            .iter()
            .find(|p| Some(&p.id) == status.settings.selected_profile.as_ref())
            .or_else(|| status.profiles.first());
        let profile = profile.map(|p| {
            json!({
                "host": p.host, "observedAt": p.observed_at,
                "windows": {"5h": p.windows.get("5h"), "7d": p.windows.get("7d")}
            })
        });
        *self.snapshot.lock().unwrap_or_else(|p| p.into_inner()) = json!({
            "protocol": PROTOCOL, "now": status.now,
            "phase": status.capture.phase, "setupComplete": status.setup_complete(),
            "connectionIssue": status.connection_issue.is_some(), "profile": profile
        });
    }
    pub fn snapshot(&self) -> String {
        self.snapshot
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .to_string()
    }
    pub fn heartbeat(&self, protocol: u32) -> bool {
        if protocol != PROTOCOL {
            return false;
        }
        *self.lease.lock().unwrap_or_else(|p| p.into_inner()) = Some(Instant::now());
        true
    }
    pub fn detach(&self) {
        *self.lease.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
    pub fn active(&self) -> bool {
        self.active_at(Instant::now())
    }
    fn active_at(&self, now: Instant) -> bool {
        self.lease
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some_and(|at| now.saturating_duration_since(at) < Duration::from_secs(15))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fallback_returns_after_extension_disappears_or_protocol_mismatches() {
        let state = DesktopState::default();
        assert!(!state.heartbeat(999));
        assert!(!state.active());
        assert!(state.heartbeat(PROTOCOL));
        assert!(state.active());
        assert!(!state.active_at(Instant::now() + Duration::from_secs(16)));
        state.detach();
        assert!(!state.active());
    }
    #[test]
    fn desktop_projection_excludes_paths_settings_and_process_identifiers() {
        let state = DesktopState::default();
        let status = AppStatus {
            settings: Default::default(),
            profiles: vec![],
            capture: Default::default(),
            setup: Default::default(),
            rotation: Default::default(),
            connection_issue: None,
            now: 1000.0,
        };
        state.update(&status);
        let snapshot: Value = serde_json::from_str(&state.snapshot()).unwrap();
        assert_eq!(snapshot["protocol"], 1);
        let mut fields: Vec<_> = snapshot
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        fields.sort();
        assert_eq!(
            fields,
            [
                "connectionIssue",
                "now",
                "phase",
                "profile",
                "protocol",
                "setupComplete"
            ]
        );
    }
}
