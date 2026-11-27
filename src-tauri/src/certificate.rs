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

/// Leaf-only store: the trusted CA key is never written. The CA is limited by
/// Name Constraints to the configured servers and signs one leaf for them;
/// mitmproxy receives only that leaf through `--certs`.
pub const LEAF_CHAIN: &str = "leaf-chain.pem";
pub const PUBLIC_CA: &str = "mitmproxy-ca-cert.cer";
const LEGACY_KEY: &str = "mitmproxy-ca.pem";
// Apple rejects user-trusted TLS leaves valid for more than 825 days; keep the
// leaf within the public 398-day limit and inside the CA validity.
const CA_DAYS: i64 = 400;
const LEAF_DAYS: i64 = 397;
const MAX_FILE: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreFormat {
    /// mitmproxy confdir with the CA private key (installations before the leaf-only CA).
    Legacy,
    /// Public CA plus a leaf chain for the configured servers.
    Leaf,
}

impl StoreFormat {
    pub fn as_str(self) -> &'static str {
        match self { StoreFormat::Legacy => "legacy", StoreFormat::Leaf => "leaf" }
    }
}

pub fn store_format(directory: &Path) -> Option<StoreFormat> {
    if complete_legacy_store(directory) { Some(StoreFormat::Legacy) }
    else if leaf_hosts(directory).is_some() { Some(StoreFormat::Leaf) }
    else { None }
}

/// Refuse to let a helper regenerate a partially deleted existing CA.
pub fn complete_store(directory: &Path) -> bool {
    store_format(directory).is_some()
}

fn regular_file(path: &Path) -> Option<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.file_type().is_file() || metadata.len() == 0 || metadata.len() > MAX_FILE {
        return None;
    }
    fs::read(path).ok()
}

fn pem_blocks(mut bytes: &[u8]) -> Option<Vec<x509_parser::pem::Pem>> {
    let mut blocks = Vec::new();
    while !bytes.iter().all(u8::is_ascii_whitespace) {
        let (rest, pem) = parse_x509_pem(bytes).ok()?;
        if rest.len() == bytes.len() { return None; }
        blocks.push(pem);
        bytes = rest;
    }
    Some(blocks)
}

fn dns_names(cert: &X509Certificate) -> Option<Vec<String>> {
    let san = cert.subject_alternative_name().ok()??;
    let mut names = Vec::new();
    for name in &san.value.general_names {
        match name {
            GeneralName::DNSName(dns) => names.push(dns.to_ascii_lowercase()),
            _ => return None,
        }
    }
    names.sort();
    names.dedup();
    (!names.is_empty()).then_some(names)
}

/// Servers covered by a valid leaf-only store, or None if the store is not one.
/// Checks: no CA key on disk, leaf key matches the leaf, the leaf is signed by
/// the public CA, and the CA is constrained to exactly the leaf's names.
pub fn leaf_hosts(directory: &Path) -> Option<Vec<String>> {
    if !fs::symlink_metadata(directory).is_ok_and(|m| m.file_type().is_dir())
        || fs::symlink_metadata(directory.join(LEGACY_KEY)).is_ok() {
        return None;
    }
    let public = regular_file(&directory.join(PUBLIC_CA))?;
    let chain = regular_file(&directory.join(LEAF_CHAIN))?;
    let (_, ca_pem) = parse_x509_pem(&public).ok()?;
    let blocks = pem_blocks(&chain)?;
    let [key, leaf_pem, chain_ca] = blocks.as_slice() else { return None; };
    if ca_pem.label != "CERTIFICATE" || key.label != "PRIVATE KEY" || leaf_pem.label != "CERTIFICATE"
        || chain_ca.label != "CERTIFICATE" || chain_ca.contents != ca_pem.contents {
        return None;
    }
    let (_, ca) = X509Certificate::from_der(&ca_pem.contents).ok()?;
    let (_, leaf) = X509Certificate::from_der(&leaf_pem.contents).ok()?;
    if !ca.is_ca() || leaf.is_ca()
        || ca.verify_signature(None).is_err()
        || leaf.verify_signature(Some(ca.public_key())).is_err() {
        return None;
    }
    let hosts = dns_names(&leaf)?;
    let constraints = ca.name_constraints().ok()??;
    if !constraints.critical { return None; }
    let mut permitted = Vec::new();
    for subtree in constraints.value.permitted_subtrees.as_ref()? {
        match &subtree.base {
            GeneralName::DNSName(dns) => permitted.push(dns.to_ascii_lowercase()),
            _ => return None,
        }
    }
    permitted.sort();
    permitted.dedup();
    if permitted != hosts { return None; }
    let rng = ring::rand::SystemRandom::new();
    let pair = ring::signature::EcdsaKeyPair::from_pkcs8(
        &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING, &key.contents, &rng).ok()?;
    use ring::signature::KeyPair as _;
    (pair.public_key().as_ref() == leaf.public_key().subject_public_key.data.as_ref()).then_some(hosts)
}

fn validate_hosts(hosts: &[String]) -> Result<Vec<String>, String> {
    let mut names: Vec<String> = hosts.iter().map(|h| h.trim().trim_end_matches('.').to_ascii_lowercase()).collect();
    names.sort();
    names.dedup();
    if names.is_empty() {
        return Err("Сначала укажите сервер: сертификат выпускается только для него.".into());
    }
    for name in &names {
        if name.parse::<std::net::IpAddr>().is_ok() {
            return Err(format!("{name} — IP-адрес. Сертификат выпускается только для доменного имени сервера."));
        }
        let valid = name.len() <= 253 && name.contains('.') && name.split('.').all(|label|
            !label.is_empty() && label.len() <= 63 && !label.starts_with('-') && !label.ends_with('-')
                && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
        if !valid {
            return Err(format!("{name} не подходит для сертификата: нужен полный домен сервера."));
        }
    }
    Ok(names)
}

/// Build CA and leaf in memory. The CA signing key is dropped on return and is
/// never serialized. Returns (public CA PEM, leaf chain PEM).
fn issue_leaf_store(hosts: &[String]) -> Result<(String, String), String> {
    use rcgen::{BasicConstraints, CertificateParams, CidrSubnet, DistinguishedName, DnType,
        ExtendedKeyUsagePurpose, GeneralSubtree, IsCa, Issuer, KeyPair, KeyUsagePurpose,
        NameConstraints, SerialNumber, PKCS_ECDSA_P256_SHA256};
    let error = |e: rcgen::Error| format!("Не удалось создать сертификат: {e}");
    let now = ::time::OffsetDateTime::now_utc();
    let serial = || -> Result<SerialNumber, String> {
        let mut bytes = [0u8; 16];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
            .map_err(|_| "Не удалось получить случайные данные.".to_string())?;
        bytes[0] &= 0x7f;
        Ok(SerialNumber::from_slice(&bytes))
    };
    let name = |common: String| {
        let mut dn = DistinguishedName::new();
        dn.push(DnType::OrganizationName, "Show My Limits");
        dn.push(DnType::CommonName, common);
        dn
    };

    let ca_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(error)?;
    let mut ca = CertificateParams::default();
    ca.distinguished_name = name(format!("Show My Limits CA {}", now.date()));
    ca.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    ca.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    // Match the legacy CA so OS trust evaluation of the root behaves the same.
    ca.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    ca.name_constraints = Some(NameConstraints {
        permitted_subtrees: hosts.iter().cloned().map(GeneralSubtree::DnsName).collect(),
        excluded_subtrees: vec![
            GeneralSubtree::IpAddress(CidrSubnet::V4([0; 4], [0; 4])),
            GeneralSubtree::IpAddress(CidrSubnet::V6([0; 16], [0; 16])),
        ],
    });
    ca.not_before = now - ::time::Duration::minutes(5);
    ca.not_after = now + ::time::Duration::days(CA_DAYS);
    ca.serial_number = Some(serial()?);
    let ca_cert = ca.self_signed(&ca_key).map_err(error)?;
    let issuer = Issuer::new(ca, ca_key);

    let leaf_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(error)?;
    let mut leaf = CertificateParams::new(hosts.to_vec()).map_err(error)?;
    leaf.distinguished_name = name(hosts[0].clone());
    leaf.is_ca = IsCa::ExplicitNoCa;
    leaf.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    leaf.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    leaf.use_authority_key_identifier_extension = true;
    leaf.not_before = now - ::time::Duration::minutes(5);
    leaf.not_after = now + ::time::Duration::days(LEAF_DAYS);
    leaf.serial_number = Some(serial()?);
    let leaf_cert = leaf.signed_by(&leaf_key, &issuer).map_err(error)?;
    drop(issuer); // the only copy of the CA private key

    let ca_pem = ca_cert.pem();
    Ok((ca_pem.clone(), format!("{}{}{}", leaf_key.serialize_pem(), leaf_cert.pem(), ca_pem)))
}

/// Create a leaf-only store at `destination`. An existing non-empty directory
/// is never replaced.
pub fn create_leaf_store(hosts: &[String], destination: &Path) -> Result<Vec<String>, String> {
    let hosts = validate_hosts(hosts)?;
    let parent = destination.parent().ok_or("Не найден каталог сертификатов.")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let staging = tempfile::tempdir_in(parent).map_err(|e| e.to_string())?;
    let generated = staging.path().join("generated");
    fs::create_dir(&generated).map_err(|e| e.to_string())?;
    secure_new_store(&generated)?;
    let (ca_pem, chain_pem) = issue_leaf_store(&hosts)?;
    write_private(&generated.join(PUBLIC_CA), ca_pem.as_bytes())?;
    write_private(&generated.join(LEAF_CHAIN), chain_pem.as_bytes())?;
    secure_new_store(&generated)?;
    if leaf_hosts(&generated).as_ref() != Some(&hosts) {
        return Err("Созданный сертификат не прошёл проверку; ничего не сохранено.".into());
    }
    if fs::symlink_metadata(destination).is_ok() {
        // Remove only an empty directory. Any existing file blocks replacement.
        fs::remove_dir(destination).map_err(|_|
            "В каталоге уже есть сертификат или ключ. Они сохранены без изменений.".to_string())?;
    }
    fs::rename(&generated, destination).map_err(|e| format!("Не удалось сохранить сертификат: {e}"))?;
    Ok(hosts)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    options.open(path).and_then(|mut file| file.write_all(bytes))
        .map_err(|e| format!("Не удалось записать сертификат: {e}"))
}

/// Directory for mitmproxy's own CA when a leaf-only store is active. That CA
/// is never trusted; with `--allow-hosts` it only signs nothing a client accepts.
pub fn untrusted_confdir(proxy: &Path) -> std::path::PathBuf {
    proxy.join("untrusted-confdir")
}

/// mitmproxy TLS arguments for the store at `ca_dir` and the servers to capture.
pub fn capture_tls_args(ca_dir: &Path, proxy: &Path, hosts: &[String]) -> Result<Vec<String>, String> {
    match store_format(ca_dir) {
        Some(StoreFormat::Legacy) => Ok(vec!["--set".into(), format!("confdir={}", ca_dir.display())]),
        Some(StoreFormat::Leaf) => {
            let covered = leaf_hosts(ca_dir).unwrap_or_default();
            let missing: Vec<_> = hosts.iter().filter(|h| !covered.contains(h)).cloned().collect();
            if !missing.is_empty() {
                return Err(format!(
                    "Сертификат выпущен для {}, а выбран {}. Создайте новый сертификат для этого сервера в настройках «Захват».",
                    covered.join(", "), missing.join(", ")));
            }
            let confdir = untrusted_confdir(proxy);
            fs::create_dir_all(&confdir).map_err(|e| format!("Не удалось подготовить каталог перехватчика: {e}"))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&confdir, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
            }
            let chain = ca_dir.join(LEAF_CHAIN);
            let mut args = vec!["--set".into(), format!("confdir={}", confdir.display())];
            for host in hosts {
                args.push("--certs".into());
                args.push(format!("{host}={}", chain.display()));
            }
            Ok(args)
        }
        None => Err("Хранилище CA неполное, содержит ссылку или файлы сертификата не совпадают. Существующие файлы сохранены; проверьте их перед восстановлением из резервной копии.".into()),
    }
}

fn complete_legacy_store(directory: &Path) -> bool {
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

/// Uppercase SHA-1 of a DER certificate: the identifier OS tools use to
/// address one exact certificate (`security -Z`, `certutil -delstore`).
pub fn sha1_hex(der: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, der)
        .as_ref().iter().map(|b| format!("{b:02X}")).collect()
}

/// DER of the first certificate in a PEM file.
pub fn certificate_der(path: &Path) -> Result<Vec<u8>, String> {
    let bytes = fs::read(path).map_err(|e| format!("Не удалось прочитать сертификат: {e}"))?;
    let (_, pem) = parse_x509_pem(&bytes).map_err(|_| "Некорректный сертификат.".to_string())?;
    if pem.label != "CERTIFICATE" {
        return Err("Некорректный сертификат.".into());
    }
    Ok(pem.contents)
}

/// Files under /usr/local/share/ca-certificates that hold exactly this one
/// certificate. A file with other certificates, or a copy under the
/// package-managed /usr/share tree, is never removed automatically: `None`.
#[cfg(target_os = "linux")]
pub fn debian_local_sources(cert_path: &Path) -> Option<Vec<std::path::PathBuf>> {
    if source_tree_contains(cert_path, &[Path::new("/usr/share/ca-certificates")]) != Some(false) {
        return None;
    }
    let expected = certificate_der(cert_path).ok()?;
    let mut found = Vec::new();
    let mut pending = vec![std::path::PathBuf::from("/usr/local/share/ca-certificates")];
    while let Some(path) = pending.pop() {
        let entries = match fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return None,
        };
        for entry in entries {
            let entry = entry.ok()?;
            let kind = entry.file_type().ok()?;
            if kind.is_symlink() { return None; }
            if kind.is_dir() { pending.push(entry.path()); continue; }
            if !kind.is_file() || entry.path().extension().is_none_or(|ext| ext != "crt") { continue; }
            let blocks = pem_blocks(&regular_file(&entry.path())?)?;
            if blocks.iter().any(|block| block.contents == expected) {
                if blocks.len() != 1 { return None; }
                found.push(entry.path());
            }
        }
    }
    Some(found)
}

/// Trust of the exact CA in the stores the platform check covers. `None` means
/// it could not be checked (or no check exists on this platform).
pub fn os_trust(path: &Path) -> Option<bool> {
    #[cfg(target_os = "macos")]
    { crate::macos_trust::is_trusted(path).ok() }
    #[cfg(target_os = "windows")]
    { crate::windows_trust::is_trusted(path).ok() }
    #[cfg(target_os = "linux")]
    { debian_system_trust(path) }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    { let _ = path; None }
}

pub fn os_trust_scope(path: &Path) -> Option<String> {
    #[cfg(target_os = "macos")]
    { crate::macos_trust::trust_scope(path).ok().map(str::to_owned) }
    #[cfg(target_os = "windows")]
    { crate::windows_trust::trust_scope(path).ok().map(str::to_owned) }
    #[cfg(target_os = "linux")]
    { debian_system_trust(path).map(|present| if present { "system" } else { "none" }.to_owned()) }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    { let _ = path; None }
}

fn has_material(directory: &Path) -> bool {
    match fs::symlink_metadata(directory) {
        Err(_) => false,
        Ok(meta) if meta.file_type().is_dir() => fs::read_dir(directory).map_or(true, |mut d| d.next().is_some()),
        Ok(_) => true,
    }
}

/// Public certificates in a store that a client could have been told to trust.
fn trust_candidates(directory: &Path) -> Vec<std::path::PathBuf> {
    [PUBLIC_CA, "mitmproxy-ca-cert.pem"].iter().map(|name| directory.join(name))
        .filter(|path| fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file()) && details(path).is_some())
        .collect()
}

/// A store left by an older version that never became usable: legacy format
/// (CA key on disk) or incomplete, and none of its certificates is trusted by
/// a checked OS store. Leaf-only stores have their own rules. `trust` returns
/// (trusted, scope) for one certificate.
pub fn abandoned_store(active: &Path, trust: &dyn Fn(&Path) -> (Option<bool>, Option<String>)) -> bool {
    if !has_material(active) || store_format(active) == Some(StoreFormat::Leaf) {
        return false;
    }
    let Ok((_, retired)) = rotation_paths(active) else { return false; };
    if fs::symlink_metadata(&retired).is_ok() {
        return false;
    }
    // Unknown trust (None) is never treated as untrusted.
    trust_candidates(active).iter().all(|cert| {
        let (trusted, scope) = trust(cert);
        trusted == Some(false) && scope.as_deref() == Some("none")
    })
}

/// Move an abandoned store into the retired slot, where it waits for the
/// explicit cleanup, and publish a leaf-only store for `hosts`. Nothing is
/// deleted here; on failure the old store is moved back.
pub fn replace_abandoned_store(active: &Path, hosts: &[String]) -> Result<Vec<String>, String> {
    validate_hosts(hosts)?;
    let (_, retired) = rotation_paths(active)?;
    if fs::symlink_metadata(&retired).is_ok() {
        return Err("Предыдущий CA ещё ожидает очистки; сначала завершите её.".into());
    }
    fs::rename(active, &retired).map_err(|e| format!("Не удалось отложить прежний сертификат: {e}"))?;
    create_leaf_store(hosts, active).or_else(|error| match fs::rename(&retired, active) {
        Ok(()) => Err(format!("{error} Прежний сертификат возвращён на место.")),
        Err(rollback) => Err(format!("{error} Не удалось вернуть прежний сертификат ({rollback}); он сохранён в {}.", retired.display())),
    })
}

/// Certificate of the retired store that cleanup must check, if any is readable.
/// A store without a readable certificate holds nothing a client could trust.
pub fn retired_certificates(active: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    Ok(trust_candidates(&rotation_paths(active)?.1))
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
    let has_material = ["mitmproxy-ca-cert.cer", "mitmproxy-ca.pem", "mitmproxy-dhparam.pem", LEAF_CHAIN]
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

/// Windows fallback when the current user's Root store is refused.
#[cfg(target_os = "windows")]
pub fn install_machine(path: &Path) -> Result<(), String> {
    if details(path).is_none() {
        return Err("Сначала создайте корректный сертификат приложения.".into());
    }
    crate::windows_trust::install_machine(path)
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
    fn hosts(names: &[&str]) -> Vec<String> { names.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn leaf_store_keeps_no_ca_key_and_covers_only_configured_servers() {
        let root = tempfile::tempdir().unwrap();
        let store = root.path().join("certificates");
        let created = create_leaf_store(&hosts(&["AI.Example.com", "api.example.org"]), &store).unwrap();
        assert_eq!(created, hosts(&["ai.example.com", "api.example.org"]));
        assert_eq!(store_format(&store), Some(StoreFormat::Leaf));
        assert!(complete_store(&store));
        assert_eq!(leaf_hosts(&store), Some(created.clone()));
        assert!(!store.join(LEGACY_KEY).exists());
        let public = fs::read_to_string(store.join(PUBLIC_CA)).unwrap();
        assert!(!public.contains("PRIVATE KEY"));
        let chain = fs::read_to_string(store.join(LEAF_CHAIN)).unwrap();
        assert_eq!(chain.matches("PRIVATE KEY-----").count(), 2, "exactly one key: the leaf's");
        assert!(details(&store.join(PUBLIC_CA)).unwrap().name.starts_with("Show My Limits CA "));
        assert_eq!(security_issue(&store), None);

        let proxy = root.path().join("proxy");
        let args = capture_tls_args(&store, &proxy, &hosts(&["ai.example.com"])).unwrap();
        assert!(args.contains(&"--certs".to_string()));
        assert!(args.iter().any(|a| a.starts_with("ai.example.com=") && a.ends_with(LEAF_CHAIN)));
        assert!(args.iter().any(|a| a == &format!("confdir={}", untrusted_confdir(&proxy).display())));
        let error = capture_tls_args(&store, &proxy, &hosts(&["other.example.net"])).unwrap_err();
        assert!(error.contains("other.example.net"));

        // An existing store is never overwritten.
        assert!(create_leaf_store(&hosts(&["ai.example.com"]), &store).is_err());
        assert_eq!(leaf_hosts(&store), Some(created));
    }

    #[test]
    fn abandoned_store_is_moved_aside_only_when_untrusted() {
        let root = tempfile::tempdir().unwrap();
        let active = root.path().join("certificates");
        let (_, retired) = rotation_paths(&active).unwrap();
        let other = root.path().join("other");
        create_leaf_store(&hosts(&["old.example.com"]), &other).unwrap();
        fs::create_dir(&active).unwrap();
        fs::copy(other.join(PUBLIC_CA), active.join(PUBLIC_CA)).unwrap();
        fs::write(active.join(LEGACY_KEY), b"legacy key material").unwrap();
        let untrusted = |_: &Path| (Some(false), Some("none".to_string()));
        let trusted = |_: &Path| (Some(true), Some("user".to_string()));
        let elsewhere = |_: &Path| (Some(false), Some("admin".to_string()));
        let unknown = |_: &Path| (None, None);
        assert!(!abandoned_store(&active, &trusted));
        assert!(!abandoned_store(&active, &elsewhere));
        assert!(!abandoned_store(&active, &unknown), "unknown trust is never treated as untrusted");
        assert!(abandoned_store(&active, &untrusted));

        assert!(replace_abandoned_store(&active, &hosts(&["10.0.0.1"])).is_err());
        assert!(active.join(LEGACY_KEY).exists() && !retired.exists(), "invalid hosts move nothing");
        let created = replace_abandoned_store(&active, &hosts(&["corp.example.com"])).unwrap();
        assert_eq!(leaf_hosts(&active), Some(created));
        assert_eq!(fs::read(retired.join(LEGACY_KEY)).unwrap(), b"legacy key material", "old key kept for explicit cleanup");
        assert_eq!(retired_certificates(&active).unwrap(), vec![retired.join(PUBLIC_CA)]);
        // A leaf store, and any store while cleanup is pending, stays as is.
        assert!(!abandoned_store(&active, &untrusted));
        assert!(replace_abandoned_store(&active, &hosts(&["corp.example.com"])).is_err());
    }

    #[test]
    fn incomplete_store_without_certificate_is_abandoned() {
        let root = tempfile::tempdir().unwrap();
        let active = root.path().join("certificates");
        let unknown = |_: &Path| (None, None);
        assert!(!abandoned_store(&active, &unknown));
        fs::create_dir(&active).unwrap();
        assert!(!abandoned_store(&active, &unknown), "an empty directory is simply reused");
        fs::write(active.join(LEGACY_KEY), b"partial").unwrap();
        fs::write(active.join(PUBLIC_CA), b"not a certificate").unwrap();
        assert!(abandoned_store(&active, &unknown), "nothing readable could have been trusted");
        replace_abandoned_store(&active, &hosts(&["corp.example.com"])).unwrap();
        assert_eq!(store_format(&active), Some(StoreFormat::Leaf));
        assert!(retired_certificates(&active).unwrap().is_empty());
    }

    #[test]
    fn leaf_store_rejects_tampering() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first");
        let second = root.path().join("second");
        create_leaf_store(&hosts(&["ai.example.com"]), &first).unwrap();
        create_leaf_store(&hosts(&["ai.example.com"]), &second).unwrap();
        // CA from another store: the leaf is no longer signed by the trusted CA.
        fs::copy(second.join(PUBLIC_CA), first.join(PUBLIC_CA)).unwrap();
        assert_eq!(store_format(&first), None);
        assert!(security_issue(&first).is_some());
        // A CA key beside the leaf turns it into an inconsistent legacy store.
        fs::write(second.join(LEGACY_KEY), b"key").unwrap();
        assert_eq!(store_format(&second), None);
    }

    #[test]
    fn leaf_store_refuses_ip_addresses_and_bare_names() {
        let root = tempfile::tempdir().unwrap();
        for bad in ["10.0.0.1", "::1", "localhost", "-bad.example.com", ""] {
            assert!(create_leaf_store(&hosts(&[bad]), &root.path().join("store")).is_err(), "{bad}");
        }
        assert!(create_leaf_store(&[], &root.path().join("store")).is_err());
        assert!(!root.path().join("store").exists());
    }

    #[cfg(unix)]
    #[test]
    fn leaf_store_name_constraints_are_enforced_by_openssl() {
        use std::process::Command;
        if Command::new("openssl").arg("version").output().is_err() { return; }
        let root = tempfile::tempdir().unwrap();
        let store = root.path().join("store");
        create_leaf_store(&hosts(&["ai.example.com"]), &store).unwrap();
        let chain = fs::read_to_string(store.join(LEAF_CHAIN)).unwrap();
        let start = chain.find("-----BEGIN CERTIFICATE-----").unwrap();
        let end = chain.find("-----END CERTIFICATE-----").unwrap() + "-----END CERTIFICATE-----\n".len();
        fs::write(root.path().join("leaf.pem"), &chain[start..end]).unwrap();
        let verify = |host: &str| Command::new("openssl")
            .args(["verify", "-CAfile"]).arg(store.join(PUBLIC_CA))
            .args(["-verify_hostname", host]).arg(root.path().join("leaf.pem"))
            .output().unwrap().status.success();
        assert!(verify("ai.example.com"));
        assert!(!verify("evil.example.com"));
        let text = Command::new("openssl").args(["x509", "-noout", "-text", "-in"]).arg(store.join(PUBLIC_CA))
            .output().unwrap();
        let text = String::from_utf8_lossy(&text.stdout);
        assert!(text.contains("X509v3 Name Constraints: critical"));
        assert!(text.contains("DNS:ai.example.com"));
    }

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
