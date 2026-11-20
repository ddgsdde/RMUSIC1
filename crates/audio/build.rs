fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(feature = "asio-output")]
    {
        if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {return;}
        println!("cargo:rerun-if-env-changed=CPAL_ASIO_DIR");
        println!("cargo:rerun-if-changed=src/output/asio_clock.cpp");
        let sdk = std::env::var_os("CPAL_ASIO_DIR")
            .expect("asio-output requires CPAL_ASIO_DIR pointing to the official ASIO SDK");
        cc::Build::new().cpp(true).file("src/output/asio_clock.cpp")
            .include(std::path::Path::new(&sdk).join("common"))
            .compile("rmusic_asio_clock");
    }
}
