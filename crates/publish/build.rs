//! Embeds the WASM viewer engine (`vizardcraft_wasm.wasm`) when it has been built: either the path
//! in `VIZARDCRAFT_VIEWER_WASM` or `target/wasm32-unknown-unknown/release/vizardcraft_wasm.wasm`
//! (see `packaging/build-wasm.sh`). Without it, bundles are published in static mode.

use std::path::PathBuf;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(has_viewer_wasm)");
    println!("cargo:rerun-if-env-changed=VIZARDCRAFT_VIEWER_WASM");
    println!("cargo:rerun-if-changed=build.rs");
    // The engine itself is compiled for wasm32: it never embeds itself.
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        return;
    }
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let default = manifest.join("../../target/wasm32-unknown-unknown/release/vizardcraft_wasm.wasm");
    let path = std::env::var("VIZARDCRAFT_VIEWER_WASM").map(PathBuf::from).unwrap_or(default);
    // Watch the file, or (while it doesn't exist) the nearest existing folder above it: a missing
    // file would make Cargo rerun this script on every build.
    let mut watch = path.clone();
    while !watch.exists() {
        match watch.parent() {
            Some(p) => watch = p.to_path_buf(),
            None => break,
        }
    }
    println!("cargo:rerun-if-changed={}", watch.display());
    if path.is_file() {
        let abs = path.canonicalize().unwrap_or(path);
        println!("cargo:rustc-cfg=has_viewer_wasm");
        println!("cargo:rustc-env=VIZARDCRAFT_VIEWER_WASM_PATH={}", abs.display());
    }
}
