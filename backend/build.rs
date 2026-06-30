// build.rs - ensures the embedded static directory always exists so that
// rust-embed can compile even before the frontend has been built. A minimal
// placeholder index.html is written only when none is present, so a real
// frontend build is never overwritten.
use std::{fs, path::PathBuf};

fn main() {
    let static_dir = PathBuf::from("static");
    if !static_dir.exists() {
        let _ = fs::create_dir_all(&static_dir);
    }
    let index = static_dir.join("index.html");
    if !index.exists() {
        let placeholder = "<!doctype html><html><head><meta charset=\"utf-8\">\
            <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
            <title>BazaarLog</title></head><body style=\"background:#000;color:#aaa;\
            font-family:sans-serif;padding:2rem\">\
            <p>Frontend not built yet. Run build.bat to embed the UI.</p></body></html>";
        let _ = fs::write(&index, placeholder);
    }
    println!("cargo:rerun-if-changed=static");
}
