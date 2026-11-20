use sha2::{Digest, Sha256};
use std::{fs, path::Path};
use x509_parser::{pem::parse_x509_pem, prelude::*};

pub struct CertificateDetails {
    pub name: String,
    pub sha256: String,
}

fn parse(bytes: &[u8]) -> Option<CertificateDetails> {
    let (_, pem) = parse_x509_pem(bytes).ok()?;
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

/// Refuse to let a helper regenerate a partially deleted existing CA.
pub fn complete_store(directory: &Path) -> bool {
    details(&directory.join("mitmproxy-ca-cert.cer")).is_some()
        && ["mitmproxy-ca.pem", "mitmproxy-dhparam.pem"].iter().all(|name|
            fs::metadata(directory.join(name)).is_ok_and(|m| m.is_file() && m.len() > 0))
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
    fn certificate_identity_matches_openssl() {
        let cert = include_bytes!("../tests/fixtures/test-ca.pem");
        let identity = parse(cert).unwrap();
        assert_eq!(identity.name, "Show My Limits Test CA");
        assert_eq!(
            identity.sha256,
            include_str!("../tests/fixtures/test-ca.sha256").trim()
        );
    }
}
