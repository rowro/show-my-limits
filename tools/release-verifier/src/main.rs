use base64::{Engine, engine::general_purpose::STANDARD};
use minisign_verify::{PublicKey, Signature};
use std::{fs, path::Path};
fn main() {
    let a: Vec<_> = std::env::args().collect();
    let conf: serde_json::Value = serde_json::from_slice(&fs::read(&a[1]).unwrap()).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&fs::read(&a[2]).unwrap()).unwrap();
    let key = STANDARD
        .decode(conf["plugins"]["updater"]["pubkey"].as_str().unwrap())
        .unwrap();
    let key = PublicKey::decode(std::str::from_utf8(&key).unwrap()).unwrap();
    assert_eq!(manifest["platforms"].as_object().unwrap().len(), 5);
    for (platform, item) in manifest["platforms"].as_object().unwrap() {
        let version = manifest["version"].as_str().unwrap();
        let suffix = match platform.as_str() {
            "darwin-aarch64" | "darwin-x86_64" => ".app.tar.gz",
            "linux-aarch64" | "linux-x86_64" => ".AppImage",
            "windows-x86_64" => "-setup.exe",
            _ => panic!("Unexpected platform"),
        };
        let filename = format!("show-my-limits_{version}_{platform}{suffix}");
        assert_eq!(
            item["url"].as_str().unwrap(),
            format!(
                "https://github.com/rowro/show-my-limits/releases/download/v{version}/{filename}"
            )
        );
        let bytes = fs::read(Path::new(&a[3]).join(filename)).unwrap();
        let sig = STANDARD
            .decode(item["signature"].as_str().unwrap())
            .unwrap();
        let sig = Signature::decode(std::str::from_utf8(&sig).unwrap()).unwrap();
        key.verify(&bytes, &sig, true).unwrap();
        if let Some(version) = sig
            .trusted_comment()
            .split('\t')
            .find_map(|s| s.strip_prefix("version:"))
        {
            assert_eq!(version, manifest["version"].as_str().unwrap());
        }
        let mut changed = bytes.clone();
        changed[0] ^= 1;
        assert!(key.verify(&changed, &sig, true).is_err());
        println!("{platform}: signature and trusted comment verified; tampered payload rejected");
    }
}
