fn main() {
    #[cfg(windows)]
    if std::env::args().any(|arg|arg=="--ui-build-info") {
        println!("{}",rmusic_desktop::app::ui_build_info());return;
    }
    if std::env::args().any(|arg|arg=="--build-info") {
        let mut info=rmusic_audio::output::build_info();
        #[cfg(windows)] {info["ui_manifest_v1"]=serde_json::json!(true);}
        println!("{info}");return;
    }
    rmusic_desktop::run();
}
