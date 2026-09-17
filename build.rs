#![allow(clippy::expect_used)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set by cargo"));

    if env::var("TARGET").is_ok_and(|target| target.starts_with("wasm32"))
        && env::var("PROFILE").is_ok_and(|profile| profile == "release")
    {
        let bundle = fs::read("frontend/dist/plugin.js")
            .expect("Build the frontend with npm run build before compiling release WASM");
        assert!(
            !bundle.is_empty(),
            "Release frontend bundle must not be empty"
        );
    }
    stage("frontend/dist/plugin.js", &out_dir.join("plugin.js"));

    let css = [
        "frontend/dist/plugin.css",
        "frontend/dist/plugin-fastdl-frontend.css",
        "frontend/dist/style.css",
    ]
    .iter()
    .find(|path| fs::metadata(path).is_ok())
    .copied();
    match css {
        Some(path) => stage(path, &out_dir.join("plugin.css")),
        None => fs::write(out_dir.join("plugin.css"), b"").expect("write empty css stub"),
    }

    println!("cargo:rerun-if-changed=frontend/dist");
}

fn stage(source: &str, destination: &Path) {
    match fs::read(source) {
        Ok(bytes) => fs::write(destination, bytes).expect("stage frontend asset"),
        Err(_) => {
            println!("cargo:warning=missing {source}; building without frontend bundle");
            fs::write(destination, b"").expect("write empty asset stub");
        }
    }
}
