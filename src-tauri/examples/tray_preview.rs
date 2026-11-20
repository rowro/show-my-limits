#[path = "../src/tray_image.rs"]
mod tray_image;
fn main() {
    let root = std::path::PathBuf::from(std::env::args_os().nth(1).expect("output directory"));
    std::fs::create_dir_all(&root).unwrap();
    for (name, title) in [
        ("unknown", "—"),
        ("zero", "0%"),
        ("normal", "65%"),
        ("full", "100%"),
    ] {
        for light in [true, false] {
            let icon = tray_image::render(title, light);
            std::fs::write(root.join(format!("{name}-{light}.rgba")), icon.rgba()).unwrap();
        }
    }
}
