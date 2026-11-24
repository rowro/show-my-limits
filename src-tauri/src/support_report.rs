use crate::engine::AppStatus;

pub fn render(status: &AppStatus) -> String {
    let session = match std::env::var("XDG_SESSION_TYPE").ok().as_deref() {
        Some("wayland") => "wayland",
        Some("x11") => "x11",
        _ => "other/unknown",
    };
    let trust = match status.setup.certificate_trusted {
        Some(true) => "yes",
        Some(false) => "no",
        None => "not checked",
    };
    let scope = match status.setup.certificate_trust_scope.as_deref() {
        Some("user") => "current user",
        Some("system") => "system-wide",
        Some("user+system") => "current user + system-wide",
        Some("none") => "not found in checked trust stores",
        _ => "unknown",
    };
    let fresh = status.profiles.iter().any(|profile| {
        (0.0..600.0).contains(&(status.now - profile.observed_at))
    });
    let ca_lifetime = match status.setup.certificate_expires_at {
        Some(expiry) if expiry as f64 <= status.now => "expired",
        Some(expiry) if expiry as f64 - status.now <= 30.0 * 86400.0 => "30 days or less",
        Some(_) => "more than 30 days",
        None => "unknown",
    };
    let reason = if status.setup.helper_path.is_none() {
        "helper_missing: install mitmproxy in the setup wizard"
    } else if !status.setup.certificate_exists {
        "ca_missing: create the local CA in the setup wizard"
    } else if status.setup.certificate_security_issue.is_some() {
        "ca_store_unsafe: inspect the CA files and restore a matching key/certificate pair"
    } else if status.setup.certificate_trusted == Some(false) {
        "ca_not_trusted: authorize trust, then restart the client"
    } else if let Some(error) = status.capture.error.as_deref() {
        if error.contains("не подтвердил готовность за 90 секунд") {
            "capture_timeout: helper did not report readiness in 90 seconds"
        } else if error.contains("права администратора") || error.contains("графический запрос прав") {
            "capture_permission: check the OS authorization prompt"
        } else {
            "capture_error: inspect the error shown in the app"
        }
    } else if status.connection_issue.is_some() {
        match status.capture.observation.as_deref() {
            Some("certificate_client" | "certificate_server") => "certificate_rejected: check CA trust and restart the client",
            Some("network_ipv6") => "network_ipv6: route to selected server unavailable over IPv6",
            Some("network_unreachable") => "network_unreachable: route to selected server unavailable",
            Some("tls_client" | "tls_server") => "tls_incomplete: TLS handshake did not complete",
            _ => "connection_error: inspect the connection message in the app",
        }
    } else if status.capture.phase == "starting" {
        "capture_starting: wait for the helper to become ready"
    } else if status.capture.phase == "stopped" {
        "capture_stopped: start capture"
    } else if !fresh {
        match status.capture.observation.as_deref() {
            Some("response_received") => "response_without_quota: target replied without quota headers",
            Some("request_seen") => "request_without_response: target request was intercepted, but no response arrived",
            _ => "no_matching_response: check client process and selected server",
        }
    } else {
        "quota_fresh"
    };
    format!(
        "Show My Limits {}\nOS: {}\nSession: {}\nHelper: {}\nCA: {}\nCA trusted: {}\nCA scope: {}\nCA lifetime: {}\nCA store security: {}\nCapture: {}\nFresh quota: {}\nStatus: {}",
        env!("CARGO_PKG_VERSION"),
        status.setup.platform,
        session,
        if status.setup.helper_path.is_some() { "present" } else { "missing" },
        if status.setup.certificate_exists { "present" } else { "missing" },
        trust,
        scope,
        ca_lifetime,
        if status.setup.certificate_security_issue.is_some() { "needs review" } else { "no issue detected" },
        status.capture.phase,
        if fresh { "yes" } else { "no" },
        reason,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn report_does_not_expose_paths_hosts_or_raw_errors() {
        let mut status = AppStatus {
            settings: Default::default(), profiles: vec![], capture: Default::default(),
            setup: Default::default(), connection_issue: Some("secret.example".into()), now: 1000.0,
        };
        status.settings.hosts = vec!["secret.example".into()];
        status.setup.helper_path = Some("/private/user/helper".into());
        status.setup.certificate_path = "/private/user/key".into();
        status.capture.error = Some("token=secret".into());
        let report = render(&status);
        for secret in ["secret.example", "/private/user", "token=secret"] {
            assert!(!report.contains(secret));
        }
    }
    #[test]
    fn report_groups_ca_expiry_without_exposing_exact_date() {
        let mut status = AppStatus {
            settings: Default::default(), profiles: vec![], capture: Default::default(),
            setup: Default::default(), connection_issue: None, now: 1000.0,
        };
        status.setup.certificate_expires_at = Some(1000 + 15 * 86400);
        let report = render(&status);
        assert!(report.contains("CA lifetime: 30 days or less"));
        assert!(!report.contains(&(1000 + 15 * 86400).to_string()));
    }
    #[test]
    fn report_classifies_network_failure_without_host() {
        let mut status = AppStatus {
            settings: Default::default(), profiles: vec![], capture: Default::default(),
            setup: Default::default(), connection_issue: Some("secret.example is unreachable".into()), now: 1000.0,
        };
        status.setup.helper_path = Some("/private/helper".into());
        status.setup.certificate_exists = true;
        status.capture.phase = "listening".into();
        status.capture.observation = Some("network_ipv6".into());
        let report = render(&status);
        assert!(report.contains("Status: network_ipv6:"));
        assert!(!report.contains("secret.example"));
    }
}
