use sha2::{Digest, Sha256};
use ring::signature::RsaKeyPair;
use std::{fs, path::Path};
use x509_parser::{pem::parse_x509_pem, prelude::*};

pub struct CertificateDetails {
    pub name: String,
    pub sha256: String,
    pub expires_at: i64,
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
    Some(CertificateDetails { name, sha256, expires_at: cert.validity().not_after.timestamp() })
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

#[cfg(any(target_os = "linux", test))]
fn source_tree_contains(cert_path: &Path, roots: &[&Path]) -> Option<bool> {
    let mut pending: Vec<_> = roots.iter().map(|root| root.to_path_buf()).collect();
    while let Some(path) = pending.pop() {
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return None,
        };
        for entry in entries {
            let entry = entry.ok()?;
            let kind = entry.file_type().ok()?;
            if kind.is_symlink() { return None; }
            if kind.is_dir() { pending.push(entry.path()); }
            else if kind.is_file() && entry.path().extension().is_some_and(|ext| ext == "crt") {
                match certificate_in_pem_bundle(cert_path, &entry.path()) {
                    Some(true) => return Some(true),
                    Some(false) => {},
                    None => return None,
                }
            }
        }
    }
    Some(false)
}

#[cfg(target_os = "linux")]
pub fn debian_source_trust(cert_path: &Path) -> Option<bool> {
    source_tree_contains(cert_path, &[
        Path::new("/usr/local/share/ca-certificates"),
        Path::new("/usr/share/ca-certificates"),
    ])
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
    let Ok((_, embedded_cert)) = parse_x509_pem(remaining) else { return false; };
    if embedded_cert.label != "CERTIFICATE" || embedded_cert.contents != public_cert.contents {
        return false;
    }
    let Ok((_, parsed_cert)) = X509Certificate::from_der(&public_cert.contents) else { return false; };
    let key_pair = match key.label.as_str() {
        "RSA PRIVATE KEY" => RsaKeyPair::from_der(&key.contents),
        "PRIVATE KEY" => RsaKeyPair::from_pkcs8(&key.contents),
        _ => return false,
    };
    key_pair.is_ok_and(|pair| pair.public().as_ref() == parsed_cert.public_key().subject_public_key.data.as_ref())
}

pub fn rotation_paths(active: &Path) -> Result<(std::path::PathBuf, std::path::PathBuf), String> {
    let parent = active.parent().ok_or("Не найден каталог данных CA.")?;
    Ok((parent.join("certificates-next"), parent.join("certificates-previous")))
}

/// The caller must first stop the trial capture and prove a fresh same-run quota response.
/// Keep the previous key until separate, explicit trust cleanup succeeds.
pub fn promote_replacement(active: &Path) -> Result<(), String> {
    let (pending, retired) = rotation_paths(active)?;
    if !complete_store(active) || !complete_store(&pending) {
        return Err("Действующий или новый CA повреждён; замена не выполнялась.".into());
    }
    let old = details(&active.join("mitmproxy-ca-cert.cer"))
        .ok_or("Не удалось прочитать действующий CA.")?;
    let new = details(&pending.join("mitmproxy-ca-cert.cer"))
        .ok_or("Не удалось прочитать новый CA.")?;
    if old.sha256 == new.sha256 {
        return Err("Новый CA совпадает с действующим; нужен новый ключ.".into());
    }
    if fs::symlink_metadata(&retired).is_ok() {
        return Err("Предыдущий CA ещё ожидает удаления доверия; новая замена недоступна.".into());
    }
    fs::rename(active, &retired)
        .map_err(|e| format!("Не удалось сохранить прежний CA: {e}"))?;
    if let Err(error) = fs::rename(&pending, active) {
        if let Err(rollback) = fs::rename(&retired, active) {
            return Err(format!("Не удалось переключить CA ({error}) и вернуть прежний ({rollback}). Оба каталога сохранены; требуется ручная проверка."));
        }
        return Err(format!("Не удалось переключить CA: {error}. Прежний CA восстановлен."));
    }
    Ok(())
}

pub fn restore_interrupted_replacement(active: &Path) -> Result<(), String> {
    let (_, retired) = rotation_paths(active)?;
    if fs::symlink_metadata(active).is_ok() {
        return Err("Действующий каталог CA существует; автоматическое восстановление недоступно.".into());
    }
    if !complete_store(&retired) {
        return Err("Прежний CA повреждён; не удалось безопасно восстановить его.".into());
    }
    fs::rename(&retired, active)
        .map_err(|e| format!("Не удалось восстановить прежний CA: {e}"))
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
    let has_material = ["mitmproxy-ca-cert.cer", "mitmproxy-ca.pem", "mitmproxy-dhparam.pem"]
        .iter().any(|name| fs::symlink_metadata(directory.join(name)).is_ok());
    if has_material && !complete_store(directory) {
        return Some("Хранилище CA неполное, содержит ссылку или закрытый ключ не соответствует сертификату. Существующие файлы не изменены.");
    }
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
    fn complete_store_rejects_unrelated_private_key_and_certificate() {
        let directory = tempfile::tempdir().unwrap();
        let public = include_bytes!("../tests/fixtures/test-ca.pem");
        fs::write(directory.path().join("mitmproxy-ca-cert.cer"), public).unwrap();
        fs::write(directory.path().join("mitmproxy-dhparam.pem"), b"dh parameters").unwrap();
        let mut combined = b"-----BEGIN PRIVATE KEY-----\nAQ==\n-----END PRIVATE KEY-----\n".to_vec();
        combined.extend_from_slice(public);
        fs::write(directory.path().join("mitmproxy-ca.pem"), &combined).unwrap();
        assert!(!complete_store(directory.path()));
        assert!(security_issue(directory.path()).unwrap().contains("ключ не соответствует"));
        combined.truncate(b"-----BEGIN PRIVATE KEY-----\nAQ==\n-----END PRIVATE KEY-----\n".len());
        combined.extend_from_slice(b"-----BEGIN CERTIFICATE-----\nAQ==\n-----END CERTIFICATE-----\n");
        fs::write(directory.path().join("mitmproxy-ca.pem"), combined).unwrap();
        assert!(!complete_store(directory.path()));
    }
    #[cfg(unix)]
    #[test]
    fn complete_store_accepts_an_ephemeral_matching_rsa_pair() {
        use std::process::{Command, Stdio};
        if Command::new("openssl").arg("version").output().is_err() { return; }
        let directory = tempfile::tempdir().unwrap();
        let key = directory.path().join("key.pem");
        let cert = directory.path().join("mitmproxy-ca-cert.cer");
        let status = Command::new("openssl")
            .args(["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-subj", "/CN=Ephemeral Test", "-days", "1", "-keyout"])
            .arg(&key).arg("-out").arg(&cert)
            .stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap();
        assert!(status.success());
        let mut combined = fs::read(&key).unwrap();
        combined.extend_from_slice(&fs::read(&cert).unwrap());
        fs::write(directory.path().join("mitmproxy-ca.pem"), combined).unwrap();
        fs::write(directory.path().join("mitmproxy-dhparam.pem"), b"dh parameters").unwrap();
        assert!(complete_store(directory.path()));
        let other_key = directory.path().join("other-key.pem");
        let other_cert = directory.path().join("other-cert.pem");
        let status = Command::new("openssl")
            .args(["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-subj", "/CN=Other Test", "-days", "1", "-keyout"])
            .arg(&other_key).arg("-out").arg(&other_cert)
            .stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap();
        assert!(status.success());
        let mut mismatched = fs::read(other_key).unwrap();
        mismatched.extend_from_slice(&fs::read(cert).unwrap());
        fs::write(directory.path().join("mitmproxy-ca.pem"), mismatched).unwrap();
        assert!(!complete_store(directory.path()));
    }
    #[cfg(unix)]
    #[test]
    fn rotation_keeps_the_previous_ca_and_refuses_an_unfinished_cleanup() {
        use std::process::{Command, Stdio};
        if Command::new("openssl").arg("version").output().is_err() { return; }
        let root = tempfile::tempdir().unwrap();
        let active = root.path().join("certificates");
        let (pending, retired) = rotation_paths(&active).unwrap();
        for (path, name) in [(&active, "Old"), (&pending, "New")] {
            fs::create_dir(path).unwrap();
            let key = path.join("key.pem");
            let cert = path.join("mitmproxy-ca-cert.cer");
            let status = Command::new("openssl")
                .args(["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-subj"])
                .arg(format!("/CN={name}"))
                .args(["-days", "1", "-keyout"])
                .arg(&key).arg("-out").arg(&cert)
                .stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap();
            assert!(status.success());
            let mut combined = fs::read(&key).unwrap();
            combined.extend_from_slice(&fs::read(&cert).unwrap());
            fs::write(path.join("mitmproxy-ca.pem"), combined).unwrap();
            fs::write(path.join("mitmproxy-dhparam.pem"), b"dh parameters").unwrap();
        }
        let old = details(&active.join("mitmproxy-ca-cert.cer")).unwrap().sha256;
        let new = details(&pending.join("mitmproxy-ca-cert.cer")).unwrap().sha256;
        fs::write(pending.join("mitmproxy-ca.pem"), b"broken key").unwrap();
        assert!(promote_replacement(&active).is_err());
        assert_eq!(details(&active.join("mitmproxy-ca-cert.cer")).unwrap().sha256, old);
        assert!(!retired.exists());
        let mut combined = fs::read(pending.join("key.pem")).unwrap();
        combined.extend_from_slice(&fs::read(pending.join("mitmproxy-ca-cert.cer")).unwrap());
        fs::write(pending.join("mitmproxy-ca.pem"), combined).unwrap();
        promote_replacement(&active).unwrap();
        assert_eq!(details(&active.join("mitmproxy-ca-cert.cer")).unwrap().sha256, new);
        assert_eq!(details(&retired.join("mitmproxy-ca-cert.cer")).unwrap().sha256, old);
        assert!(promote_replacement(&active).is_err());
        assert!(restore_interrupted_replacement(&active).is_err());
        fs::rename(&active, &pending).unwrap();
        restore_interrupted_replacement(&active).unwrap();
        assert_eq!(details(&active.join("mitmproxy-ca-cert.cer")).unwrap().sha256, old);
        assert_eq!(details(&pending.join("mitmproxy-ca-cert.cer")).unwrap().sha256, new);
    }
    #[test]
    #[ignore = "read-only check of an explicitly provided CA store; never changes OS trust"]
    fn real_ca_store_probe() {
        let directory = std::env::var("SML_CA_PROBE_STORE").expect("set SML_CA_PROBE_STORE");
        assert!(complete_store(Path::new(&directory)));
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
        assert!(!complete_store(directory.path()));
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
    #[test]
    fn cleanup_checks_exact_ca_in_debian_source_files() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("sources");
        fs::create_dir(&source).unwrap();
        let cert = root.path().join("old.cer");
        fs::write(&cert, include_bytes!("../tests/fixtures/test-ca.pem")).unwrap();
        assert_eq!(source_tree_contains(&cert, &[&source]), Some(false));
        fs::write(source.join("different.crt"), b"-----BEGIN CERTIFICATE-----\nAQ==\n-----END CERTIFICATE-----\n").unwrap();
        assert_eq!(source_tree_contains(&cert, &[&source]), Some(false));
        fs::write(source.join("old.crt"), include_bytes!("../tests/fixtures/test-ca.pem")).unwrap();
        assert_eq!(source_tree_contains(&cert, &[&source]), Some(true));
    }
    #[cfg(unix)]
    #[test]
    fn new_ca_store_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let staging = tempfile::tempdir().unwrap();
        let store = staging.path().join("generated");
        fs::create_dir(&store).unwrap();
        let key = store.join("test-key");
        fs::write(&key, b"test key").unwrap();
        secure_new_store(&store).unwrap();
        assert_eq!(fs::metadata(&store).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(fs::metadata(&key).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(security_issue(&store), None);
        fs::set_permissions(&key, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(security_issue(&store).is_some());
    }
}
