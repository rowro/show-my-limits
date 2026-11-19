//! Installs a pinned official portable distribution into this app's data directory.
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

pub const VERSION: &str = "12.3.1";
const MAX_DOWNLOAD: u64 = 160 * 1024 * 1024;

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installation {
    pub phase: String,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub error: Option<String>,
}
impl Installation {
    pub fn active(&self) -> bool {
        matches!(
            self.phase.as_str(),
            "downloading" | "verifying" | "extracting"
        )
    }
}

pub struct Package {
    pub file: &'static str,
    pub sha256: &'static str,
    pub executable: &'static str,
}
// Digests from the release's published .sigstore statement, pinned at build time.
// https://downloads.mitmproxy.org/12.3.1/mitmproxy-12.3.1.sigstore
pub fn package(os: &str, arch: &str) -> Result<Package, String> {
    let (file, sha256) = match (os, arch) {
        ("macos", "aarch64") => ("mitmproxy-12.3.1-macos-arm64.tar.gz", "9b7f7a8e3fd1b1544089c937c579ef1f0f503f0ea598c25bda9eab077c735b93"),
        ("macos", "x86_64") => ("mitmproxy-12.3.1-macos-x86_64.tar.gz", "f4cf5e563e7068af297a27be97686f58917760d3c0dcc6a997ca83f6f5ffb3db"),
        ("linux", "aarch64") => ("mitmproxy-12.3.1-linux-aarch64.tar.gz", "97a21d57b2288f7e293e63d3b11af34f394a8bbf760e6cf101ed1d57057a1f40"),
        ("linux", "x86_64") => ("mitmproxy-12.3.1-linux-x86_64.tar.gz", "f7b4237f5702f6cd21357fe6e4680a7e905f8f9b816b872230d018ea696c34d5"),
        ("windows", "x86_64") => ("mitmproxy-12.3.1-windows-x86_64.zip", "336c03e7215876af329f57c44b1b738cfb22cd29dc7cd4c0714f15c7648af829"),
        _ => return Err("Для этой ОС и архитектуры нет встроенной установки. Установите mitmproxy самостоятельно и укажите путь к mitmdump.".into()),
    };
    Ok(Package {
        file,
        sha256,
        executable: match os {
            "macos" => "mitmproxy.app/Contents/MacOS/mitmdump",
            "windows" => "mitmdump.exe",
            _ => "mitmdump",
        },
    })
}

pub fn installed_path(base: &Path) -> Option<PathBuf> {
    let pkg = package(std::env::consts::OS, std::env::consts::ARCH).ok()?;
    let path = base.join(VERSION).join(pkg.executable);
    path.is_file().then_some(path)
}

fn check_digest(actual: &[u8], expected: &str) -> Result<(), String> {
    let hex = actual
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if hex != expected {
        return Err(
            "Контрольная сумма загрузки не совпала. Файл не установлен; повторите попытку.".into(),
        );
    }
    Ok(())
}

fn extract(archive: &Path, output: &Path, zip: bool) -> Result<(), String> {
    fs::create_dir_all(output).map_err(|e| e.to_string())?;
    let file = fs::File::open(archive).map_err(|e| e.to_string())?;
    if zip {
        let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
            let relative = entry.enclosed_name().ok_or("Небезопасный путь в архиве.")?;
            if entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
            {
                return Err("Неожиданная ссылка в ZIP-архиве.".into());
            }
            let target = output.join(relative);
            if entry.is_dir() {
                fs::create_dir_all(target).map_err(|e| e.to_string())?;
            } else {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                let mut destination = fs::File::create(target).map_err(|e| e.to_string())?;
                std::io::copy(&mut entry, &mut destination).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    } else {
        // tar validates traversal paths and preserves the signed macOS app's internal symlinks.
        tar::Archive::new(flate2::read::GzDecoder::new(file))
            .unpack(output)
            .map_err(|e| e.to_string())
    }
}

pub fn install_to(base: &Path, mut progress: impl FnMut(Installation)) -> Result<PathBuf, String> {
    let pkg = package(std::env::consts::OS, std::env::consts::ARCH)?;
    if let Some(path) = installed_path(base) {
        return Ok(path);
    }
    fs::create_dir_all(base).map_err(|e| e.to_string())?;
    let temporary = tempfile::Builder::new()
        .prefix(".install-")
        .tempdir_in(base)
        .map_err(|e| e.to_string())?;
    let archive = temporary.path().join("package");
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(600))
        .user_agent("Show-My-Limits/0.1")
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client
        .get(format!(
            "https://downloads.mitmproxy.org/{VERSION}/{}",
            pkg.file
        ))
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Не удалось загрузить mitmproxy: {e}"))?;
    let total = response.content_length();
    if total.is_some_and(|size| size > MAX_DOWNLOAD) {
        return Err("Размер загрузки превышает ожидаемый.".into());
    }
    let mut downloaded = 0;
    let mut last_report = 0;
    let mut hash = Sha256::new();
    let mut destination = fs::File::create(&archive).map_err(|e| e.to_string())?;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = response
            .read(&mut buffer)
            .map_err(|e| format!("Загрузка прервана: {e}"))?;
        if count == 0 {
            break;
        }
        downloaded += count as u64;
        if downloaded > MAX_DOWNLOAD {
            return Err("Размер загрузки превышает ожидаемый.".into());
        }
        destination
            .write_all(&buffer[..count])
            .map_err(|e| e.to_string())?;
        hash.update(&buffer[..count]);
        if downloaded - last_report >= 256 * 1024 {
            progress(Installation {
                phase: "downloading".into(),
                downloaded,
                total,
                error: None,
            });
            last_report = downloaded;
        }
    }
    drop(destination);
    progress(Installation {
        phase: "verifying".into(),
        downloaded,
        total,
        error: None,
    });
    check_digest(&hash.finalize(), pkg.sha256)?;
    progress(Installation {
        phase: "extracting".into(),
        downloaded,
        total,
        error: None,
    });
    let unpacked = temporary.path().join("unpacked");
    extract(&archive, &unpacked, pkg.file.ends_with(".zip"))?;
    if !unpacked.join(pkg.executable).is_file() {
        return Err("В официальном архиве не найден mitmdump.".into());
    }
    let final_directory = base.join(VERSION);
    // Never overwrite a pre-existing installation or a running helper.
    if final_directory.exists() {
        return Err("Каталог установки уже существует, но mitmdump не найден. Укажите другой путь в настройках.".into());
    }
    fs::rename(unpacked, &final_directory).map_err(|e| e.to_string())?;
    Ok(final_directory.join(pkg.executable))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manifests_and_integrity() {
        for (os, arch) in [
            ("macos", "aarch64"),
            ("macos", "x86_64"),
            ("linux", "aarch64"),
            ("linux", "x86_64"),
            ("windows", "x86_64"),
        ] {
            let pkg = package(os, arch).unwrap();
            assert_eq!(pkg.sha256.len(), 64);
        }
        assert!(package("windows", "aarch64").is_err());
        assert!(
            check_digest(
                &Sha256::digest(b"corrupt"),
                package("macos", "aarch64").unwrap().sha256
            )
            .is_err()
        );
    }
    #[test]
    #[ignore = "downloads an official portable distribution into a temporary directory; never runs it"]
    fn official_package_download_and_extract() {
        let temporary = tempfile::tempdir().unwrap();
        let path = install_to(temporary.path(), |_| {}).unwrap();
        assert!(path.is_file());
        assert_eq!(installed_path(temporary.path()).unwrap(), path);
    }
}
