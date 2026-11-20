use std::ffi::c_void;
#[repr(C)]
struct Subscription {callback:unsafe extern "system" fn(*mut c_void,u32,*mut c_void)->u32,context:*mut c_void}
// Immutable static ABI data; context is always null and the callback uses only REGISTRY.
unsafe impl Sync for Subscription {}
static SUBSCRIPTION:Subscription=Subscription{callback:notification,context:std::ptr::null_mut()};
#[link(name="powrprof")]
unsafe extern "system" {fn PowerRegisterSuspendResumeNotification(flags:u32,recipient:*const c_void,handle:*mut *mut c_void)->u32;}
unsafe extern "system" fn notification(_context:*mut c_void,kind:u32,_setting:*mut c_void)->u32 {
    if let Some(Ok(registry))=super::REGISTRY.get(){registry.event(kind);}0
}

pub(super) fn register()->Result<usize,String> {
    let mut handle=std::ptr::null_mut();
    let code=unsafe{PowerRegisterSuspendResumeNotification(2,&SUBSCRIPTION as *const _ as *const c_void,&mut handle)};
    if code==0{Ok(handle as usize)}else{Err(format!("Windows 错误 {code}"))}
}
