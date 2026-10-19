//! Fetch the official signed `wintun.dll` so Windows + `listener-tun`
//! builds can `include_bytes!` it as a last-resort sidecar.
//!
//! Non-Windows targets, and Windows builds without `listener-tun`, skip
//! the download. Override the file with `MEOW_WINTUN_DLL=/path/to/wintun.dll`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const WINTUN_VERSION: &str = "0.14.1";
const WINTUN_URL: &str = "https://www.wintun.net/builds/wintun-0.14.1.zip";
const WINTUN_SHA256: &str = "07c256185d6ee3652e09fa55c0b673e2624b565e02c4b9091c79ca7d2f24ef51";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../scripts/fetch-wintun.sh");
    println!("cargo:rerun-if-env-changed=MEOW_WINTUN_DLL");

    if env::var("CARGO_CFG_TARGET_OS").ok().as_deref() != Some("windows") {
        return;
    }
    if env::var("CARGO_FEATURE_LISTENER_TUN").is_err() {
        return;
    }

    if let Err(e) = prepare_wintun() {
        panic!(
            "failed to embed wintun.dll: {e}\n\
             Set MEOW_WINTUN_DLL to a signed wintun.dll, or run \
             scripts/fetch-wintun.sh --target $TARGET --outdir <dir>."
        );
    }
}

fn prepare_wintun() -> Result<(), String> {
    let dest = out_dll();
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }

    if let Ok(src) = env::var("MEOW_WINTUN_DLL") {
        let src = PathBuf::from(src);
        if !src.is_file() {
            return Err(format!("MEOW_WINTUN_DLL is not a file: {}", src.display()));
        }
        fs::copy(&src, &dest).map_err(|e| format!("copy {}: {e}", src.display()))?;
        return Ok(());
    }

    if dest.is_file() && dest.metadata().is_ok_and(|m| m.len() > 0) {
        return Ok(());
    }

    let outdir = dest
        .parent()
        .expect("OUT_DIR/wintun/wintun.dll has a parent");
    if let Some(e) = fetch_with_script(outdir).err() {
        fetch_with_python(outdir).map_err(|py| format!("script: {e}; python: {py}"))?;
    }
    if !dest.is_file() {
        return Err(format!("fetch succeeded but {} is missing", dest.display()));
    }
    Ok(())
}

fn out_dll() -> PathBuf {
    PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("wintun/wintun.dll")
}

fn fetch_with_script(outdir: &Path) -> Result<(), String> {
    let script = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"))
        .join("../../scripts/fetch-wintun.sh");
    if !script.is_file() {
        return Err(format!("{} not found", script.display()));
    }
    let target = env::var("TARGET").unwrap_or_else(|_| "x86_64-pc-windows-msvc".into());
    let status = Command::new("bash")
        .arg(&script)
        .arg("--target")
        .arg(target)
        .arg("--outdir")
        .arg(outdir)
        .status()
        .map_err(|e| format!("spawn bash: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("fetch-wintun.sh exited {status}"))
    }
}

fn fetch_with_python(outdir: &Path) -> Result<(), String> {
    let arch = wintun_arch()?;
    // `NamedTemporaryFile` stays open on Windows and the second open of the
    // same path raises WinError 32. mkstemp + close, then unlink after the
    // zip handle is dropped.
    let py = r#"
import hashlib, os, sys, tempfile, urllib.request, zipfile
url, expect, arch, outdir = sys.argv[1:5]
os.makedirs(outdir, exist_ok=True)
fd, tmp = tempfile.mkstemp(suffix=".zip")
os.close(fd)
try:
    with urllib.request.urlopen(url) as src, open(tmp, "wb") as dst:
        dst.write(src.read())
    with open(tmp, "rb") as f:
        digest = hashlib.sha256(f.read()).hexdigest()
    if digest != expect:
        raise SystemExit(f"SHA-256 mismatch: expected {expect}, got {digest}")
    with zipfile.ZipFile(tmp) as zf:
        src_name = f"wintun/bin/{arch}/wintun.dll"
        with zf.open(src_name) as src, open(os.path.join(outdir, "wintun.dll"), "wb") as dst:
            dst.write(src.read())
        try:
            with zf.open("wintun/LICENSE.txt") as src, open(os.path.join(outdir, "LICENSE.txt"), "wb") as dst:
                dst.write(src.read())
        except KeyError:
            pass
finally:
    try:
        os.unlink(tmp)
    except OSError:
        pass
"#;
    for bin in ["python3", "python"] {
        match Command::new(bin)
            .arg("-c")
            .arg(py)
            .arg(WINTUN_URL)
            .arg(WINTUN_SHA256)
            .arg(arch)
            .arg(outdir)
            .status()
        {
            Ok(st) if st.success() => return Ok(()),
            Ok(st) => return Err(format!("{bin} exited {st}")),
            Err(_) => continue,
        }
    }
    Err(format!(
        "no python interpreter (needed to fetch Wintun {WINTUN_VERSION})"
    ))
}

fn wintun_arch() -> Result<&'static str, String> {
    match env::var("CARGO_CFG_TARGET_ARCH")
        .unwrap_or_default()
        .as_str()
    {
        "x86_64" => Ok("amd64"),
        "aarch64" => Ok("arm64"),
        "x86" => Ok("x86"),
        "arm" => Ok("arm"),
        other => Err(format!("unsupported Windows arch for Wintun: {other}")),
    }
}
