//! Serialize authorization changes against admitted account mutations. Media
//! bodies do not retain this lock; they revalidate while streaming instead.
use super::*;
use std::{collections::HashMap,sync::{Arc,Mutex,OnceLock,Weak}};
use tokio::sync::{RwLock};
fn lock(s:&AppState,user:&str)->Arc<RwLock<()>>{
 static LOCKS:OnceLock<Mutex<HashMap<String,Weak<RwLock<()>>>>>=OnceLock::new();
 let mut locks=LOCKS.get_or_init(Default::default).lock().unwrap();locks.retain(|_,v|v.strong_count()>0);
 let key=format!("{}:{user}",s.core_id);let value=locks.get(&key).and_then(Weak::upgrade).unwrap_or_default();locks.insert(key,Arc::downgrade(&value));value
}
pub async fn read(s:&AppState,user:&str)->tokio::sync::OwnedRwLockReadGuard<()>{lock(s,user).read_owned().await}
pub async fn write(s:&AppState,user:&str)->tokio::sync::OwnedRwLockWriteGuard<()>{lock(s,user).write_owned().await}
