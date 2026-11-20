import {useEffect,useState,type FormEvent} from "react";
import type {Endpoint,RMusicClient,SyncStart,Zone} from "@r-music/sdk";
import {Modal} from "./components";
import {groupRateReason} from "./groupRate";

type Props={api:RMusicClient;job:SyncStart;zones:Zone[];locked:boolean;onClose:()=>void;onApply:(from:string[],ids:string[])=>boolean};
export function GroupMembers({api,job,zones,locked,onClose,onApply}:Props) {
 const [from]=useState(()=>job.members.map(m=>m.zone_id).sort()),[ids,setIds]=useState(from);
 const [outputs,setOutputs]=useState<Endpoint[]>([]),[loading,setLoading]=useState(true),[notice,setNotice]=useState("");
 useEffect(()=>{
  let live=true;const controller=new AbortController(),timeout=setTimeout(()=>controller.abort(),10000);
  void api.request<{items:Endpoint[]}>("/endpoints",{signal:controller.signal}).then(value=>{if(live)setOutputs(value.items);}).catch(()=>{if(live)setNotice("无法读取输出，请关闭后重试。");}).finally(()=>{clearTimeout(timeout);if(live)setLoading(false);});
  return()=>{live=false;clearTimeout(timeout);controller.abort();};
 },[api]);
 const stale=JSON.stringify(from)!==JSON.stringify(job.members.map(m=>m.zone_id).sort());
 const ended=!["preparing","committed","playing","pausing","paused","seeking"].includes(job.phase);
 function reason(zone:Zone):string {
  const member=from.includes(zone.id),endpoint=outputs.find(e=>e.id===zone.endpoint_id);
  if(!member&&(zone.status!=="stopped"||zone.playback_id))return "请先停止此音区";
  if(!member&&job.members.some(m=>m.endpoint_id===zone.endpoint_id))return "此输出属于当前组的其他音区";
  if(!endpoint?.online)return "输出离线";
  if(endpoint.capabilities.scheduled_start_v1!==true||endpoint.capabilities.continuous_pcm!==true)return "此输出暂不支持分组播放";
  if(job.queue&&endpoint.capabilities.scheduled_queue_v1!==true)return "此输出暂不支持连续组队列";
  if(job.measured_version===2&&(endpoint.capabilities.group_output_v2!==true||endpoint.capabilities.measured_listens_v2!==true||endpoint.capabilities.scheduled_queue_v1!==true))return "此输出暂不支持新版分组计时";
  if(job.rate_adjustment?.enabled){const why=groupRateReason(endpoint,job.rate_adjustment.automatic);if(why)return why;}
  if(!endpoint.clock_calibration)return "输出时钟正在准备";
  if(ids.some(id=>id!==zone.id&&zones.find(z=>z.id===id)?.endpoint_id===zone.endpoint_id))return "同一输出只能选择一个音区";
  return "";
 }
 const valid=!loading&&!notice&&!stale&&!ended&&!locked&&ids.length>=1&&ids.length<=4&&JSON.stringify([...ids].sort())!==JSON.stringify(from)&&ids.every(id=>{const zone=zones.find(z=>z.id===id);return zone&&!reason(zone);});
 function submit(event:FormEvent){event.preventDefault();if(valid&&onApply(from,[...ids].sort()))onClose();}
 return <Modal title="调整分组成员" onClose={onClose} wide><form className="group-form" onSubmit={submit}>
  <p className="fine-print">加入或移除音区时，整组会短暂重新缓冲，再从共同确认的位置继续。暂停中的组保持暂停，听音次数仍按整组统计。</p>
  {notice&&<p role="alert">{notice}</p>}{stale&&<p role="alert">分组成员已变化，请关闭后重新选择。</p>}{ended&&<p role="alert">分组已结束，请关闭此窗口。</p>}
  {loading?<p className="muted">正在读取输出…</p>:<fieldset disabled={locked||stale||ended}><legend>选择 1–4 个音区</legend>{zones.map(zone=>{const why=reason(zone),checked=ids.includes(zone.id);return <label className="group-room" key={zone.id}><input type="checkbox" checked={checked} disabled={!checked&&(!!why||ids.length>=4)} onChange={event=>setIds(old=>event.target.checked?[...old,zone.id]:old.filter(id=>id!==zone.id))}/><span>{zone.name}<small>{why||outputs.find(e=>e.id===zone.endpoint_id)?.name}</small></span></label>;})}</fieldset>}
  <p className="fine-print">只留一个音区时仍可用整组控制；停止全部输出请撤销整组。</p>
  <button type="submit" disabled={!valid}>应用成员调整</button>
 </form></Modal>;
}
