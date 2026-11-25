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
use std::{fs, path::Path, process::{Command, Stdio}};

unsafe extern "C" {
    fn SecTrustSettingsRemoveTrustSettings(
        certificate: security_framework_sys::base::SecCertificateRef,
        domain: security_framework_sys::trust_settings::SecTrustSettingsDomain,
    ) -> i32;
}

fn contains_certificate(
    keychain: &SecKeychain,
    certificate: &SecCertificate,
) -> Result<bool, String> {
    Ok(find_certificate(keychain, certificate)?.is_some())
}

/// The exact certificate (by DER) stored in this keychain, if any.
fn find_certificate(
    keychain: &SecKeychain,
    certificate: &SecCertificate,
) -> Result<Option<SecCertificate>, String> {
    let result = ItemSearchOptions::new()
        .class(ItemClass::certificate())
        .keychains(&[keychain.clone()])
        .load_refs(true)
        .limit(Limit::All)
        .search();
    let items = match result {
        Ok(items) => items,
        Err(error) if error.code() == -25300 => return Ok(None),
        Err(error) => {
            return Err(system_error(
                error.code(),
                "Не удалось проверить связку ключей пользователя",
            ));
        }
    };
    let expected = certificate.to_der();
    Ok(items.into_iter().find_map(|item| match item {
        SearchResult::Ref(Reference::Certificate(found)) if found.to_der() == expected => Some(found),
        _ => None,
    }))
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

fn read_certificate(path: &Path) -> Result<SecCertificate, String> {
    let der = crate::certificate::certificate_der(path)?;
    SecCertificate::from_der(&der).map_err(|e| e.to_string())
}

fn removal_script(steps: &[&str]) -> String {
    format!(
        "on run argv\ndo shell script ({}) with prompt \"Show My Limits удаляет старый сертификат приложения.\" with administrator privileges\nend run",
        steps.join(r#" & " && " & "#)
    )
}

fn removal_error(code: i32, operation: &str) -> String {
    match code {
        -128 | -60006 => "Удаление отменено. Старый сертификат остался доверенным.".into(),
        _ => system_error(code, operation),
    }
}

/// Remove this exact certificate (matched by DER) and its HTTPS trust from the
/// login and System keychains. Called only from the explicit cleanup button.
/// The user domain asks for the login password; the System keychain and the
/// admin trust domain need one administrator prompt. Nothing else is touched.
pub fn remove_everywhere(path: &Path) -> Result<(), String> {
    let certificate = read_certificate(path)?;
    let has_trust = |domain| matches!(
        TrustSettings::new(domain).tls_trust_settings_for_certificate(&certificate),
        Ok(Some(_))
    ) || TrustSettings::new(domain).iter().is_ok_and(|mut all| all.any(|c| c.to_der() == certificate.to_der()));
    if has_trust(Domain::User) {
        // SAFETY: the certificate wrapper is retained for the duration of the call.
        let status = unsafe {
            SecTrustSettingsRemoveTrustSettings(certificate.as_concrete_TypeRef(), kSecTrustSettingsDomainUser)
        };
        if status != 0 && status != -25300 {
            return Err(removal_error(status, "Не удалось снять доверие пользователя"));
        }
    }
    let login = SecKeychain::default().map_err(|e| e.to_string())?;
    if let Some(found) = find_certificate(&login, &certificate)? {
        found.delete().map_err(|e| removal_error(e.code(), "Не удалось удалить сертификат из связки «Вход»"))?;
    }
    let system = SecKeychain::open("/Library/Keychains/System.keychain").map_err(|e| e.to_string())?;
    let in_system = contains_certificate(&system, &certificate)?;
    let admin_trust = has_trust(Domain::Admin);
    if in_system || admin_trust {
        let sha1 = crate::certificate::sha1_hex(&certificate.to_der());
        // The path and fingerprint reach the shell only through `quoted form of`.
        let mut steps = Vec::new();
        if admin_trust {
            steps.push(r#""/usr/bin/security remove-trusted-cert -d " & quoted form of (item 1 of argv)"#);
        }
        if in_system {
            steps.push(r#""/usr/bin/security delete-certificate -Z " & quoted form of (item 2 of argv) & " /Library/Keychains/System.keychain""#);
        }
        let script = removal_script(&steps);
        let output = Command::new("/usr/bin/osascript")
            .arg("-e").arg(script).arg(path).arg(&sha1)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(if stderr.contains("-128") {
                removal_error(-128, "")
            } else {
                "Не удалось удалить сертификат из системной связки ключей.".into()
            });
        }
    }
    if present_in_checked_keychains(path)? || trust_scope(path)? != "none" {
        return Err("Удаление завершилось, но macOS всё ещё видит старый сертификат. Удалите его в «Связке ключей».".into());
    }
    Ok(())
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
    fn removal_script_compiles_and_quotes_arguments() {
        let steps = [
            r#""/usr/bin/security remove-trusted-cert -d " & quoted form of (item 1 of argv)"#,
            r#""/usr/bin/security delete-certificate -Z " & quoted form of (item 2 of argv) & " /Library/Keychains/System.keychain""#,
        ];
        for script in [removal_script(&steps), removal_script(&steps[..1]), removal_script(&steps[1..])] {
            let dir = tempfile::tempdir().unwrap();
            let status = Command::new("/usr/bin/osacompile")
                .arg("-o").arg(dir.path().join("check.scpt")).arg("-e").arg(&script)
                .status().unwrap();
            assert!(status.success(), "{script}");
        }
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
