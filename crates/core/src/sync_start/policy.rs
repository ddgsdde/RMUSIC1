//! Fail-closed monitoring of known clock error. Unknown driver/acoustic error is never hidden.
use super::*;
#[derive(Clone,Debug,Deserialize,Serialize,PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Policy {pub max_known_error_us:u32,pub warmup_ms:u32}
impl Policy {pub(super) fn valid(&self)->bool{(100..=50000).contains(&self.max_known_error_us)&&(1000..=10000).contains(&self.warmup_ms)}}
#[derive(Clone,Default)]pub(super) struct Monitor {last:Option<u64>,since:Option<u64>,good:u32,pub(super) value:Value}
pub(super) fn evaluate(d:&Value,limit:u64,rates:&[u32])->Result<u64,&'static str>{
 if d["state"]!="available"||d["valid_for_ms"].as_u64().is_none_or(|v|v==0){return Err("无法取得全部输出的有效配对时钟");}
 let rows=d["members"].as_array().filter(|a|a.len()==rates.len()&&a.len()>=2).ok_or("输出成员或时钟数量不一致")?;
 let reference=d["reference_zone_id"].as_str().ok_or("参考输出缺失")?;
 let reference_span=rows.iter().find(|r|r["zone_id"]==reference).and_then(|r|r["driver_read_span_ns"].as_u64()).ok_or("参考读取误差缺失")?;
 let mut worst=0u64;
 for(row,rate)in rows.iter().zip(rates){
  if row["available"]!=true||row["projection"]!="observed"||*rate<8000{return Err("设备实际速率尚未收敛");}
  let phase=row["estimated_offset_ns"].as_i64().ok_or("节目偏差缺失")?.unsigned_abs();
  let error=row["known_host_uncertainty_ns"].as_u64().zip(row["processing_projection_uncertainty_ns"].as_u64()).zip(row["driver_read_span_ns"].as_u64()).ok_or("时钟误差边界缺失")?;
  let bound=phase.saturating_add(error.0.0).saturating_add(error.0.1).saturating_add(error.1).saturating_add(reference_span).saturating_add(2_000_000_000/(*rate as u64));
  worst=worst.max(bound);
 }if worst>limit{return Err("节目偏差与已知不确定度超出用户上限");}Ok(worst)
}
impl Monitor {
 fn observe(&mut self,d:&Value,p:&Policy,rates:&[u32],now:u64)->bool{
  let result=evaluate(d,p.max_known_error_us as u64*1000,rates);
  let samples=d["members"].as_array().and_then(|rows|rows.iter().map(|r|r["sample_age_ns"].as_u64().map(|age|now.saturating_sub(age))).min().flatten());
  let fresh=samples.is_some_and(|t|self.last.is_none_or(|old|t>old));
  if fresh {self.last=samples;self.good=if result.is_ok(){self.good.saturating_add(1)}else{0};}
  if result.is_err(){self.good=0;}
  let elapsed=now.saturating_sub(*self.since.get_or_insert(now));
  let verified=result.is_ok()&&self.good>=4;
  self.value=json!({"version":1,"state":if verified{"within_known_bound"}else if elapsed<(p.warmup_ms as u64)*1_000_000{"warming"}else{"violated"},"reason":result.as_ref().err(),"max_known_error_us":p.max_known_error_us,"observed_known_bound_ns":result.ok(),"fresh_samples":self.good,"driver_accuracy_verified":false,"acoustic_accuracy_verified":false,"strict_sync":false,"on_violation":"stop_group"});
  elapsed>=(p.warmup_ms as u64)*1_000_000&&!verified
 }
}
pub(super) async fn tick(job:&mut Job,s:&AppState)->anyhow::Result<()> {
 let Some(policy)=job.request.sync_policy.clone()else{return Ok(());};
 if job.phase!="playing"{job.sync_monitor=Monitor::default();return Ok(());}
 let d=clock_feedback::snapshot(job,s).await;let now=d["evaluated_at_core_ns"].as_u64().unwrap_or_else(crate::clock::now_ns);
 let rates=job.members.iter().map(|m|m.rate).collect::<Vec<_>>();
 if job.sync_monitor.observe(&d,&policy,&rates,now){job.abort(s,"同步误差保证已失效，整组已停止；请检查设备时钟、网络和缓冲",false).await?;}
 Ok(())
}
#[cfg(test)]mod tests{
 use super::*;
 fn diagnostics(at:u64,phase:i64,host:u64)->Value{json!({"state":"available","valid_for_ms":100,"reference_zone_id":"a","members":[{"zone_id":"a","available":true,"projection":"observed","estimated_offset_ns":0,"known_host_uncertainty_ns":0,"processing_projection_uncertainty_ns":0,"driver_read_span_ns":100,"sample_age_ns":0,"stamp":at},{"zone_id":"b","available":true,"projection":"observed","estimated_offset_ns":phase,"known_host_uncertainty_ns":host,"processing_projection_uncertainty_ns":0,"driver_read_span_ns":100,"sample_age_ns":0}]})}
 #[test]fn policy_includes_uncertainty_and_requires_fresh_feedback(){let p=Policy{max_known_error_us:1000,warmup_ms:1000};let mut m=Monitor::default();let rates=[48000,48000];for now in [1,250000001,500000001,750000001]{assert!(!m.observe(&diagnostics(now,500,100),&p,&rates,now));}assert_eq!(m.value["state"],"within_known_bound");assert_eq!(m.value["strict_sync"],false);let mut stale=diagnostics(1,0,0);stale["valid_for_ms"]=json!(0);assert!(m.observe(&stale,&p,&rates,1000000001));assert_eq!(m.value["state"],"violated");assert!(evaluate(&diagnostics(1,0,1000000),1000000,&rates).is_err());}
 #[test]fn repeated_measurement_does_not_warm_a_monitor_and_large_offsets_stop(){let p=Policy{max_known_error_us:1000,warmup_ms:1000};let mut m=Monitor::default();let mut d=diagnostics(1,0,0);for now in [1,250000001,500000001,1000000001]{for r in d["members"].as_array_mut().unwrap(){r["sample_age_ns"]=json!(now-1);}m.observe(&d,&p,&[48000,48000],now);}assert_eq!(m.value["fresh_samples"],1);assert_eq!(m.value["state"],"violated");assert!(evaluate(&diagnostics(1,1000001,0),1000000,&[48000,48000]).is_err());}
}
