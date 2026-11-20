fn main() {
    // Tauri overrides rustflags; apply ARM64 page alignment to the final shared library.
    if std::env::var("TARGET").as_deref() == Ok("aarch64-linux-android") {
        println!("cargo:rustc-link-arg-cdylib=-Wl,-z,max-page-size=16384");
        println!("cargo:rustc-link-arg-cdylib=-Wl,-z,common-page-size=16384");
    }
    #[cfg(feature = "tauri-app")]
    tauri_build::build()
}
