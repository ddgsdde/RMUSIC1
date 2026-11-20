//! Direct worker JNI; shared session locks serialize state without waiting for the UI Looper.
use anyhow::{Context,Result};
use jni::{JNIEnv,JavaVM,objects::{GlobalRef,JClass,JObject,JString,JValue}};
use serde_json::Value;
use std::sync::{Arc,Mutex,OnceLock};
struct Bridge {vm:JavaVM,plugin:GlobalRef}
static BRIDGE:OnceLock<Mutex<Option<Arc<Bridge>>>>=OnceLock::new();
pub(super) fn attach_worker()->Result<()> {
    let bridge=BRIDGE.get().context("Android JNI bridge not initialized")?.lock().map_err(|_|anyhow::anyhow!("Android JNI bridge lock unavailable"))?.clone().context("Android JNI bridge unavailable")?;
    bridge.vm.attach_current_thread_permanently()?;Ok(())
}

pub(super) fn device_name()->Result<String> {
    let bridge=BRIDGE.get().context("Android JNI bridge not initialized")?.lock().map_err(|_|anyhow::anyhow!("Android JNI bridge lock unavailable"))?.clone().context("Android JNI bridge unavailable")?;
    let mut env=bridge.vm.attach_current_thread()?;
    let manufacturer=JString::from(env.get_static_field("android/os/Build","MANUFACTURER","Ljava/lang/String;")?.l()?);
    let model=JString::from(env.get_static_field("android/os/Build","MODEL","Ljava/lang/String;")?.l()?);
    let manufacturer:String=env.get_string(&manufacturer)?.into();let model:String=env.get_string(&model)?.into();
    Ok(if model.to_lowercase().starts_with(&manufacturer.to_lowercase()){model}else{format!("{manufacturer} {model}")})
}

#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_AudioPlugin_registerNativeBridge(mut env:JNIEnv<'_>,_:JClass<'_>,plugin:JObject<'_>) {
    let installed=(||->Result<()> {
        let bridge=Arc::new(Bridge{vm:env.get_java_vm()?,plugin:env.new_global_ref(plugin)?});
        *BRIDGE.get_or_init(Default::default).lock().map_err(|_|anyhow::anyhow!("Android JNI bridge lock unavailable"))?=Some(bridge);Ok(())
    })();
    if let Err(error)=installed {let _=env.throw_new("java/lang/IllegalStateException",error.to_string());}
}
pub(super) fn call(command:&str,args:Value)->Result<Value> {
    let bridge=BRIDGE.get().context("Android JNI bridge not initialized")?.lock()
        .map_err(|_|anyhow::anyhow!("Android JNI bridge lock unavailable"))?.clone().context("Android JNI bridge unavailable")?;
    let mut env=bridge.vm.attach_current_thread()?;
    env.with_local_frame(8,|env|->Result<Value>{
    let command=env.new_string(command)?;let args=env.new_string(args.to_string())?;
    let reply=env.call_method(bridge.plugin.as_obj(),"nativeControls","(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
        &[JValue::Object(&command),JValue::Object(&args)]);
    if reply.is_err(){let _=env.exception_describe();let _=env.exception_clear();}
    let reply=JString::from(reply?.l()?);let text:String=env.get_string(&reply)?.into();
    Ok(serde_json::from_str(&text)?)
    })
}
pub(super) fn write_pcm(id:&str,bytes:&[u8])->Result<usize> {
    let bridge=BRIDGE.get().context("Android JNI bridge not initialized")?.lock().map_err(|_|anyhow::anyhow!("Android JNI bridge lock unavailable"))?.clone().context("Android JNI bridge unavailable")?;
    let mut env=bridge.vm.attach_current_thread()?;
    env.with_local_frame(4,|env|->Result<usize>{let id=env.new_string(id)?;let bytes=env.byte_array_from_slice(bytes)?;
    let reply=env.call_method(bridge.plugin.as_obj(),"writePcm","(Ljava/lang/String;[B)I",&[JValue::Object(&id),JValue::Object(&bytes)]);
    if reply.is_err(){let _=env.exception_describe();let _=env.exception_clear();}
    Ok(reply?.i()?.try_into()?)})
}
pub(super) fn decoder(command:&str,args:Value,output:&mut[u8])->Result<Value> {
    let bridge=BRIDGE.get().context("Android JNI bridge not initialized")?.lock().map_err(|_|anyhow::anyhow!("Android JNI bridge lock unavailable"))?.clone().context("Android JNI bridge unavailable")?;
    let mut env=bridge.vm.attach_current_thread()?;
    env.with_local_frame(8,|env|->Result<Value>{
        let command=env.new_string(command)?;let args=env.new_string(args.to_string())?;let bytes=env.new_byte_array(output.len().try_into()?)?;
        let reply=env.call_method(bridge.plugin.as_obj(),"nativeDecoder","(Ljava/lang/String;Ljava/lang/String;[B)Ljava/lang/String;",&[JValue::Object(&command),JValue::Object(&args),JValue::Object(&bytes)]);
        if reply.is_err(){let _=env.exception_clear();}
        let reply=JString::from(reply?.l()?);let text:String=env.get_string(&reply)?.into();let reply:Value=serde_json::from_str(&text)?;
        if let Some(error)=reply["error"].as_str(){anyhow::bail!("Android 解码失败：{error}");}
        let count=reply["bytes"].as_u64().unwrap_or(0) as usize;
        if count>output.len(){anyhow::bail!("Android decoder exceeded its output buffer");}
        if count>0 {let mut samples=vec![0i8;count];env.get_byte_array_region(&bytes,0,&mut samples)?;for (out,value) in output.iter_mut().zip(samples){*out=value as u8;}}
        Ok(reply)
    })
}
