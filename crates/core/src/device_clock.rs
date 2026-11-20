//! Report driver counter/time associations separately from strict-sync admission.
use rmusic_audio::output::clock::{Domain,Sample,Source};
use serde_json::{json,Value};
const FRESH:u64=500_000_000;
#[derive(Default)]
pub(crate) struct DeviceFilter {key:Option<(String,u64,u64,Source,Domain,bool)>,points:Vec<(u64,u64)>,last:Value}
impl DeviceFilter {
    pub(crate) fn clear(&mut self){self.key=None;self.points.clear();self.last=Value::Null;}
    pub(crate) fn observe(&mut self,report:&Value,host:&Value,now:u64)->Value {
        let result=self.update(report,host,now);if result.is_null(){self.clear();}else{self.last=result.clone();}result
    }
    fn update(&mut self,report:&Value,host:&Value,now:u64)->Value {
        let Ok(s)=serde_json::from_value::<Sample>(report["device_clock"].clone()) else{return Value::Null;};
        let Some(host_id)=report["host_clock_id"].as_str() else{return Value::Null;};
        if host["host_clock_id"]!=host_id||host["valid_until_core_ns"].as_u64().is_none_or(|until|now>until)||s.version!=1||s.epoch==0||s.nominal_frequency_hz==0||s.host_read_before_ns>s.host_read_after_ns{return Value::Null;}
        let Some(sent)=report["monotonic_ns"].as_u64() else{return Value::Null;};
        if sent.checked_sub(s.host_read_after_ns).is_none_or(|age|age>FRESH)||s.host_read_after_ns-s.host_read_before_ns>FRESH{return Value::Null;}
        let Some(offset)=host["host_minus_core_ns"].as_i64() else{return Value::Null;};
        let convert=|time:u64|u64::try_from(time as i128-offset as i128).ok();
        let Some(read_before)=convert(s.host_read_before_ns) else{return Value::Null;};let Some(read_after)=convert(s.host_read_after_ns) else{return Value::Null;};
        if now.saturating_sub(read_after)>FRESH||read_after>now.saturating_add(host["network_uncertainty_ns"].as_u64().unwrap_or(0)).saturating_add(1_000_000){return Value::Null;}
        let known=matches!(s.timestamp_domain,Domain::Qpc) && report["host_clock_basis"]=="qpc"||matches!(s.timestamp_domain,Domain::Monotonic)&&report["host_clock_basis"]=="monotonic";
        let mapped=known&&s.device_time_ns>0&&s.running&&s.device_time_ns<=s.host_read_after_ns.saturating_add(100)&&s.host_read_after_ns.saturating_sub(s.device_time_ns)<=FRESH;
        let time=if mapped{s.device_time_ns}else{s.host_read_before_ns+(s.host_read_after_ns-s.host_read_before_ns)/2};
        let Some(core_time)=convert(time) else{return Value::Null;};
        let key=(host_id.to_owned(),s.epoch,s.nominal_frequency_hz,s.source,s.timestamp_domain,mapped);
        if self.key.as_ref()!=Some(&key){self.points.clear();self.key=Some(key);}
        if !s.running {self.points.clear();}
        if s.running {
            if let Some(&(last_time,last_counter))=self.points.last(){if time<last_time||s.counter<last_counter{return Value::Null;}if time==last_time&&s.counter!=last_counter{return Value::Null;}if mapped&&s.counter==last_counter&&time.saturating_sub(last_time)>100_000_000{return Value::Null;}}
            if !self.points.last().is_some_and(|&(t,_)|t==time){self.points.retain(|&(t,_)|time.saturating_sub(t)<=8_000_000_000);if self.points.len()==32{self.points.remove(0);}self.points.push((time,s.counter));}
        }
        let mut rate=Value::Null;
        if let Some(&(first_time,first_counter))=self.points.first(){let span=time-first_time;
            if self.points.len()>=4&&span>=1_000_000_000 {
                let n=self.points.len() as f64;let mean_x=self.points.iter().map(|&(t,_)|(t-first_time) as f64/1e9).sum::<f64>()/n;let mean_y=self.points.iter().map(|&(_,c)|(c-first_counter) as f64).sum::<f64>()/n;
                let variance=self.points.iter().map(|&(t,_)|((t-first_time) as f64/1e9-mean_x).powi(2)).sum::<f64>();
                let covariance=self.points.iter().map(|&(t,c)|((t-first_time) as f64/1e9-mean_x)*((c-first_counter) as f64-mean_y)).sum::<f64>();
                let observed=covariance/variance;let ppm=(observed/s.nominal_frequency_hz as f64-1.)*1e6;
                rate=json!({"observed_frequency_hz":observed,"observed_drift_ppm":ppm,"span_ns":span,"sample_count":self.points.len(),"query_time_approximation":!mapped,"rate_sane":ppm.abs()<=10_000.});
            }
        }
        json!({"version":1,"core_clock_id":host["core_clock_id"],"host_clock_id":host_id,"device_epoch":s.epoch,"source":s.source,"timestamp_domain":s.timestamp_domain,
            "counter":s.counter,"nominal_frequency_hz":s.nominal_frequency_hz,"core_sample_ns":if mapped{Some(core_time)}else{None},"core_read_before_ns":read_before,"core_read_after_ns":read_after,
            "host_network_uncertainty_ns":host["network_uncertainty_ns"],"driver_read_span_ns":s.host_read_after_ns-s.host_read_before_ns,"valid_until_core_ns":read_after.saturating_add(FRESH),"running":s.running,
            "device_clock_mapped":mapped,"driver_accuracy_verified":false,"rate_observation":rate,"programme_clock":programme(report,&s,mapped.then_some(core_time))})
    }
    pub(crate) fn latest(&self,now:u64,host:&Value)->Value {if self.last["host_clock_id"]==host["host_clock_id"]&&host["valid_until_core_ns"].as_u64().is_some_and(|until|now<=until)&&self.last["valid_until_core_ns"].as_u64().is_some_and(|until|now<=until){self.last.clone()}else{Value::Null}}
}
/// Bounded v2 programme coordinates at one actual output cursor; integer origins stay integer.
pub(crate) fn source_clock(value:&Value,output:u64)->Option<(u64,f64,f64)> {
    if value["version"]!=2||value["track_output_frames"].as_u64()?>output{return None;}
    let frame=value["source_frame"].as_u64()?;let fraction=value["source_fraction"].as_f64()?;let slope=value["source_per_output"].as_f64()?;
    if !fraction.is_finite()||!(0. ..1.).contains(&fraction)||!slope.is_finite()||!(0.9995-1e-12..=1.0005+1e-12).contains(&slope)
        ||((frame as i128-output as i128) as f64+fraction).abs()>output as f64*0.0005+2.{return None;}
    Some((frame,fraction,slope))
}
// This validates a paired authenticated diagnostic, not ownership or hardware precision.
fn programme(report:&Value,s:&Sample,core_time:Option<u64>)->Value {
    let Some(p)=s.programme else{return Value::Null;};let value=&report["programme_clock"];
    let Some(zone)=report["zone_id"].as_str().filter(|id|!id.is_empty()&&id.len()<=200)else{return Value::Null;};
    let Some(playback)=value["core_playback_id"].as_str().filter(|id|uuid::Uuid::parse_str(id).is_ok())else{return Value::Null;};
    let Some(track)=value["track_id"].as_str().filter(|id|!id.is_empty()&&id.len()<=200)else{return Value::Null;};
    let Some(position)=value["position"].as_f64().filter(|position|position.is_finite()&&*position>=0.&&*position<=86400.)else{return Value::Null;};
    if !s.running||!matches!(s.source,Source::WasapiTicks|Source::AlsaFrames|Source::AsioFrames|Source::AndroidFrames)||(8000..=768000).contains(&p.sample_rate)==false
        ||value["device_epoch"]!=s.epoch||value["device_counter"]!=s.counter||value["output_frame"]!=p.frames||value["sample_rate"]!=p.sample_rate{return Value::Null;}
    let mut paired=json!({"version":value["version"],"zone_id":zone,"core_playback_id":playback,"track_id":track,"position":position,"output_frame":p.frames,"sample_rate":p.sample_rate,
        "device_epoch":s.epoch,"device_counter":s.counter,"core_sample_ns":core_time,"driver_accuracy_verified":false});
    if value["version"]==1 {
        let Some(frames)=value["source_frames"].as_u64().filter(|frames|*frames<=p.frames)else{return Value::Null;};
        if position+0.000001<frames as f64/p.sample_rate as f64{return Value::Null;}paired["source_frames"]=json!(frames);
    }else {
        let Some((frame,fraction,slope))=source_clock(value,p.frames)else{return Value::Null;};let frames=value["track_output_frames"].as_u64().unwrap();
        if position+2./(p.sample_rate as f64)<frames as f64*0.9995/p.sample_rate as f64{return Value::Null;}
        paired["source_frame"]=json!(frame);paired["source_fraction"]=json!(fraction);paired["source_per_output"]=json!(slope);paired["track_output_frames"]=json!(frames);
    }paired
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bridge_clock_frozen_running_counter_is_not_projected_as_playing(){
        let mut f=DeviceFilter::default();assert!(!f.observe(&report(1_000_000_000,48000,1),&host(),1_000_001_000).is_null());
        assert!(f.observe(&report(1_200_000_000,48000,1),&host(),1_200_001_000).is_null());
        assert!(f.latest(1_200_001_001,&host()).is_null());
    }
    const BASE:u64=100_000_000_000_000_000;
    fn host()->Value{json!({"core_clock_id":"core","host_clock_id":"host","host_minus_core_ns":BASE,"valid_until_core_ns":20_000_000_000u64,"network_uncertainty_ns":1000})}
    fn report(time:u64,counter:u64,epoch:u64)->Value {
        json!({"host_clock_id":"host","host_clock_basis":"qpc","monotonic_ns":BASE+time+1000,"device_clock":{"version":1,"epoch":epoch,"source":"wasapi_ticks","timestamp_domain":"qpc","counter":counter,"nominal_frequency_hz":48000,"device_time_ns":BASE+time,"host_read_before_ns":BASE+time-50,"host_read_after_ns":BASE+time+50,"running":true}})
    }
    #[test]
    fn device_mapping_and_rate_observation_preserve_large_integer_counter_differences() {
        let mut f=DeviceFilter::default();let start=1u64<<60;let mut value=Value::Null;
        for second in 1..=5 {let time=second*1_000_000_000;value=f.observe(&report(time,start+second*48012,1),&host(),time+1000);}
        assert_eq!(value["core_sample_ns"],5_000_000_000u64);assert_eq!(value["device_clock_mapped"],true);assert_eq!(value["driver_accuracy_verified"],false);
        assert!((value["rate_observation"]["observed_drift_ppm"].as_f64().unwrap()-250.).abs()<1e-5);
        assert!(f.latest(5_500_000_051,&host()).is_null());let mut replaced=host();replaced["host_clock_id"]=json!("new-host");assert!(f.latest(5_000_001_000,&replaced).is_null());
    }
    #[test]
    fn opaque_pause_reset_replay_and_foreign_host_do_not_claim_a_device_mapping() {
        let mut f=DeviceFilter::default();let mut r=report(1_000_000_000,48000,1);r["device_clock"]["timestamp_domain"]=json!("driver_opaque");let v=f.observe(&r,&host(),1_000_001_000);assert_eq!(v["device_clock_mapped"],false);assert!(v["core_sample_ns"].is_null());
        let r=report(2_000_000_000,96000,2);assert_eq!(f.observe(&r,&host(),2_000_001_000)["device_epoch"],2);
        assert!(f.observe(&report(3_000_000_000,1,2),&host(),3_000_001_000).is_null());
        let mut paused=report(4_000_000_000,1,3);paused["device_clock"]["running"]=json!(false);let v=f.observe(&paused,&host(),4_000_001_000);assert_eq!(v["device_clock_mapped"],false);assert!(v["rate_observation"].is_null());
        let mut foreign=report(5_000_000_000,1,4);foreign["host_clock_id"]=json!("old-host");assert!(f.observe(&foreign,&host(),5_000_001_000).is_null());
    }
}

#[cfg(test)]
#[test]
fn source_clock_v2_keeps_integer_fraction_and_slope_bound_to_the_same_device_sample(){
    let host=json!({"core_clock_id":"core","host_clock_id":"host","host_minus_core_ns":0,"valid_until_core_ns":20_000_000_000u64,"network_uncertainty_ns":1000});let frame=1u64<<60;
    let report=json!({"zone_id":"zone","host_clock_id":"host","host_clock_basis":"qpc","monotonic_ns":1_000_000_001u64,"device_clock":{"version":1,"epoch":9,"source":"wasapi_ticks","timestamp_domain":"qpc","counter":frame+9,"nominal_frequency_hz":10_000_000,"device_time_ns":1_000_000_000u64,"host_read_before_ns":1_000_000_000u64,"host_read_after_ns":1_000_000_001u64,"running":true,"programme":{"frames":frame,"sample_rate":768000}},"programme_clock":{"version":2,"device_epoch":9,"device_counter":frame+9,"output_frame":frame,"sample_rate":768000,"core_playback_id":uuid::Uuid::new_v4().to_string(),"track_id":"track","position":0.01,"source_frame":frame+1,"source_fraction":0.25,"source_per_output":1.0004,"track_output_frames":7680}});
    let mut filter=DeviceFilter::default();let value=filter.observe(&report,&host,1_000_001_000);assert_eq!(value["programme_clock"]["source_frame"],frame+1);assert_eq!(value["programme_clock"]["source_fraction"],0.25);assert_eq!(value["programme_clock"]["output_frame"],frame);assert!(value["programme_clock"]["source_frames"].is_null());
    for(field,bad)in [("source_frame",json!(frame+frame/100)),("source_fraction",json!(1.)),("source_fraction",json!(-0.1)),("source_per_output",json!(1.001)),("source_per_output",Value::Null),("track_output_frames",json!(frame+1)),("device_counter",json!(frame+10)),("version",json!(3))]{let mut invalid=report.clone();invalid["programme_clock"][field]=bad;assert!(filter.observe(&invalid,&host,1_000_001_000)["programme_clock"].is_null(),"{field}");}
    let mut legacy=report;legacy["programme_clock"]["version"]=json!(1);legacy["programme_clock"]["source_frames"]=json!(7680);let value=filter.observe(&legacy,&host,1_000_001_000);assert_eq!(value["programme_clock"]["version"],1);assert!(value["programme_clock"]["source_frame"].is_null());
}
#[cfg(test)]
#[test]
fn programme_clock_validation_binds_exact_integer_pair_and_never_reuses_an_invalid_duplicate() {
    let clock=json!({"core_clock_id":"core","host_clock_id":"host","host_minus_core_ns":0,"valid_until_core_ns":20_000_000_000u64,"network_uncertainty_ns":1000});let frame=1u64<<60;let playback=uuid::Uuid::new_v4().to_string();
    let report=json!({"zone_id":"zone","host_clock_id":"host","host_clock_basis":"qpc","monotonic_ns":1_000_000_001u64,"device_clock":{"version":1,"epoch":9,"source":"wasapi_ticks","timestamp_domain":"qpc","counter":frame+1,"nominal_frequency_hz":10_000_000,"device_time_ns":1_000_000_000u64,"host_read_before_ns":1_000_000_000u64,"host_read_after_ns":1_000_000_001u64,"running":true,"programme":{"frames":frame,"sample_rate":768000}},"programme_clock":{"version":1,"device_epoch":9,"device_counter":frame+1,"output_frame":frame,"sample_rate":768000,"core_playback_id":playback,"track_id":"track","position":0.01,"source_frames":7680}});
    let mut filter=DeviceFilter::default();let value=filter.observe(&report,&clock,1_000_001_000);assert_eq!(value["programme_clock"]["output_frame"],frame);assert_eq!(value["programme_clock"]["core_sample_ns"],1_000_000_000u64);assert_eq!(value["programme_clock"]["driver_accuracy_verified"],false);
    for (field,bad) in [("device_epoch",json!(10)),("device_counter",json!(frame+2)),("output_frame",json!(frame+1)),("sample_rate",json!(48000)),("core_playback_id",json!("old")),("source_frames",json!(frame+1)),("position",Value::Null)] {
        let mut altered=report.clone();altered["programme_clock"][field]=bad;let value=filter.observe(&altered,&clock,1_000_001_000);assert!(!value.is_null());assert!(value["programme_clock"].is_null(),"{field}");
    }
    let mut opaque=report.clone();opaque["device_clock"]["timestamp_domain"]=json!("driver_opaque");let value=filter.observe(&opaque,&clock,1_000_001_000);assert!(!value["programme_clock"].is_null());assert!(value["programme_clock"]["core_sample_ns"].is_null());assert_eq!(value["device_clock_mapped"],false);
    let mut legacy=report.clone();legacy["device_clock"].as_object_mut().unwrap().remove("programme");assert!(filter.observe(&legacy,&clock,1_000_001_000)["programme_clock"].is_null());
    let mut android=report;android["host_clock_basis"]=json!("monotonic");android["device_clock"]["source"]=json!("android_frames");android["device_clock"]["timestamp_domain"]=json!("monotonic");
    let value=filter.observe(&android,&clock,1_000_001_000);assert_eq!(value["device_clock_mapped"],true);assert_eq!(value["programme_clock"]["output_frame"],frame);assert_eq!(value["driver_accuracy_verified"],false);
}
