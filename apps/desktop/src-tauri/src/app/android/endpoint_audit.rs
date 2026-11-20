//! Debug-only real native socket/output entry, used with a disposable Core and emulator.
use super::*;
use jni::{JNIEnv,objects::{JClass,JString}};
use crate::group_output::{Manager,LocalPlayer,Registration};
use futures_util::future::BoxFuture;
struct Independent;
impl LocalPlayer for Independent {fn prepare<'a>(&'a self,_:&'a Value)->BoxFuture<'a,Result<()>>{Box::pin(async{Ok(())})}}
struct Worker {manager:Arc<Manager>}
static WORKER:Mutex<Option<Worker>>=Mutex::new(None);
#[derive(serde::Deserialize)]
struct Input {profile:rmusic_bridge::connection::ConnectionProfile,token:String,endpoint:String,journal:String,#[serde(default)]output:OutputConfig}
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_EndpointDiagnostics_start(mut env:JNIEnv<'_>,_:JClass<'_>,input:JString<'_>) {
    let result=(||->Result<()> {
        let input:String=env.get_string(&input)?.into();let input:Input=serde_json::from_str(&input)?;input.profile.validate()?;
        let mut saved=WORKER.lock().map_err(|_|anyhow::anyhow!("endpoint audit lock unavailable"))?;
        if saved.is_some(){bail!("endpoint audit already running");}
        let store=rmusic_bridge::listens::Store::open(Path::new(&input.journal))?;
        let manager=Arc::new(Manager::default());
        let registered=tauri::async_runtime::block_on(manager.register(Registration{id:uuid::Uuid::new_v4().to_string(),profile:input.profile,token:input.token,endpoint:input.endpoint,name:"Android 同步验收".into(),output:input.output,listens:Some(store),primary:false},Arc::new(Independent)));
        if let Err(error)=registered{tauri::async_runtime::block_on(manager.stop(None,None));return Err(error);}
        *saved=Some(Worker{manager});Ok(())
    })();if let Err(error)=result{let _=env.throw_new("java/lang/IllegalStateException",error.to_string());}
}
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_EndpointDiagnostics_stop(_env:JNIEnv<'_>,_:JClass<'_>) {
    let worker=WORKER.lock().ok().and_then(|mut saved|saved.take());
    if let Some(worker)=worker {tauri::async_runtime::block_on(worker.manager.stop(None,None));}
}
