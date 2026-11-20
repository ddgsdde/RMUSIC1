//! Opt-in experimental programme-clock control. Hardware error remains unverified.
use super::*;
const REFRESH_NS:u64=500_000_000;
const LEASE_NS:u64=1_500_000_000;
const MAX_UNCERTAINTY:u64=25_000_000;
#[derive(Clone)]
pub(super) struct Controller {state:&'static str,reason:Option<&'static str>,last_send:u64,samples:Vec<u64>,good:u8,bad:u8,realignments:u32,last_realign:u64,pub(super) buffer_recoveries:u32,pub(super) last_buffer_recovery:u64,pub(super) buffer_recovering:bool}
impl Default for Controller {fn default()->Self{Self{state:"warming",reason:None,last_send:0,samples:Vec::new(),good:0,bad:0,realignments:0,last_realign:0,buffer_recoveries:0,last_buffer_recovery:0,buffer_recovering:false}}}
impl Controller {
    pub(super) fn value(&self)->Value {json!({"state":self.state,"reason":self.reason,"lease_ms":LEASE_NS/1_000_000,"max_ppm":500,"max_slew_ppm_per_second":20,"driver_accuracy_verified":false,"known_uncertainty_limit_ns":MAX_UNCERTAINTY,"realignments":self.realignments})}
    pub(super) fn rearmed(&self)->Self {Self{realignments:self.realignments,last_realign:self.last_realign,buffer_recoveries:self.buffer_recoveries,last_buffer_recovery:self.last_buffer_recovery,buffer_recovering:self.buffer_recovering,..Default::default()}}
}
fn uncertainty(row:&Value,reference_span:u64,rate:u32)->Result<u64,&'static str> {
    row["known_host_uncertainty_ns"].as_u64().zip(row["processing_projection_uncertainty_ns"].as_u64()).zip(row["driver_read_span_ns"].as_u64())
        .map(|((host,processing),span)|host.saturating_add(processing).saturating_add(span).saturating_add(reference_span).saturating_add(2_000_000_000/rate as u64))
        .filter(|n|*n<=MAX_UNCERTAINTY).ok_or("已知时钟不确定度过大")
}
fn desired(row:&Value,reference_span:u64,rate:u32)->Result<f64,&'static str> {
    if row["available"]!=true||row["programme_clock_version"]!=2||row["projection"]!="observed"{return Err("等待有效 v2 节目及设备速率反馈");}
    let ppm=row["relative_drift_ppm"].as_f64().filter(|n|n.is_finite()).ok_or("设备相对速率缺失")?;
    let phase=row["estimated_offset_ns"].as_i64().ok_or("节目相位缺失")? as f64;
    let uncertainty=uncertainty(row,reference_span,rate)? as f64;
    // Positive phase means this programme is ahead. Ignore the known uncertainty band.
    let error=phase.signum()*(phase.abs()-uncertainty-50_000.).max(0.);
    let ratio=1.+ppm/1e6;
    Ok(((1./ratio-1.)*1e6-error/20_000.).clamp(-500.,500.))
}
fn slew(previous:f64,target:f64,elapsed:u64)->f64 {let step=20.*elapsed.min(1_000_000_000) as f64/1e9;previous+(target-previous).clamp(-step,step)}
pub(super) async fn tick(job:&mut Job,s:&AppState)->anyhow::Result<()> {
    if !job.request.automatic_rate_adjustment||job.phase!="playing"{return Ok(());}
    let now=crate::clock::now_ns();if job.drift.last_send>0&&now.saturating_sub(job.drift.last_send)<REFRESH_NS{return Ok(());}
    let diagnostics=clock_feedback::snapshot(job,s).await;let rows=diagnostics["members"].as_array().unwrap();let reference=job.members.first().unwrap();
    let result=(||->Result<Vec<f64>,&'static str>{
        if job.members.len()<2||diagnostics["reference_zone_id"]!=reference.zone.id||rows.len()!=job.members.len(){return Err("固定参考输出不可用");}
        let span=rows[0]["driver_read_span_ns"].as_u64().ok_or("参考驱动读取区间缺失")?;
        rows.iter().zip(&job.members).map(|(row,member)|{if member.phase!="playing"{return Err("等待全部输出正在播放");}desired(row,span,member.rate)}).collect()
    })();
    let old=job.drift.state;
    let targets=match result {
        Ok(targets)=>{
            let at=diagnostics["evaluated_at_core_ns"].as_u64().unwrap();let stamps=rows.iter().map(|row|at-row["sample_age_ns"].as_u64().unwrap()).collect::<Vec<_>>();
            let fresh=job.drift.samples.len()!=stamps.len()||stamps.iter().zip(&job.drift.samples).all(|(a,b)|a>b);
            if fresh {
                job.drift.good=job.drift.good.saturating_add(1);job.drift.samples=stamps;
                let large=rows.iter().zip(&job.members).skip(1).any(|(row,m)|row["estimated_offset_ns"].as_i64().zip(uncertainty(row,rows[0]["driver_read_span_ns"].as_u64().unwrap_or(0),m.rate).ok())
                    .is_some_and(|(phase,noise)|phase.unsigned_abs()>noise.saturating_add(20_000_000)));
                job.drift.bad=if large{job.drift.bad.saturating_add(1)}else{0};
                if job.drift.bad>=3&&(job.drift.last_realign==0||now.saturating_sub(job.drift.last_realign)>=30_000_000_000) {
                    if job.drift.realignments>=3{return job.abort(s,"同步偏差持续过大，请检查输出路由、缓冲或网络后重新起播",false).await;}
                    job.drift.realignments+=1;job.drift.last_realign=now;
                    return control::realign(job,s).await;
                }
            }
            job.drift.reason=None;
            if job.drift.good<3{job.drift.state="warming";vec![0.;job.members.len()]}else{
                job.drift.state="tracking";let elapsed=if job.drift.last_send==0{REFRESH_NS}else{now.saturating_sub(job.drift.last_send)};
                targets.iter().enumerate().map(|(i,target)|if i==0{0.}else if fresh{slew(job.members[i].requested_rate_ppm,*target,elapsed)}else{job.members[i].requested_rate_ppm}).collect()
            }
        },
        Err(reason)=>{job.drift.state="held";job.drift.reason=Some(reason);job.drift.good=0;job.drift.bad=0;job.drift.samples.clear();vec![0.;job.members.len()]}
    };
    if job.drift.state!="tracking"&&job.members.iter().all(|m|m.requested_rate_ppm==0.){if old!=job.drift.state{s.emit("sync_start",job.value());}return Ok(());}
    let bridges=s.bridges.read().await.clone();let mut commands=Vec::new();
    for (i,member) in job.members.iter().enumerate().filter(|(_,m)|m.phase=="playing") {
        if !bridges.get(&member.zone.endpoint_id).is_some_and(|tx|tx.same_channel(&member.tx))||!valid_gate(member,&json!({"gate":member.gate}),"released"){return Ok(());}
        let estimate=s.host_clock_estimate(&member.zone.endpoint_id,&member.tx,now).await;
        let expires=match host_time(&estimate,now.saturating_add(LEASE_NS)){Ok(value)=>value,Err(_)=>return Ok(())};
        let permit=match member.tx.clone().try_reserve_owned(){Ok(permit)=>permit,Err(_)=>return Ok(())};
        commands.push((i,permit,json!({"type":"group_rate","zone_id":member.zone.id,"playback_id":member.playback_id,"start":member.token,"group_queue":true,"measured_version":2,"ppm":targets[i],"rate_expires_host_ns":expires})));
    }
    let changed=old!=job.drift.state||job.members.iter().zip(&targets).any(|(m,target)|m.requested_rate_ppm!=*target);
    for(i,permit,command)in commands{job.members[i].requested_rate_ppm=targets[i];permit.send(command);}job.drift.last_send=now;
    if changed{s.emit("sync_start",job.value());}Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drift_controller_bounds_slew_sign_uncertainty_and_closed_loop_programme_phase(){
        let row=|phase:i64,ppm:f64|json!({"available":true,"programme_clock_version":2,"projection":"observed","relative_drift_ppm":ppm,"estimated_offset_ns":phase,"known_host_uncertainty_ns":2000,"processing_projection_uncertainty_ns":1000,"driver_read_span_ns":100});
        assert!(desired(&row(1_000_000,100.),100,768000).unwrap()<0.);assert!(desired(&row(-1_000_000,-100.),100,768000).unwrap()>0.);assert_eq!(slew(0.,500.,500_000_000),10.);
        let mut noisy=row(0,100.);noisy["known_host_uncertainty_ns"]=json!(2_000_000);assert!(desired(&noisy,100,768000).unwrap()<0.);
        let mut bad=row(0,0.);bad["known_host_uncertainty_ns"]=json!(MAX_UNCERTAINTY+1);assert!(desired(&bad,100,768000).is_err());
        for ppm in [-200.,200.] {let(mut phase,mut target)=(0f64,0f64);for _ in 0..1200{target=slew(target,desired(&row(phase.round() as i64,ppm),100,768000).unwrap(),500_000_000);assert!(target.abs()<=500.);phase+=((1.+ppm/1e6)*(1.+target/1e6)-1.)*500_000_000.;}assert!(phase.abs()<100_000.,"{ppm}: {phase}");}
    }
}
