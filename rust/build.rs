// tauri-build gives the wizard apps a manifest asking for common controls v6.
// a test exe has none, so comctl32 resolves to the v5 copy in System32, which
// lacks TaskDialogIndirect and the loader refuses to start it. cargo only
// honours the -tests form for tests/, not the lib's own test exe, and a
// dependency's link args never reach the wizard binaries.
fn main() {
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if !msvc {
        return;
    }
    let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("common-controls-v6.manifest");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
}
