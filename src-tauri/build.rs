fn main() {
    #[cfg(feature = "gui")]
    if let Err(e) = tauri_build::try_build(tauri_build::Attributes::new()) {
        eprintln!("tauri_build warning: {e}");
    }
}
