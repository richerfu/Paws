fn main() {
    println!("cargo:rerun-if-env-changed=PAWS_NSH_REVISION");
}
