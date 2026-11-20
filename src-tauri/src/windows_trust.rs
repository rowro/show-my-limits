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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matching_uses_der_bytes_in_an_ephemeral_store() {
        let store = unsafe { CertOpenStore(CERT_STORE_PROV_MEMORY, 0, 0, 0, ptr::null()) };
        assert!(!store.is_null());
        let store = Store(store);
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test-ca.pem");
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
