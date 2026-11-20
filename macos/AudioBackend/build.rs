fn main() {
    println!("cargo:rerun-if-changed=native/feed_wake.h");
    println!("cargo:rerun-if-changed=native/feed_priority.h");
    println!("cargo:rerun-if-changed=native/capabilities.c");
    println!("cargo:rerun-if-changed=native/coreaudio.c");
    println!("cargo:rerun-if-changed=native/coreaudio.h");
    println!("cargo:rerun-if-changed=native/volume.c");
    cc::Build::new().file("native/coreaudio.c").file("native/volume.c").file("native/capabilities.c").flag("-std=c11").warnings(true).compile("rmusic_coreaudio");
    for framework in ["CoreAudio", "AudioUnit", "AudioToolbox", "CoreFoundation"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}
