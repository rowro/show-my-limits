//! Inspect executable names only; never read or persist command arguments or credentials.
#[cfg(any(target_os = "macos", test))]
fn proxy_pids(processes: &str, own_pid: Option<u32>) -> Vec<u32> {
    let mut pids = std::collections::BTreeSet::new();
    for line in processes.lines() {
        let line = line.trim();
        let Some((pid, executable)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };
        let name = std::path::Path::new(executable.trim())
            .file_name()
            .and_then(|n| n.to_str());
        if Some(pid) != own_pid && matches!(name, Some("mitmdump" | "mitmproxy" | "mitmweb")) {
            pids.insert(pid);
        }
    }
    pids.into_iter().collect()
}

pub fn ensure_exclusive() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("/bin/ps")
            .args(["-axo", "pid=,comm="])
            .output()
            .map_err(|_| "Не удалось проверить другие перехватчики. Повторите запуск.")?;
        if !output.status.success() {
            return Err("Не удалось проверить другие перехватчики. Повторите запуск.".into());
        }
        let pids = proxy_pids(&String::from_utf8_lossy(&output.stdout), None);
        if !pids.is_empty() {
            let ids = pids
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "Уже запущен другой mitmproxy (PID {ids}). Остановите перехват в другом приложении, затем запустите Show My Limits. Одновременный захват отключён."
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recognizes_helpers_without_matching_apps_or_the_network_extension() {
        let sample = " 964 /Applications/Example.app/Contents/MacOS/Example\n 12017 /opt/homebrew/bin/mitmdump\n 12022 /Library/SystemExtensions/ABC/org.mitmproxy.macos-redirector.network-extension\n 42773 /Applications/Show My Limits.app/Contents/Resources/binaries/mitmdump\n 42 /bin/sh\n";
        assert_eq!(proxy_pids(sample, Some(42773)), vec![12017]);
        assert_eq!(proxy_pids(sample, None), vec![12017, 42773]);
    }
    #[test]
    fn ignores_partial_names_and_invalid_lines() {
        assert!(
            proxy_pids(
                "not-a-pid /bin/mitmdump\n 10 /tmp/mitmdump-backup\n 20 /tmp/my-mitmproxy",
                None
            )
            .is_empty()
        );
    }
}
