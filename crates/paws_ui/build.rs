fn main() {
    println!("cargo:rerun-if-env-changed=PAWS_NSH_REVISION");
    napi_build_ohos::setup();
}
