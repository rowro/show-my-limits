use sha2::{Digest, Sha256};
use std::{fs, path::Path};
use x509_parser::{pem::parse_x509_pem, prelude::*};

pub struct CertificateDetails {
    pub name: String,
    pub sha256: String,
}

fn parse(bytes: &[u8]) -> Option<CertificateDetails> {
    let (_, pem) = parse_x509_pem(bytes).ok()?;
    if pem.label != "CERTIFICATE" { return None; }
    let (_, cert) = X509Certificate::from_der(&pem.contents).ok()?;
    let name = cert
        .subject()
        .iter_common_name()
        .next()?
        .as_str()
        .ok()?
        .to_owned();
    let sha256 = Sha256::digest(&pem.contents)
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(":");
    Some(CertificateDetails { name, sha256 })
}

pub fn details(path: &Path) -> Option<CertificateDetails> {
    if fs::metadata(path).ok()?.len() > 64 * 1024 {
        return None;
    }
    parse(&fs::read(path).ok()?)
}

#[cfg(any(target_os = "linux", test))]
pub fn certificate_in_pem_bundle(cert_path: &Path, bundle_path: &Path) -> Option<bool> {
    let cert_bytes = fs::read(cert_path).ok()?;
    let (_, expected) = parse_x509_pem(&cert_bytes).ok()?;
    let bundle = fs::read(bundle_path).ok()?;
    if bundle.len() > 4 * 1024 * 1024 { return None; }
    let mut remaining = bundle.as_slice();
    let mut found_any = false;
    while !remaining.iter().all(u8::is_ascii_whitespace) {
        let (rest, pem) = parse_x509_pem(remaining).ok()?;
        if rest.len() == remaining.len() { return None; }
        found_any = true;
        if pem.contents == expected.contents { return Some(true); }
        remaining = rest;
    }
    found_any.then_some(false)
}

#[cfg(target_os = "linux")]
pub fn debian_system_trust(cert_path: &Path) -> Option<bool> {
    certificate_in_pem_bundle(cert_path, Path::new("/etc/ssl/certs/ca-certificates.crt"))
}

/// Refuse to let a helper regenerate a partially deleted existing CA.
pub fn complete_store(directory: &Path) -> bool {
    if !fs::symlink_metadata(directory).is_ok_and(|m| m.file_type().is_dir()) {
        return false;
    }
    let public_path = directory.join("mitmproxy-ca-cert.cer");
    let combined_path = directory.join("mitmproxy-ca.pem");
    let dh_path = directory.join("mitmproxy-dhparam.pem");
    if ![&public_path, &combined_path, &dh_path].iter().all(|path|
        fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file() && m.len() > 0 && m.len() <= 64 * 1024)) {
        return false;
    }
    let (Ok(public), Ok(combined)) = (fs::read(public_path), fs::read(combined_path)) else {
        return false;
    };
    let Ok((_, public_cert)) = parse_x509_pem(&public) else { return false; };
    if public_cert.label != "CERTIFICATE" || X509Certificate::from_der(&public_cert.contents).is_err() {
        return false;
    }
    let Ok((remaining, key)) = parse_x509_pem(&combined) else { return false; };
    if !matches!(key.label.as_str(), "PRIVATE KEY" | "RSA PRIVATE KEY" | "EC PRIVATE KEY") {
        return false;
    }
    let Ok((_, embedded_cert)) = parse_x509_pem(remaining) else { return false; };
    embedded_cert.label == "CERTIFICATE" && embedded_cert.contents == public_cert.contents
}

/// Limit access to newly generated CA material before publishing the store.
/// The staging directory is private, so no other user can access files before this runs.
pub fn secure_new_store(directory: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("Не удалось защитить каталог сертификата: {e}"))?;
        for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
                return Err("Неожиданный объект в каталоге сертификата.".into());
            }
            fs::set_permissions(entry.path(), fs::Permissions::from_mode(0o600))
                .map_err(|e| format!("Не удалось защитить файл сертификата: {e}"))?;
        }
    }
    #[cfg(not(unix))]
    let _ = directory;
    Ok(())
}

/// Report insecure permissions without changing an existing CA or its key.
pub fn security_issue(directory: &Path) -> Option<&'static str> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if !directory.exists() {
            return None;
        }
        let Ok(metadata) = fs::symlink_metadata(directory) else {
            return Some("Не удалось проверить права каталога CA.");
        };
        if !metadata.file_type().is_dir() || metadata.permissions().mode() & 0o077 != 0 {
            return Some("Каталог сертификата доступен другим пользователям. Проверьте права доступа к CA и его закрытому ключу.");
        }
        let Ok(entries) = fs::read_dir(directory) else {
            return Some("Не удалось проверить права файлов CA.");
        };
        for entry in entries {
            let Ok(entry) = entry else {
                return Some("Не удалось проверить права файлов CA.");
            };
            let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
                return Some("Не удалось проверить права файлов CA.");
            };
            if !metadata.file_type().is_file() || metadata.permissions().mode() & 0o077 != 0 {
                return Some("Файлы CA доступны другим пользователям или содержат ссылку. Проверьте права доступа к закрытому ключу.");
            }
        }
    }
    #[cfg(not(unix))]
    let _ = directory;
    None
}

pub fn install(path: &Path) -> Result<(), String> {
    if details(path).is_none() {
        return Err("Сначала создайте корректный сертификат приложения.".into());
    }
    #[cfg(target_os = "macos")]
    {
        crate::macos_trust::install(path)
    }
    #[cfg(target_os = "windows")]
    {
        crate::windows_trust::install(path)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Err(
            "Установка по кнопке пока доступна на macOS. Используйте ручной импорт сертификата."
                .into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_certificate_has_no_identity() {
        assert!(parse(b"not a certificate").is_none());
    }
    #[test]
    fn public_certificate_alone_is_not_a_complete_store() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("mitmproxy-ca-cert.cer"), include_bytes!("../tests/fixtures/test-ca.pem")).unwrap();
        assert!(!complete_store(directory.path()));
    }
    #[test]
    fn complete_store_requires_matching_public_and_embedded_ca() {
        let directory = tempfile::tempdir().unwrap();
        let public = include_bytes!("../tests/fixtures/test-ca.pem");
        fs::write(directory.path().join("mitmproxy-ca-cert.cer"), public).unwrap();
        fs::write(directory.path().join("mitmproxy-dhparam.pem"), b"dh parameters").unwrap();
        let mut combined = b"-----BEGIN PRIVATE KEY-----\nAQ==\n-----END PRIVATE KEY-----\n".to_vec();
        combined.extend_from_slice(public);
        fs::write(directory.path().join("mitmproxy-ca.pem"), &combined).unwrap();
        assert!(complete_store(directory.path()));
        combined.truncate(b"-----BEGIN PRIVATE KEY-----\nAQ==\n-----END PRIVATE KEY-----\n".len());
        combined.extend_from_slice(b"-----BEGIN CERTIFICATE-----\nAQ==\n-----END CERTIFICATE-----\n");
        fs::write(directory.path().join("mitmproxy-ca.pem"), combined).unwrap();
        assert!(!complete_store(directory.path()));
    }
    #[cfg(unix)]
    #[test]
    fn complete_store_rejects_symlinked_key() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let public = include_bytes!("../tests/fixtures/test-ca.pem");
        fs::write(directory.path().join("mitmproxy-ca-cert.cer"), public).unwrap();
        fs::write(directory.path().join("mitmproxy-dhparam.pem"), b"dh parameters").unwrap();
        let mut combined = b"-----BEGIN PRIVATE KEY-----\nAQ==\n-----END PRIVATE KEY-----\n".to_vec();
        combined.extend_from_slice(public);
        fs::write(directory.path().join("mitmproxy-ca.pem"), combined).unwrap();
        assert!(complete_store(directory.path()));
        let linked_directory = directory.path().join("linked-certificates");
        symlink(directory.path(), &linked_directory).unwrap();
        assert!(!complete_store(&linked_directory));
        let key = directory.path().join("key.pem");
        fs::write(&key, b"-----BEGIN PRIVATE KEY-----\nAQ==\n-----END PRIVATE KEY-----\n").unwrap();
        fs::remove_file(directory.path().join("mitmproxy-ca.pem")).unwrap();
        symlink(key, directory.path().join("mitmproxy-ca.pem")).unwrap();
        assert!(!complete_store(directory.path()));
    }
    #[test]
    fn certificate_identity_matches_openssl() {
        let cert = include_bytes!("../tests/fixtures/test-ca.pem");
        let identity = parse(cert).unwrap();
        assert_eq!(identity.name, "Show My Limits Test CA");
        assert_eq!(
            identity.sha256,
            include_str!("../tests/fixtures/test-ca.sha256").trim()
        );
    }
    #[test]
    fn bundle_check_matches_exact_ca_without_changing_trust() {
        let temp = tempfile::tempdir().unwrap();
        let cert = temp.path().join("cert.pem");
        let bundle = temp.path().join("bundle.pem");
        let fixture = include_bytes!("../tests/fixtures/test-ca.pem");
        fs::write(&cert, fixture).unwrap();
        fs::write(&bundle, fixture).unwrap();
        assert_eq!(certificate_in_pem_bundle(&cert, &bundle), Some(true));
        fs::write(&bundle, b"-----BEGIN CERTIFICATE-----\nAQ==\n-----END CERTIFICATE-----\n").unwrap();
        assert_eq!(certificate_in_pem_bundle(&cert, &bundle), Some(false));
    }
    #[cfg(unix)]
    #[test]
    fn new_ca_store_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let staging = tempfile::tempdir().unwrap();
        let store = staging.path().join("generated");
        fs::create_dir(&store).unwrap();
        let key = store.join("mitmproxy-ca.pem");
        fs::write(&key, b"test key").unwrap();
        secure_new_store(&store).unwrap();
        assert_eq!(fs::metadata(&store).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(fs::metadata(&key).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(security_issue(&store), None);
        fs::set_permissions(&key, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(security_issue(&store).is_some());
    }
}
