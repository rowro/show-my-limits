fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let windows = tauri_build::WindowsAttributes::new()
            .app_manifest(include_str!("windows-app.manifest"));
        tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
            .expect("failed to run the Windows Tauri build script");
    } else {
        tauri_build::build();
    }
}
