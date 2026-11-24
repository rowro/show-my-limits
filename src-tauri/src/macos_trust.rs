//! Read-only trust checks and explicit, user-initiated CA installation.
use core_foundation::{
    base::{CFType, TCFType},
    dictionary::CFDictionary,
    number::CFNumber,
    string::CFString,
};
use security_framework::{
    certificate::SecCertificate,
    item::{ItemClass, ItemSearchOptions, Limit, Reference, SearchResult},
    os::macos::keychain::SecKeychain,
    policy::SecPolicy,
    secure_transport::SslProtocolSide,
    trust::SecTrust,
    trust_settings::{Domain, TrustSettings, TrustSettingsForCertificate},
};
use security_framework_sys::trust_settings::{
    SecTrustSettingsSetTrustSettings, kSecTrustSettingsDomainUser,
    kSecTrustSettingsResultTrustRoot,
};
use std::{fs, path::Path};

fn contains_certificate(
    keychain: &SecKeychain,
    certificate: &SecCertificate,
) -> Result<bool, String> {
    let result = ItemSearchOptions::new()
        .class(ItemClass::certificate())
        .keychains(&[keychain.clone()])
        .load_refs(true)
        .limit(Limit::All)
        .search();
    let items = match result {
        Ok(items) => items,
        Err(error) if error.code() == -25300 => return Ok(false),
        Err(error) => {
            return Err(system_error(
                error.code(),
                "Не удалось проверить связку ключей пользователя",
            ));
        }
    };
    let expected = certificate.to_der();
    Ok(items.into_iter().any(|item| matches!(item, SearchResult::Ref(Reference::Certificate(found)) if found.to_der() == expected)))
}

fn system_error(code: i32, operation: &str) -> String {
    match code {
        -128 | -60006 => "Установка сертификата отменена. Можно повторить её позже.".into(),
        -25308 | -60007 => "macOS не разрешила показать подтверждение. Используйте «Установить вручную» или команду в Терминале.".into(),
        _ => format!("{operation}: {} (код {code}).", security_framework::base::Error::from_code(code)),
    }
}

fn ssl_settings() -> CFDictionary<CFString, CFType> {
    // Keep the same HTTPS-only scope as the previous security -p ssl command.
    let policy = SecPolicy::create_ssl(SslProtocolSide::SERVER, None);
    let result = CFNumber::from(kSecTrustSettingsResultTrustRoot as i32);
    CFDictionary::from_CFType_pairs(&[
        (CFString::new("kSecTrustSettingsPolicy"), policy.as_CFType()),
        (CFString::new("kSecTrustSettingsResult"), result.as_CFType()),
    ])
}

pub fn is_trusted(path: &Path) -> Result<bool, String> {
    let bytes = fs::read(path).map_err(|e| format!("Не удалось прочитать сертификат: {e}"))?;
    let (_, pem) =
        x509_parser::pem::parse_x509_pem(&bytes).map_err(|_| "Некорректный сертификат.")?;
    let certificate = SecCertificate::from_der(&pem.contents).map_err(|e| e.to_string())?;
    let policy = SecPolicy::create_ssl(SslProtocolSide::SERVER, None);
    let mut trust =
        SecTrust::create_with_certificates(&[certificate], &[policy]).map_err(|e| e.to_string())?;
    // Evaluate against actual OS trust; never install this CA as an evaluation anchor.
    trust
        .set_network_fetch_allowed(false)
        .map_err(|e| e.to_string())?;
    Ok(trust.evaluate_with_error().is_ok())
}

pub fn trust_scope(path: &Path) -> Result<&'static str, String> {
    let bytes = fs::read(path).map_err(|e| format!("Не удалось прочитать сертификат: {e}"))?;
    let (_, pem) = x509_parser::pem::parse_x509_pem(&bytes)
        .map_err(|_| "Некорректный сертификат.".to_string())?;
    let certificate = SecCertificate::from_der(&pem.contents).map_err(|e| e.to_string())?;
    let has_trust = |domain| match TrustSettings::new(domain).tls_trust_settings_for_certificate(&certificate) {
        Ok(Some(TrustSettingsForCertificate::TrustRoot)) => Ok(true),
        Ok(_) => Ok(false),
        Err(error) if error.code() == -25300 => Ok(false),
        Err(error) => Err(system_error(error.code(), "Не удалось проверить область доверия")),
    };
    let admin = has_trust(Domain::Admin)?;
    // A locked or unavailable login keychain must not hide an existing
    // machine-wide trust setting. In that case "system" means at least Admin.
    let user = match has_trust(Domain::User) {
        Ok(user) => user,
        Err(_) if admin => false,
        Err(error) => return Err(error),
    };
    Ok(match (user, admin) {
        (true, true) => "user+system",
        (true, false) => "user",
        (false, true) => "system",
        (false, false) => "none",
    })
}

pub fn present_in_checked_keychains(path: &Path) -> Result<bool, String> {
    let bytes = fs::read(path).map_err(|e| format!("Не удалось прочитать сертификат: {e}"))?;
    let (_, pem) = x509_parser::pem::parse_x509_pem(&bytes)
        .map_err(|_| "Некорректный сертификат.".to_string())?;
    let certificate = SecCertificate::from_der(&pem.contents).map_err(|e| e.to_string())?;
    let user = SecKeychain::default().map_err(|e| e.to_string())?;
    let system = SecKeychain::open("/Library/Keychains/System.keychain")
        .map_err(|e| e.to_string())?;
    Ok(contains_certificate(&user, &certificate)? || contains_certificate(&system, &certificate)?)
}

pub fn install(path: &Path) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|e| format!("Не удалось прочитать сертификат: {e}"))?;
    let (_, pem) =
        x509_parser::pem::parse_x509_pem(&bytes).map_err(|_| "Некорректный файл сертификата.")?;
    let certificate = SecCertificate::from_der(&pem.contents)
        .map_err(|e| system_error(e.code(), "Не удалось открыть сертификат"))?;
    let keychain = SecKeychain::default()
        .map_err(|e| system_error(e.code(), "Не удалось открыть связку ключей пользователя"))?;
    if !contains_certificate(&keychain, &certificate)? {
        certificate.add_to_keychain(Some(keychain.clone()))
            .map_err(|e| system_error(e.code(), "Не удалось импортировать сертификат"))?;
        if !contains_certificate(&keychain, &certificate)? {
            return Err("Импорт завершился, но сертификат не найден в связке ключей пользователя. Доверие не изменено.".into());
        }
    }
    let settings = ssl_settings();
    // SAFETY: CF wrappers retain the certificate, policy and dictionary for this call.
    // Running inside the real app GUI session lets Security.framework request authorization.
    let status = unsafe {
        SecTrustSettingsSetTrustSettings(
            certificate.as_concrete_TypeRef(),
            kSecTrustSettingsDomainUser,
            settings.as_CFTypeRef(),
        )
    };
    if status != 0 {
        return Err(system_error(status, "Не удалось настроить доверие"));
    }
    // Presence alone is insufficient: verify the persisted user-domain TLS trust setting.
    match TrustSettings::new(Domain::User).tls_trust_settings_for_certificate(&certificate) {
        Ok(Some(TrustSettingsForCertificate::TrustRoot)) => Ok(()),
        Ok(_) => Err("Сертификат импортирован, но доверие для HTTPS не подтверждено. Настройка не завершена.".into()),
        Err(error) => Err(system_error(error.code(), "Не удалось проверить сохранённое доверие")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_and_noninteractive_failures_are_not_success() {
        for code in [-128, -60006] {
            assert!(system_error(code, "test").contains("отменена"));
        }
        assert!(system_error(-25308, "test").contains("подтверждение"));
    }
    #[test]
    fn uninstalled_ca_is_not_trusted_by_the_operating_system() {
        let fixture = tempfile::NamedTempFile::new().unwrap();
        fs::write(fixture.path(), include_bytes!("../tests/fixtures/test-ca.pem")).unwrap();
        assert!(!is_trusted(fixture.path()).unwrap());
    }
    #[test]
    #[ignore = "read-only probe of the local application CA; never installs or changes trust"]
    fn current_ca_trust_probe() {
        let path = std::path::PathBuf::from(std::env::var("SML_TRUST_PROBE_CERT").unwrap());
        println!("actual OS trust: {}", is_trusted(&path).unwrap());
    }
    #[test]
    fn native_certificate_and_policy_can_be_built_without_changing_keychain() {
        let (_, pem) =
            x509_parser::pem::parse_x509_pem(include_bytes!("../tests/fixtures/test-ca.pem"))
                .unwrap();
        let certificate = SecCertificate::from_der(&pem.contents).unwrap();
        assert_eq!(certificate.subject_summary(), "Show My Limits Test CA");
        assert_eq!(certificate.to_der(), pem.contents);
        let settings = ssl_settings();
        assert_eq!(settings.len(), 2);
        assert!(settings.contains_key(&CFString::new("kSecTrustSettingsPolicy")));
    }

    #[test]
    fn user_keychain_search_reads_certificates_without_importing() {
        let (_, pem) =
            x509_parser::pem::parse_x509_pem(include_bytes!("../tests/fixtures/test-ca.pem"))
                .unwrap();
        let certificate = SecCertificate::from_der(&pem.contents).unwrap();
        let keychain = SecKeychain::default().unwrap();
        assert!(!contains_certificate(&keychain, &certificate).unwrap());
        let fixture = tempfile::NamedTempFile::new().unwrap();
        fs::write(fixture.path(), include_bytes!("../tests/fixtures/test-ca.pem")).unwrap();
        assert!(!present_in_checked_keychains(fixture.path()).unwrap());
    }
}
