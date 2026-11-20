//! Four-timestamp mapping of a bridge host's monotonic clock, not its DAC clock.
use serde_json::{json,Value};
use std::{sync::OnceLock,time::Instant};
const MAX_RTT:u64=500_000_000;
const MAX_AGE:u64=6_000_000_000;
struct Epoch {id:String,start:Instant}
fn epoch()->&'static Epoch {static CLOCK:OnceLock<Epoch>=OnceLock::new();CLOCK.get_or_init(||Epoch{id:uuid::Uuid::new_v4().to_string(),start:Instant::now()})}
pub(crate) fn now_ns()->u64 {epoch().start.elapsed().as_nanos().min(i64::MAX as u128) as u64}
#[derive(Clone)]
struct Sample {host:String,offset:i64,rtt:u64,observed:u64}
#[derive(Default,Clone)]
pub(crate) struct Filter {pending:Option<(String,u64)>,samples:Vec<Sample>}
impl Filter {
    pub(crate) fn probe(&mut self,endpoint:&str,now:u64)->Value {
        let id=uuid::Uuid::new_v4().to_string();self.pending=Some((id.clone(),now));
        json!({"type":"clock_probe","version":1,"endpoint_id":endpoint,"core_clock_id":epoch().id,"probe_id":id,"t1_ns":now})
    }
    pub(crate) fn accept(&mut self,reply:&Value,now:u64)->bool {
        let Some((id,t1))=self.pending.as_ref() else{return false;};
        if reply["version"]!=1||reply["core_clock_id"]!=epoch().id||reply["probe_id"]!=*id||reply["t1_ns"]!=*t1{return false;}
        let t1=*t1;self.pending=None;
        let Some(host)=reply["host_clock_id"].as_str().filter(|s|uuid::Uuid::parse_str(s).is_ok()) else{return false;};
        let Some(t2)=reply["t2_ns"].as_u64().filter(|n|*n<=i64::MAX as u64) else{return false;};
        let Some(t3)=reply["t3_ns"].as_u64().filter(|n|*n<=i64::MAX as u64) else{return false;};
        let Some(elapsed)=now.checked_sub(t1).filter(|n|*n<=MAX_RTT) else{return false;};
        let Some(processing)=t3.checked_sub(t2).filter(|n|*n<=elapsed) else{return false;};
        let rtt=elapsed-processing;
        let offset=((t2 as i128-t1 as i128+t3 as i128-now as i128)/2) as i64;
        self.samples.retain(|s|s.host==host&&now.saturating_sub(s.observed)<=MAX_AGE);
        if self.samples.len()==8{self.samples.remove(0);}
        self.samples.push(Sample{host:host.into(),offset,rtt,observed:now});true
    }
    pub(crate) fn estimate(&self,now:u64)->Value {
        let fresh:Vec<_>=self.samples.iter().filter(|s|now.checked_sub(s.observed).is_some_and(|age|age<=MAX_AGE)).collect();
        let Some(best)=fresh.iter().min_by_key(|s|s.rtt) else{return Value::Null;};
        json!({"version":1,"core_clock_id":epoch().id,"host_clock_id":best.host,"host_minus_core_ns":best.offset,
            "network_rtt_ns":best.rtt,"network_uncertainty_ns":best.rtt.div_ceil(2),"observed_at_core_ns":best.observed,
            "valid_until_core_ns":best.observed+MAX_AGE,"sample_count":fresh.len(),"device_clock_mapped":false})
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn reply(probe:&Value,host:&str,t2:u64,t3:u64)->Value {let mut r=probe.clone();r["type"]=json!("clock_reply");r["host_clock_id"]=json!(host);r["t2_ns"]=json!(t2);r["t3_ns"]=json!(t3);r}
    #[test]
    fn four_timestamp_offsets_remove_remote_processing_and_keep_network_bounds() {
        let mut f=Filter::default();let host=uuid::Uuid::new_v4().to_string();let p=f.probe("e",1_000_000_000);
        assert!(f.accept(&reply(&p,&host,1_009_000_000,1_012_000_000),1_009_000_000));let v=f.estimate(1_009_000_000);
        assert_eq!(v["host_minus_core_ns"],6_000_000);assert_eq!(v["network_rtt_ns"],6_000_000);assert_eq!(v["network_uncertainty_ns"],3_000_000);
        let p=f.probe("e",2_000_000_000);assert!(f.accept(&reply(&p,&host,2_008_000_000,2_009_000_000),2_003_000_000));
        let v=f.estimate(2_003_000_000);assert_eq!(v["host_minus_core_ns"],7_000_000);assert_eq!(v["network_rtt_ns"],2_000_000);assert_eq!(v["sample_count"],2);
        let mut f=Filter::default();let p=f.probe("e",1_000_000_000);assert!(f.accept(&reply(&p,&host,995_000_000,996_000_000),1_007_000_000));assert_eq!(f.estimate(1_007_000_000)["host_minus_core_ns"],-8_000_000);
    }
    #[test]
    fn stale_duplicate_invalid_and_restarted_host_samples_are_not_reused() {
        let mut f=Filter::default();let host=uuid::Uuid::new_v4().to_string();let p=f.probe("e",1000);let valid=reply(&p,&host,1100,1200);
        let mut wrong=valid.clone();wrong["probe_id"]=json!(uuid::Uuid::new_v4().to_string());assert!(!f.accept(&wrong,1300));assert!(f.accept(&valid,1300));assert!(!f.accept(&valid,1300));
        assert!(f.estimate(1301+MAX_AGE).is_null());
        for (t2,t3,received) in [(1200,1100,1300),(1100,1401,1300),(1100,u64::MAX,1300),(1100,1200,MAX_RTT+1001)] {
            let p=f.probe("e",1000);assert!(!f.accept(&reply(&p,&host,t2,t3),received));
        }
        let p=f.probe("e",2000);let other=uuid::Uuid::new_v4().to_string();assert!(f.accept(&reply(&p,&other,2100,2200),2300));assert_eq!(f.estimate(2300)["sample_count"],1);assert_eq!(f.estimate(2300)["host_clock_id"],other);
    }
}
