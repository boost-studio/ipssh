fn main() {
    println!("cargo:rustc-link-arg-bin=ipssh-installer=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-bin=ipssh-installer=/MANIFESTUAC:level='asInvoker' uiAccess='false'"
    );
}
