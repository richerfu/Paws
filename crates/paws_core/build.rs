use std::{env, fs, path::PathBuf};

fn main() {
    let manifest =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../../Cargo.toml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    let contents = fs::read_to_string(manifest).expect("read workspace manifest");
    let dependency = contents
        .lines()
        .find(|line| line.starts_with("arkit = "))
        .expect("workspace must declare arkit");
    let revision = dependency
        .split("rev = \"")
        .nth(1)
        .and_then(|value| value.split('"').next())
        .expect("arkit must use a pinned commit");
    assert!(
        revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "arkit must use a full git commit"
    );
    println!("cargo:rustc-env=PAWS_ARKIT_REV={revision}");
}
