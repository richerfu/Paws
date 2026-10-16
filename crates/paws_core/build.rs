use std::{env, fs, path::PathBuf};

fn main() {
    let manifest =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../../Cargo.toml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    let contents = fs::read_to_string(manifest).expect("read workspace manifest");
    let manifest: toml::Value = toml::from_str(&contents).expect("parse workspace manifest");
    let version = manifest["workspace"]["dependencies"]["arkit"]["version"]
        .as_str()
        .and_then(|version| version.strip_prefix('='))
        .expect("arkit must use an exact published version");
    println!("cargo:rustc-env=PAWS_ARKIT_VERSION={version}");
}
