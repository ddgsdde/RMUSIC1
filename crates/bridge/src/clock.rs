//! Monotonic host-clock probe replies. This does not adjust audio or claim device synchronization.
use serde_json::{json,Value};
pub fn reply(probe:&Value,endpoint:&str,host:&str,received:u64,sent:u64)->Option<Value> {
    if probe["type"]!="clock_probe"||probe["version"]!=1||probe["endpoint_id"]!=endpoint||sent<received{return None;}
    let core=probe["core_clock_id"].as_str()?;let id=probe["probe_id"].as_str()?;let t1=probe["t1_ns"].as_u64()?;
    if uuid::Uuid::parse_str(core).is_err()||uuid::Uuid::parse_str(id).is_err()||t1>i64::MAX as u64||sent>i64::MAX as u64{return None;}
    Some(json!({"type":"clock_reply","version":1,"endpoint_id":endpoint,"core_clock_id":core,"host_clock_id":host,"probe_id":id,"t1_ns":t1,"t2_ns":received,"t3_ns":sent}))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_probe_replies_preserve_exchange_identity_and_reject_invalid_timestamps() {
        let probe=json!({"type":"clock_probe","version":1,"endpoint_id":"e","core_clock_id":uuid::Uuid::new_v4().to_string(),"probe_id":uuid::Uuid::new_v4().to_string(),"t1_ns":100});
        let v=reply(&probe,"e","host",200,220).unwrap();assert_eq!(v["probe_id"],probe["probe_id"]);assert_eq!((v["t2_ns"].as_u64(),v["t3_ns"].as_u64()),(Some(200),Some(220)));
        assert!(reply(&probe,"other","host",200,220).is_none());assert!(reply(&probe,"e","host",220,200).is_none());
        for (field,value) in [("version",json!(2)),("t1_ns",json!(u64::MAX)),("t1_ns",json!(-1)),("core_clock_id",json!("foreign"))] {
            let mut invalid=probe.clone();invalid[field]=value;assert!(reply(&invalid,"e","host",200,220).is_none());
        }
    }
}
