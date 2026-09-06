fn main() {
    // windows "installer detection" auto-elevates exes whose names contain
    // setup/install/uninstall UNLESS the embedded manifest carries an
    // explicit requestedExecutionLevel. rustc's default manifest is an
    // empty <assembly>, so feed ours via /MANIFESTINPUT (verbatim embed;
    // /MANIFESTUAC inline quoting proved fragile through cargo -> link).
    // the whole install is per-user (%LOCALAPPDATA% + HKCU): never needs
    // admin.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("app.manifest");
    println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", manifest.display());
}
