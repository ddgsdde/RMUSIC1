//! Android activities are windows; the native runtime also owns background playback.
use super::*;
use std::sync::OnceLock;
static APP:OnceLock<tauri::AppHandle>=OnceLock::new();
pub(super) fn install(app:&tauri::AppHandle) {let _=APP.set(app.clone());}
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_MainActivity_restoreNativeWindow(mut env:jni::JNIEnv<'_>,_:jni::objects::JClass<'_>) {
    // The first Activity precedes Tauri setup and receives its configured window there.
    let Some(app)=APP.get() else{return;};let target=app.clone();
    if let Err(error)=app.run_on_main_thread(move|| {
        let Some(config)=target.config().app.windows.first() else{return;};
        if target.get_webview_window(&config.label).is_some(){return;}
        let result=tauri::WebviewWindowBuilder::from_config(&target,config).and_then(|builder|builder.build());
        if let Err(error)=result {eprintln!("Android window restore failed: {error}");}
    }) {let _=env.throw_new("java/lang/IllegalStateException",error.to_string());}
}

static FOREGROUND:std::sync::atomic::AtomicBool=std::sync::atomic::AtomicBool::new(true);
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_AudioPlugin_playbackChanged(_:jni::JNIEnv<'_>,_:jni::objects::JClass<'_>) {
    if let Some(app)=APP.get(){app.state::<super::super::DesktopState>().playback_controls.lock().unwrap().control.wake_output();}
}
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_AudioPlugin_remoteMediaCommand(
    mut env:jni::JNIEnv<'_>,_:jni::objects::JClass<'_>,
    zone:jni::objects::JString<'_>,media:jni::objects::JString<'_>,action:jni::objects::JString<'_>,
) {
    let Some(app)=APP.get() else{return;};
    let Ok(zone)=env.get_string(&zone) else{return;};let zone:String=zone.into();
    let Ok(media)=env.get_string(&media) else{return;};let media:String=media.into();
    let Ok(action)=env.get_string(&action) else{return;};
    app.state::<super::super::DesktopState>().travel.media_action(zone,media,action.into());
}
pub(crate) fn foreground()->bool{FOREGROUND.load(Ordering::Relaxed)}
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_MainActivity_activityVisible(_:jni::JNIEnv<'_>,_:jni::objects::JClass<'_>,visible:jni::sys::jboolean){
    FOREGROUND.store(visible!=0,Ordering::Relaxed);
    if let Some(app)=APP.get(){app.state::<super::super::DesktopState>().travel.wake(false);app.state::<super::super::DesktopState>().storage.wake();}
}
static NETWORK_CHANGE: Mutex<Option<std::time::Instant>> = Mutex::new(None);

pub(crate) fn network_changed_since(checked: std::time::Instant) -> bool {
    NETWORK_CHANGE.lock().unwrap().is_some_and(|changed| changed > checked)
}

#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_MainActivity_defaultNetworkChanged(
    _: jni::JNIEnv<'_>, _: jni::objects::JClass<'_>,
) {
    *NETWORK_CHANGE.lock().unwrap() = Some(std::time::Instant::now());
    if let Some(app) = APP.get() {
        app.state::<super::super::DesktopState>().travel.wake(true);
        app.state::<super::super::DesktopState>().listens.changes.send_modify(|v|*v=v.wrapping_add(1));
        for window in app.webview_windows().into_values() {
            let _ = window.eval("window.dispatchEvent(new Event('rmusic-network-change'))");
        }
    }
}
