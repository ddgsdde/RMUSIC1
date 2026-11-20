//! Read-only programme-phase estimates; neither driver accuracy nor acoustic sync is asserted.
use super::*;
use rmusic_audio::output::clock::{Domain,Source};

#[derive(Clone,PartialEq)]
pub(super) struct Identity {epoch:u64,frequency:u64,rate:u32,source:Source,domain:Domain,programme_version:u32}
#[derive(Clone,Default)]
pub(super) enum Binding {#[default] Unseen,Seen(Identity),Changed}
struct Measurement {identity:Identity,frames:u64,fraction:f64,slope:f64,time:u64,ratio:Option<f64>,valid_until:u64,network:u64,read_span:u64}

fn measure(job:&Job,i:usize,endpoint:&Value,host:&Value,now:u64)->Result<Measurement,&'static str> {
    let m=&job.members[i];let v=&endpoint["device_clock_calibration"];let a=&v["programme_clock"];
    if !job.dispatched||job.phase!="playing"||!valid_gate(m,&json!({"gate":m.gate}),"released"){return Err("等待当前输出起播确认");}
    if host["valid_until_core_ns"].as_u64().is_none_or(|n|n<now)||host["core_clock_id"]!=m.token["core_clock_id"]||host["host_clock_id"]!=m.token["host_clock_id"] {return Err("宿主时钟校准已失效");}
    if v.is_null()||a.is_null(){return Err("等待节目与设备时钟反馈");}
    let until=v["valid_until_core_ns"].as_u64().filter(|n|*n>=now).ok_or("设备反馈已过期")?;
    if v["core_clock_id"]!=host["core_clock_id"]||v["host_clock_id"]!=host["host_clock_id"]||v["running"]!=true||v["device_clock_mapped"]!=true {return Err("设备时钟尚不能映射到共同时间");}
    let time=v["core_sample_ns"].as_u64().filter(|t|*t<=now&&now-*t<=500_000_000).ok_or("设备采样时刻不可用于比较")?;
    let rate=a["sample_rate"].as_u64().and_then(|r|u32::try_from(r).ok()).filter(|r|(8000..=768000).contains(r)&&*r==m.rate).ok_or("采样率与当前输出不匹配")?;
    let epoch=v["device_epoch"].as_u64().filter(|n|*n>0).ok_or("设备输出代数无效")?;
    let frequency=v["nominal_frequency_hz"].as_u64().filter(|n|*n>0).ok_or("设备计数单位无效")?;
    let source=serde_json::from_value::<Source>(v["source"].clone()).map_err(|_|"设备计数来源无效")?;
    let domain=serde_json::from_value::<Domain>(v["timestamp_domain"].clone()).map_err(|_|"设备时间域无效")?;
    if !matches!(source,Source::WasapiTicks|Source::AlsaFrames|Source::AsioFrames|Source::AndroidFrames)||domain==Domain::DriverOpaque {return Err("当前设备时间域不可比较");}
    if a["zone_id"]!=m.zone.id||a["device_epoch"]!=epoch||a["device_counter"]!=v["counter"]||a["core_sample_ns"]!=v["core_sample_ns"] {return Err("节目反馈与当前设备采样不匹配");}
    let playback=a["core_playback_id"].as_str().ok_or("节目实例缺失")?;
    let track=if let Some(q)=&job.queue {
        let index=q.known[i].get(playback).filter(|n|**n>=q.segment).ok_or("节目不属于当前输出实例")?;
        m.zone.queue.get(*index).ok_or("节目曲序已失效")?
    }else {
        if playback!=m.playback_id{return Err("节目不属于当前输出实例");}
        m.zone.track.as_ref().ok_or("节目曲目已失效")?
    };
    if a["track_id"]!=track.id{return Err("节目曲目与当前分组不匹配");}
    let output=a["output_frame"].as_u64().ok_or("输出帧缺失")?;
    let (frames,fraction,slope,programme_version)=if a["version"]==1{(output,0.,1.,1)}else {
        if job.request.measured_version!=2{return Err("变速节目反馈须使用分组双时间协议");}
        let(frame,fraction,slope)=crate::device_clock::source_clock(a,output).ok_or("变速节目反馈无效")?;(frame,fraction,slope,2)
    };
    let network=v["host_network_uncertainty_ns"].as_u64().filter(|n|*n<=250_000_000).ok_or("宿主时间不确定度无效")?;
    let deadline=job.deadline.ok_or("共同起播时刻缺失")?;
    if time.saturating_add(network).saturating_add(1_000_000)<deadline||output as u128*1_000_000_000>(now.saturating_sub(deadline) as u128+1_000_000_000)*rate as u128 {return Err("节目反馈超出当前起播范围");}
    let read_span=v["driver_read_span_ns"].as_u64().filter(|n|*n<=500_000_000).ok_or("驱动读取区间无效")?;
    let observation=&v["rate_observation"];
    if observation["rate_sane"]==false{return Err("设备速率反馈不适于比较");}
    let ratio=observation["observed_frequency_hz"].as_f64().filter(|f|f.is_finite()&&*f>0.).map(|f|f/frequency as f64).filter(|r|(0.99..=1.01).contains(r))
        .filter(|_|observation["rate_sane"]==true&&observation["query_time_approximation"]==false&&observation["sample_count"].as_u64().is_some_and(|n|n>=4)&&observation["span_ns"].as_u64().is_some_and(|n|(1_000_000_000..=8_000_000_000).contains(&n)));
    Ok(Measurement{identity:Identity{epoch,frequency,rate,source,domain,programme_version},frames,fraction,slope,time,ratio,valid_until:until,network,read_span})
}

/// Called after the endpoint's authenticated calibration was updated; no database/audio mutation.
pub(super) async fn observe(s:&AppState,id:&str,tx:&mpsc::Sender<Value>) {
    let endpoint=s.endpoints.read().await.get(id).cloned().unwrap_or(Value::Null);let now=crate::clock::now_ns();let host=s.host_clock_estimate(id,tx,now).await;
    let mut registry=s.sync_starts.lock().await;
    for job in registry.jobs.values_mut().filter(|j|j.phase=="playing") {
        for i in 0..job.members.len() {
            if job.members[i].zone.endpoint_id!=id||!job.members[i].tx.same_channel(tx){continue;}
            if let Ok(value)=measure(job,i,&endpoint,&host,now) {
                let binding=&mut job.members[i].clock;
                match binding {Binding::Unseen=>*binding=Binding::Seen(value.identity),Binding::Seen(old) if *old!=value.identity=>*binding=Binding::Changed,_=>{}}
            }
        }
    }
}

fn phase_ns(value:&Measurement,reference:&Measurement,now:u64)->i64 {
    // Subtract integer programme times before any float conversion; raw device counters have arbitrary origins.
    let a=value.frames as u128*1_000_000_000/value.identity.rate as u128;
    let b=reference.frames as u128*1_000_000_000/reference.identity.rate as u128;
    let base=a as i128-b as i128+reference.time as i128-value.time as i128;
    let fraction=value.fraction*1e9/value.identity.rate as f64-reference.fraction*1e9/reference.identity.rate as f64;
    let drift=(now-value.time) as f64*(value.ratio.unwrap_or(1.)*value.slope-1.)-(now-reference.time) as f64*(reference.ratio.unwrap_or(1.)*reference.slope-1.);
    (base as f64+fraction+drift).round().clamp(i64::MIN as f64,i64::MAX as f64) as i64
}
fn processing_bound(value:&Measurement,now:u64)->Option<u64> {
    if value.identity.programme_version==1{return Some(0);}value.ratio?;
    Some(((now-value.time) as f64*1.01*(1.0005-value.slope).abs().max((0.9995-value.slope).abs())).ceil() as u64)
}
fn response(now:u64,state:&str,reference:Option<&str>,members:Vec<Value>)->Value {
    let valid_for=members.iter().filter(|m|m["available"]==true).filter_map(|m|m["valid_until_core_ns"].as_u64()).min().unwrap_or(now).saturating_sub(now).min(500_000_000)/1_000_000;
    json!({"version":1,"state":state,"evaluated_at_core_ns":now,"valid_for_ms":valid_for,"reference_zone_id":reference,"members":members,"total_uncertainty_ns":null,"driver_accuracy_verified":false,"strict_sync":false,"correction":"off"})
}
pub(super) fn inactive()->Value {response(crate::clock::now_ns(),"inactive",None,Vec::new())}
pub(super) async fn snapshot(job:&Job,s:&AppState)->Value {
    let now=crate::clock::now_ns();if job.phase!="playing"{return response(now,"inactive",None,Vec::new());}
    let endpoints=s.endpoints.read().await.clone();let bridges=s.bridges.read().await.clone();let mut rows=Vec::new();
    for (i,m) in job.members.iter().enumerate() {
        let host=s.host_clock_estimate(&m.zone.endpoint_id,&m.tx,now).await;
        let value=if !bridges.get(&m.zone.endpoint_id).is_some_and(|tx|tx.same_channel(&m.tx)){Err("当前输出连接已变化")}else{
            measure(job,i,endpoints.get(&m.zone.endpoint_id).unwrap_or(&Value::Null),&host,now).and_then(|value|match &m.clock {
                Binding::Seen(identity) if *identity==value.identity=>Ok(value),Binding::Unseen=>Err("等待关联当前输出"),_=>Err("设备输出已重置，请重新起播")
            })
        };rows.push(value);
    }
    let reference=rows.iter().position(Result::is_ok);let count=rows.iter().filter(|r|r.is_ok()).count();let state=if job.members.len()<2{"single_output"}else if count==job.members.len(){"available"}else if count>0{"partial"}else{"unavailable"};
    let reference=reference.filter(|_|job.members.len()>=2);let mut members=Vec::new();
    for (i,row) in rows.iter().enumerate() {
        let m=&job.members[i];let mut entry=json!({"zone_id":m.zone.id,"endpoint_id":m.zone.endpoint_id,"available":row.is_ok(),"reason":row.as_ref().err(),"estimated_offset_ns":null,"relative_drift_ppm":null,"projection":null});
        if let Ok(value)=row {
            entry["device_epoch"]=json!(value.identity.epoch);entry["programme_clock_version"]=json!(value.identity.programme_version);entry["source_per_output"]=json!(value.slope);entry["valid_until_core_ns"]=json!(value.valid_until);entry["sample_age_ns"]=json!(now-value.time);entry["driver_read_span_ns"]=json!(value.read_span);entry["projection"]=json!(if value.ratio.is_some(){"observed"}else{"nominal"});
            if let Some(r)=reference {let reference=rows[r].as_ref().unwrap();entry["estimated_offset_ns"]=json!(phase_ns(value,reference,now));entry["known_host_uncertainty_ns"]=json!(if i==r{0}else{value.network+reference.network});entry["relative_drift_ppm"]=json!(value.ratio.zip(reference.ratio).map(|(a,b)|(a/b-1.)*1e6));entry["relative_programme_drift_ppm"]=json!(value.ratio.zip(reference.ratio).map(|(a,b)|(a*value.slope/(b*reference.slope)-1.)*1e6));entry["processing_projection_uncertainty_ns"]=json!(if i==r{Some(0)}else{processing_bound(value,now).zip(processing_bound(reference,now)).map(|(a,b)|a.saturating_add(b))});}
        }members.push(entry);
    }
    let mut value=response(now,state,reference.map(|r|job.members[r].zone.id.as_str()),members);if job.request.automatic_rate_adjustment{value["correction"]=json!("experimental");}value
}

#[cfg(test)]
#[test]
fn group_clock_integer_phase_projection_keeps_one_frame_next_to_large_totals() {
    let identity=Identity{epoch:1,frequency:768000,rate:768000,source:Source::AlsaFrames,domain:Domain::Monotonic,programme_version:1};let frames=1u64<<60;
    let a=Measurement{identity:identity.clone(),frames:frames+1,fraction:0.,slope:1.,time:1_000_000_000,ratio:None,valid_until:2_000_000_000,network:0,read_span:0};let b=Measurement{identity,frames,fraction:0.,slope:1.,time:a.time,ratio:None,valid_until:a.valid_until,network:0,read_span:0};
    assert!((1302..=1303).contains(&phase_ns(&a,&b,1_500_000_000)));
}
#[cfg(test)]
#[test]
fn group_clock_v2_projects_programme_fraction_and_conversion_slope_separately_from_device_rate(){
    let identity=Identity{epoch:1,frequency:768000,rate:768000,source:Source::AlsaFrames,domain:Domain::Monotonic,programme_version:2};let frames=1u64<<60;
    let a=Measurement{identity:identity.clone(),frames:frames+1,fraction:0.25,slope:1.0005,time:1_000_000_000,ratio:Some(1.),valid_until:2_000_000_000,network:0,read_span:0};let b=Measurement{identity,frames,fraction:0.,slope:1.,time:a.time,ratio:a.ratio,valid_until:a.valid_until,network:0,read_span:0};
    assert!((1627..=1629).contains(&phase_ns(&a,&b,a.time)));assert!((251627..=251629).contains(&phase_ns(&a,&b,1_500_000_000)));assert!((505000..=505001).contains(&processing_bound(&a,1_500_000_000).unwrap()));assert_eq!(phase_ns(&a,&a,1_500_000_000),0);let unknown=Measurement{ratio:None,..a};assert!(processing_bound(&unknown,1_500_000_000).is_none());
}
