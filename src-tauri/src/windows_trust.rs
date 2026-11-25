//! Exact DER matching against Windows root stores; no trust-by-name or saved flag.
use std::{fs, path::Path, ptr};
use windows_sys::Win32::Security::Cryptography::*;
use x509_parser::pem::parse_x509_pem;
struct Store(HCERTSTORE);
impl Drop for Store {
    fn drop(&mut self) {
        unsafe {
            CertCloseStore(self.0, 0);
        }
    }
}
fn open(scope: u32, name: &str, readonly: bool) -> Result<Store, String> {
    let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let flags = scope
        | CERT_STORE_OPEN_EXISTING_FLAG
        | if readonly {
            CERT_STORE_READONLY_FLAG
        } else {
            0
        };
    let store =
        unsafe { CertOpenStore(CERT_STORE_PROV_SYSTEM_W, 0, 0, flags, name.as_ptr().cast()) };
    if store.is_null() {
        return Err(format!(
            "Не удалось открыть хранилище Windows: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(Store(store))
}
fn der(path: &Path) -> Result<Vec<u8>, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let (_, pem) = parse_x509_pem(&bytes).map_err(|_| "Некорректный сертификат приложения.")?;
    Ok(pem.contents)
}
fn contains(store: &Store, der: &[u8]) -> bool {
    let mut context = ptr::null_mut();
    loop {
        context = unsafe { CertEnumCertificatesInStore(store.0, context) };
        if context.is_null() {
            return false;
        }
        let bytes = unsafe {
            std::slice::from_raw_parts((*context).pbCertEncoded, (*context).cbCertEncoded as usize)
        };
        if bytes == der {
            unsafe {
                CertFreeCertificateContext(context);
            }
            return true;
        }
    }
}
pub fn is_trusted(path: &Path) -> Result<bool, String> {
    let der = der(path)?;
    for scope in [
        CERT_SYSTEM_STORE_CURRENT_USER,
        CERT_SYSTEM_STORE_LOCAL_MACHINE,
    ] {
        if let Ok(disallowed) = open(scope, "Disallowed", true) {
            if contains(&disallowed, &der) {
                return Ok(false);
            }
        }
    }
    for scope in [
        CERT_SYSTEM_STORE_CURRENT_USER,
        CERT_SYSTEM_STORE_LOCAL_MACHINE,
    ] {
        if contains(&open(scope, "ROOT", true)?, &der) {
            return Ok(true);
        }
    }
    Ok(false)
}
pub fn trust_scope(path: &Path) -> Result<&'static str, String> {
    let der = der(path)?;
    let user = contains(&open(CERT_SYSTEM_STORE_CURRENT_USER, "ROOT", true)?, &der);
    let machine = contains(&open(CERT_SYSTEM_STORE_LOCAL_MACHINE, "ROOT", true)?, &der);
    Ok(match (user, machine) {
        (true, true) => "user+system",
        (true, false) => "user",
        (false, true) => "system",
        (false, false) => "none",
    })
}
pub fn install(path: &Path) -> Result<(), String> {
    let der = der(path)?;
    let store = open(CERT_SYSTEM_STORE_CURRENT_USER, "ROOT", false)?;
    // Called only by the user's Install certificate button. Other CAs untouched.
    let installed = unsafe {
        CertAddEncodedCertificateToStore(
            store.0,
            X509_ASN_ENCODING,
            der.as_ptr(),
            der.len() as u32,
            CERT_STORE_ADD_REPLACE_EXISTING,
            ptr::null_mut(),
        )
    };
    if installed == 0 {
        return Err(format!(
            "Windows не разрешила установить сертификат: {}",
            std::io::Error::last_os_error()
        ));
    }
    if !is_trusted(path)? {
        return Err("Сертификат добавлен, но Windows не подтверждает доверие. Проверьте ограничения системы.".into());
    }
    Ok(())
}
/// Remove this exact certificate (by DER) from the Root stores. Called only
/// from the explicit cleanup button. Windows confirms the Current User
/// deletion itself; Local Machine needs UAC and goes through certutil with the
/// SHA-1 of this certificate only.
pub fn remove_everywhere(path: &Path) -> Result<(), String> {
    let der = der(path)?;
    let user = open(CERT_SYSTEM_STORE_CURRENT_USER, "ROOT", false)?;
    let mut context = ptr::null_mut();
    loop {
        context = unsafe { CertEnumCertificatesInStore(user.0, context) };
        if context.is_null() {
            break;
        }
        let bytes = unsafe {
            std::slice::from_raw_parts((*context).pbCertEncoded, (*context).cbCertEncoded as usize)
        };
        if bytes == der {
            // CertDeleteCertificateFromStore frees the duplicate; the
            // enumeration context is freed separately.
            let duplicate = unsafe { CertDuplicateCertificateContext(context) };
            let deleted = unsafe { CertDeleteCertificateFromStore(duplicate) };
            let error = std::io::Error::last_os_error();
            unsafe { CertFreeCertificateContext(context); }
            if deleted == 0 {
                return Err(if error.raw_os_error() == Some(1223) {
                    "Удаление отменено. Старый сертификат остался доверенным.".into()
                } else {
                    format!("Windows не разрешила удалить сертификат: {error}")
                });
            }
            break;
        }
    }
    if contains(&open(CERT_SYSTEM_STORE_LOCAL_MACHINE, "ROOT", true)?, &der) {
        let sha1 = crate::certificate::sha1_hex(&der);
        debug_assert!(sha1.bytes().all(|b| b.is_ascii_hexdigit()));
        let script = format!(
            "$p = Start-Process -FilePath certutil.exe -ArgumentList '-delstore','Root','{sha1}' -Verb RunAs -WindowStyle Hidden -Wait -PassThru; exit $p.ExitCode"
        );
        let status = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() && contains(&open(CERT_SYSTEM_STORE_LOCAL_MACHINE, "ROOT", true)?, &der) {
            return Err("Удаление из хранилища компьютера отменено или не разрешено.".into());
        }
    }
    if trust_scope(path)? != "none" {
        return Err("Удаление завершилось, но Windows всё ещё видит старый сертификат.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matching_uses_der_bytes_in_an_ephemeral_store() {
        let store = unsafe { CertOpenStore(CERT_STORE_PROV_MEMORY, 0, 0, 0, ptr::null()) };
        assert!(!store.is_null());
        let store = Store(store);
        let fixture = tempfile::NamedTempFile::new().unwrap();
        fs::write(fixture.path(), include_bytes!("../tests/fixtures/test-ca.pem")).unwrap();
        let path = fixture.path();
        let der = der(&path).unwrap();
        assert!(!contains(&store, &der));
        assert_ne!(
            unsafe {
                CertAddEncodedCertificateToStore(
                    store.0,
                    X509_ASN_ENCODING,
                    der.as_ptr(),
                    der.len() as u32,
                    CERT_STORE_ADD_REPLACE_EXISTING,
                    ptr::null_mut(),
                )
            },
            0
        );
        assert!(contains(&store, &der));
        let mut different = der.clone();
        *different.last_mut().unwrap() ^= 1;
        assert!(!contains(&store, &different));
        // The in-memory store is destroyed on drop. OS trust is never modified by this test.
    }
}
