//! Cross-frontend coordination. OS locks disappear on owner exit, not on stale PID checks.
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
};

pub fn lock(root: &Path, name: &str) -> Result<File, String> {
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join(format!("{name}.lock")))
        .map_err(|e| e.to_string())?;
    file.try_lock().map_err(|_| format!("Операция {name} уже выполняется в Show My Limits или CLI. Для просмотра запустите sml watch."))?;
    Ok(file)
}

pub fn publish_run(root: &Path, run: &str) -> Result<(), String> {
    // The capture lock serializes writers. Persist a pointer, never settings or raw traffic.
    let mut temp = tempfile::NamedTempFile::new_in(root).map_err(|e| e.to_string())?;
    use std::io::Write;
    temp.write_all(run.as_bytes()).map_err(|e| e.to_string())?;
    temp.persist(root.join("active-run"))
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn active_state(root: &Path, now: f64) -> Option<crate::quota::ProxyState> {
    // An unlocked capture is stopped, regardless of its old heartbeat or PID.
    if let Ok(_available) = lock(root, "capture") {
        return None;
    }
    let run = fs::read_to_string(root.join("active-run")).ok()?;
    if run.is_empty() || run.len() > 128 || !run.bytes().all(|c| c.is_ascii_digit() || c == b'-') {
        return None;
    }
    let path = root.join("runs").join(&run).join("state.json");
    if fs::metadata(&path).ok()?.len() > 262_144 {
        return None;
    }
    let state: crate::quota::ProxyState = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    (state.run_id.as_deref() == Some(run.as_str())
        && state.running
        && (0.0..20.0).contains(&(now - state.updated_at)))
    .then_some(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_one_owner_and_stale_state_is_not_live() {
        let dir = tempfile::tempdir().unwrap();
        let first = lock(dir.path(), "capture").unwrap();
        assert!(lock(dir.path(), "capture").is_err());
        publish_run(dir.path(), "123-456").unwrap();
        let run = dir.path().join("runs/123-456");
        fs::create_dir_all(&run).unwrap();
        fs::write(run.join("state.json"), r#"{"runId":"123-456","pid":123,"running":true,"updatedAt":100,"startedAt":90,"profiles":[],"health":null}"#).unwrap();
        assert!(active_state(dir.path(), 101.0).is_some());
        assert!(active_state(dir.path(), 121.0).is_none());
        drop(first);
        // Concurrent tests can fork while this FD is open. A fork briefly inherits
        // flock until exec closes CLOEXEC descriptors; allow that bounded handoff.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if let Ok(available) = lock(dir.path(), "capture") { drop(available); break; }
            assert!(std::time::Instant::now() < deadline, "capture lock leaked");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(active_state(dir.path(), 101.0).is_none());
    }
}
