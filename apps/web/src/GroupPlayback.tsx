import {CatalogLink} from "./CatalogLinks";
import {useEffect,useRef,useState,type FormEvent} from "react";
import {GroupMembers} from "./GroupMembers";
import {GroupClockDiagnostics} from "./GroupClockDiagnostics";
import {GroupRecovery} from "./GroupRecovery";
import {ApiError,type Endpoint,type RMusicClient,type SyncControlRequest,type SyncStart,type SyncStartRequest,type Track,type Zone} from "@r-music/sdk";
import {Cover,Modal,quality} from "./components";
import {clientId} from "./clientId";
import {SavedOutputGroups} from "./SavedOutputGroups";
import {groupRateReason} from "./groupRate";

type Control={request:SyncControlRequest;confirmed?:boolean;rejected?:boolean;error?:string};
type Saved={request:SyncStartRequest;title:string;rejected?:boolean;rejection?:string;external?:boolean;control?:Control};
type Props={preset?:{ids:string[];key:number};api:RMusicClient;coreId:string;userId:string;supported:boolean;listSupported?:boolean;queueSupported?:boolean;outputTimingSupported?:boolean;automaticRateSupported?:boolean;syncPolicySupported?:boolean;transportSupported?:boolean;resyncSupported?:boolean;queueJumpSupported?:boolean;membershipSupported?:boolean;clockDiagnosticsSupported?:boolean;recoverySupported?:boolean;definitionsSupported?:boolean;zones:Zone[];endpoints:Endpoint[];refresh:()=>Promise<unknown>};
type QueueItem={key:string;track:Track};
const active=(job:SyncStart)=>["preparing","committed","playing","pausing","paused","seeking"].includes(job.phase);
const phases:Record<SyncStart["phase"],string>={preparing:"正在准备各音区",committed:"已准备，等待起播",playing:"正在播放",pausing:"正在暂停整组",paused:"整组已暂停",seeking:"正在定位整组",ended:"已结束",cancelled:"已撤销",failed:"分组播放已停止",interrupted:"核心重启，播放已中断"};
const memberPhases:Record<string,string>={preparing:"准备中",ready:"已就绪",playing:"播放中",pausing:"正在收尾",paused:"已暂停",ended:"已结束",cancelled:"已撤销"};
function readControl(value:any):Control|undefined {
 const r=value?.request;
 if(typeof r?.request_id!=="string"||!/^[0-9a-f-]{36}$/i.test(r.request_id))return;
 if(r.action==="seek"||r.action==="jump") {
  if(!(Number.isSafeInteger(r.queue_index)&&r.queue_index>=0&&r.queue_index<10000&&typeof r.position==="number"&&Number.isFinite(r.position)&&r.position>=0&&r.position<=86400))return;
  if(r.action==="jump"?!(Number.isSafeInteger(r.from_queue_index)&&r.from_queue_index>=0&&r.from_queue_index<10000&&r.from_queue_index!==r.queue_index):r.from_queue_index!==undefined)return;
 }else if(r.action==="members") {
  if(r.position!==undefined||r.queue_index!==undefined||r.from_queue_index!==undefined||![r.zone_ids,r.from_zone_ids].every(ids=>Array.isArray(ids)&&ids.length>=1&&ids.length<=4&&ids.every((id:unknown)=>typeof id==="string"&&id.length>0&&id.length<=200)&&new Set(ids).size===ids.length))return;
 }else if(!(["pause","resume","resync"].includes(r.action)&&r.position===undefined&&r.queue_index===undefined&&r.from_queue_index===undefined))return;
 if(r.action!=="members"&&(r.zone_ids!==undefined||r.from_zone_ids!==undefined))return;
 return {request:r,confirmed:value.confirmed===true,rejected:value.rejected===true,error:typeof value.error==="string"?value.error:undefined};
}
function read(key:string):Saved|null {
 try {const value=JSON.parse(sessionStorage.getItem(key)||"null"),r=value?.request,control=value?.control===undefined?undefined:readControl(value.control);
  if(value?.control!==undefined&&!control)return null;
  if(r?.sync_policy!==undefined&&(!Number.isInteger(r.sync_policy?.max_known_error_us)||r.sync_policy.max_known_error_us<100||r.sync_policy.max_known_error_us>50000||!Number.isInteger(r.sync_policy?.warmup_ms)||r.sync_policy.warmup_ms<1000||r.sync_policy.warmup_ms>10000))return null;
  if(r?.measured_version!==undefined&&r.measured_version!==1&&r.measured_version!==2)return null;
  if([r?.pcm_rate_adjustment,r?.automatic_rate_adjustment].some(value=>value!==undefined&&typeof value!=="boolean")||r?.pcm_rate_adjustment===true&&r.measured_version!==2||r?.automatic_rate_adjustment===true&&r.pcm_rate_adjustment!==true)return null;
  return typeof value?.title==="string"&&typeof r?.request_id==="string"&&/^[0-9a-f-]{36}$/i.test(r.request_id)&&typeof r.track_id==="string"&&typeof r.position==="number"&&Number.isFinite(r.position)&&r.position>=0&&(r.queue===undefined||Array.isArray(r.queue)&&r.queue.length>0&&r.queue.length<=10000&&r.queue[0]===r.track_id&&r.queue.every((id:unknown)=>typeof id==="string"&&id.length>0&&id.length<=200))&&Array.isArray(r.zone_ids)&&r.zone_ids.length>=(value.external===true?1:2)&&r.zone_ids.length<=4&&r.zone_ids.every((id:unknown)=>typeof id==="string")?{request:r,title:value.title,rejected:value.rejected===true,rejection:typeof value.rejection==="string"?value.rejection:undefined,external:value.external===true,control}:null;
 }catch{return null;}
}
function pcm(track:Track){return track.available&&track.bit_depth!==1&&!['dsf','dff','unknown',''].includes(track.format.toLowerCase())&&track.sample_rate>0&&track.sample_rate<=768000;}
function resolvable(track:Track){return track.available&&track.bit_depth!==1&&!['dsf','dff'].includes(track.format.toLowerCase())&&(!track.sample_rate||track.format==='unknown');}
export function GroupPlayback(props:Props) {
 const storageKey="rmusic.group-start.v1:"+JSON.stringify([props.coreId,props.userId,props.api.baseUrl]);
 return <GroupSession key={storageKey} {...props} storageKey={storageKey}/>;
}
function GroupSession({preset,api,supported,listSupported,queueSupported,outputTimingSupported,automaticRateSupported,syncPolicySupported,transportSupported,resyncSupported,queueJumpSupported,membershipSupported,clockDiagnosticsSupported,recoverySupported,definitionsSupported,zones,endpoints,refresh,storageKey}:Props&{storageKey:string}) {
 const [saved,setSaved]=useState<Saved|null>(()=>read(storageKey)),[job,setJob]=useState<SyncStart|null>(null);
 const [open,setOpen]=useState(false),[busy,setBusy]=useState(false),[notice,setNotice]=useState(""),[missing,setMissing]=useState(false);
 const [rooms,setRooms]=useState<string[]>([]),[track,setTrack]=useState<Track|null>(null),[query,setQuery]=useState(""),[tracks,setTracks]=useState<Track[]>([]),[loading,setLoading]=useState(false);
 useEffect(()=>{if(preset&&!saved){setRooms(preset.ids);setOpen(true);setNotice("");}},[preset?.key]);
 const [outputs,setOutputs]=useState(endpoints);
 const [queueMode,setQueueMode]=useState(false),[queue,setQueue]=useState<QueueItem[]>([]);
 const [resolving,setResolving]=useState("");const formatRequest=useRef<AbortController|null>(null);
 useEffect(()=>{setResolving("");return()=>formatRequest.current?.abort();},[api,open,queueMode]);
 const [outputTiming,setOutputTiming]=useState(false);
 const [automaticRate,setAutomaticRate]=useState(false);
 const [monitorSync,setMonitorSync]=useState(false),[syncBound,setSyncBound]=useState(1000),[syncWarmup,setSyncWarmup]=useState(3000);
 const policyValid=Number.isInteger(syncBound)&&syncBound>=100&&syncBound<=50000&&Number.isInteger(syncWarmup)&&syncWarmup>=1000&&syncWarmup<=10000;
 const [clockDeadline,setClockDeadline]=useState<number>();
 const [memberEdit,setMemberEdit]=useState(false);
 const [seek,setSeek]=useState<{queue_index:number;target:string;position:string}|null>(null);
 const [listOpen,setListOpen]=useState(false),[groups,setGroups]=useState<SyncStart[]>([]),[listLoading,setListLoading]=useState(false),[listNotice,setListNotice]=useState(""),[check,setCheck]=useState(0);
 const pending=useRef<AbortController|null>(null),current=useRef(api),refreshRef=useRef(refresh),savedRef=useRef(saved);current.current=api;refreshRef.current=refresh;savedRef.current=saved;
 useEffect(()=>{setBusy(false);setMemberEdit(false);setClockDeadline(undefined);return()=>{pending.current?.abort();pending.current=null;};},[api]);
 // Poll only this saved request, after mutations finish. Never re-POST automatically on reload.
 useEffect(()=>{
  if(!saved||busy)return;
  let live=true,controller:AbortController|undefined,timer:ReturnType<typeof setTimeout>|undefined;
  const poll=async()=>{
   controller=new AbortController();const timeout=setTimeout(()=>controller?.abort(),10000);let again=true;
   try {const started=performance.now();const value=await api.getGroupStart(saved.request.request_id,controller.signal);
    if(!live||current.current!==api)return;setNotice("");receiveJob(value,undefined,started);again=active(value)||value.settled===false;
    if(!again)void refreshRef.current();
   }catch(error){if(live&&current.current===api){const notFound=error instanceof ApiError&&error.status===404;setMissing(notFound);if(notFound&&saved.external)setJob(null);setNotice(notFound?(saved.external?"暂时找不到此分组，可重新查询或返回列表。":saved.rejected?saved.rejection||"起播请求未被接受，可修改选择。":"尚未确认起播结果，可重试本次请求。"):"暂时无法确认播放状态，请检查连接后重试。");}}
   finally{clearTimeout(timeout);if(live&&again)timer=setTimeout(poll,1000);}
  };
  void poll();return()=>{live=false;clearTimeout(timer);controller?.abort();};
 },[api,saved?.request.request_id,busy,check]);
 useEffect(()=>{
  if(!listSupported||!listOpen)return;let live=true,controller:AbortController|undefined,timer:ReturnType<typeof setTimeout>|undefined;setListLoading(true);setGroups([]);setListNotice("");
  const load=async()=>{controller=new AbortController();const timeout=setTimeout(()=>controller?.abort(),10000);
   try{const value=await api.listGroupStarts({active:true,limit:32,...(recoverySupported?{recoverable:true}:{})},controller.signal);if(live&&current.current===api){setGroups(value.items);setListNotice("");}}
   catch{if(live&&current.current===api)setListNotice("暂时无法读取分组，请检查连接或权限。");}
   finally{clearTimeout(timeout);if(live){setListLoading(false);timer=setTimeout(load,2000);}}
  };void load();return()=>{live=false;clearTimeout(timer);controller?.abort();};
 },[api,listSupported,listOpen,recoverySupported]);
 useEffect(()=>{
  if(!open)return;let live=true,controller:AbortController|undefined,timer:ReturnType<typeof setTimeout>|undefined;
  const load=async()=>{controller=new AbortController();const timeout=setTimeout(()=>controller?.abort(),10000);
   try{const value=await api.request<{items:Endpoint[]}>("/endpoints",{signal:controller.signal});if(live&&current.current===api)setOutputs(value.items);}
   catch{if(live&&current.current===api)setNotice("暂时无法刷新输出状态。");}
   finally{clearTimeout(timeout);if(live)timer=setTimeout(load,2000);}
  };void load();return()=>{live=false;clearTimeout(timer);controller?.abort();};
 },[api,open]);
 useEffect(()=>{
  if(!open)return;let live=true;const controller=new AbortController();setLoading(true);setTracks([]);
  const timer=setTimeout(async()=>{const timeout=setTimeout(()=>controller.abort(),10000);
   try{const value=await api.request<{items:Track[]}>(`/library/tracks?limit=20&q=${encodeURIComponent(query)}`,{signal:controller.signal});if(live&&current.current===api)setTracks(value.items);}
   catch{if(live&&current.current===api)setNotice("无法读取歌曲，请检查连接。");}
   finally{clearTimeout(timeout);if(live)setLoading(false);}
  },250);return()=>{live=false;clearTimeout(timer);controller.abort();};
 },[api,open,query]);
 function saveControl(control?:Control):boolean {
  const plan=savedRef.current;if(!plan)return false;const next={...plan,control};
  try{sessionStorage.setItem(storageKey,JSON.stringify(next));}catch{setNotice("无法保存本次控制，请检查会话存储后重试。");return false;}
  savedRef.current=next;setSaved(next);return true;
 }
 function receiveJob(value:SyncStart,accepted?:string,started?:number) {
  const plan=savedRef.current;if(plan&&plan.request.request_id!==value.id)return;
  const now=performance.now();setClockDeadline(value.clock_diagnostics&&started!==undefined&&value.clock_diagnostics.valid_for_ms>0?now+Math.max(0,value.clock_diagnostics.valid_for_ms-(now-started)):undefined);
  setJob(value);setMissing(false);const control=plan?.control;if(!control||control.rejected&&active(value))return;
  const echo=value.transport?.request_id===control.request.request_id;
  if(!active(value)||echo&&value.transport?.state!=="pending"||!echo&&(accepted===control.request.request_id||control.confirmed))saveControl();
  else if((echo||accepted===control.request.request_id)&&(!control.confirmed||control.error))saveControl({...control,confirmed:true,error:undefined});
 }
 async function mutateControl(plan:Saved) {
  const command=plan.control;if(!command||pending.current||!transportSupported||command.request.action==="jump"&&!queueJumpSupported||command.request.action==="members"&&!membershipSupported)return;
  const controller=new AbortController();pending.current=controller;setBusy(true);setNotice("");const timeout=setTimeout(()=>controller.abort(),10000);
  try{const value=await api.controlGroupStart(plan.request.request_id,command.request,controller.signal);
   if(current.current===api&&!controller.signal.aborted&&pending.current===controller){receiveJob(value,command.request.request_id);void refreshRef.current();}
  }catch(error){const latest=savedRef.current?.control;
   if(current.current===api&&pending.current===controller&&latest?.request.request_id===command.request.request_id){
    const rejected=error instanceof ApiError&&[400,401,403,404,409,422].includes(error.status);
    saveControl({...latest,rejected,error:error instanceof ApiError?error.message:"尚未收到控制结果；重试将使用同一请求。"});
   }
  }finally{clearTimeout(timeout);if(pending.current===controller){pending.current=null;if(current.current===api)setBusy(false);}}
 }
 function beginControl(request:SyncControlRequest):boolean {
  const plan=savedRef.current;if(!transportSupported||request.action==="jump"&&!queueJumpSupported||request.action==="members"&&!membershipSupported||pending.current||plan?.control||!job||!active(job)||["pausing","seeking"].includes(job.phase))return false;
  if(!saveControl({request}))return false;void mutateControl(savedRef.current!);return true;
 }
 function reason(zone:Zone):string {
  const endpoint=outputs.find(e=>e.id===zone.endpoint_id);
  if(zone.status!=="stopped"||zone.playback_id)return "请先停止此音区";
  if(!endpoint?.online)return "输出离线";
  if(endpoint.capabilities.scheduled_start_v1!==true||endpoint.capabilities.continuous_pcm!==true)return "此输出暂不支持分组起播";
  if((queueMode||outputTiming)&&(!queueSupported||endpoint.capabilities.scheduled_queue_v1!==true))return "此输出暂不支持连续组队列";
  if(outputTiming&&(!outputTimingSupported||endpoint.capabilities.group_output_v2!==true||endpoint.capabilities.measured_listens_v2!==true))return "此输出暂不支持新版分组计时";
  if(automaticRate){const why=groupRateReason(endpoint,true);if(why)return why;}
  if(monitorSync&&(!syncPolicySupported||endpoint.capabilities.device_clock_v1!==true||endpoint.capabilities.programme_clock_v1!==true))return "此输出缺少配对时钟反馈";
  if(!endpoint.clock_calibration)return "输出时钟正在准备";
  if(rooms.some(id=>id!==zone.id&&zones.find(z=>z.id===id)?.endpoint_id===zone.endpoint_id))return "同一输出只能选择一个音区";
  return "";
 }
 function automaticReason():string {
  if(!automaticRateSupported)return "当前核心尚未提供实验自动校正";
  if(!queueSupported||!outputTimingSupported)return "当前核心尚未提供新版分组计时";
  if(rooms.length<2)return "请先选择至少两个音区";
  for(const id of rooms){const zone=zones.find(zone=>zone.id===id),why=groupRateReason(outputs.find(endpoint=>endpoint.id===zone?.endpoint_id),true);if(why)return why;}
  return "";
 }
 async function mutate(kind:"start"|"cancel",plan:Saved) {
  if(pending.current||kind==="start"&&(plan.external||plan.control||(plan.request.measured_version===2&&(!outputTimingSupported||!queueSupported)||plan.request.automatic_rate_adjustment===true&&!automaticRateSupported)))return;
  const controller=new AbortController();pending.current=controller;setBusy(true);setNotice("");
  const timeout=setTimeout(()=>controller.abort(),10000);
  try {const value=await (kind==="start"?api.startGroup(plan.request,controller.signal):api.cancelGroupStart(plan.request.request_id,controller.signal));
   if(current.current===api&&!controller.signal.aborted){receiveJob(value);void refreshRef.current();}
  }catch(error){if(current.current===api&&pending.current===controller){
   if(kind==="start"&&error instanceof ApiError&&[400,422].includes(error.status)){const rejected={...plan,rejected:true,rejection:error.message};try{sessionStorage.setItem(storageKey,JSON.stringify(rejected));}catch{}savedRef.current=rejected;setSaved(rejected);}
   setNotice(error instanceof ApiError?error.message:"尚未收到请求结果；重试会继续使用同一请求，不会另建一组。");
  }}finally{clearTimeout(timeout);if(pending.current===controller){pending.current=null;if(current.current===api)setBusy(false);}}
 }
 function start(event:FormEvent) {
  event.preventDefault();const selected=queueMode?queue.map(item=>item.track):track?[track]:[];
  if(monitorSync&&(!syncPolicySupported||!policyValid))return;
  if(automaticRate&&(!outputTiming||automaticReason()))return;
  if(busy||saved||!supported||!selected.length||selected.some(item=>!pcm(item))||queueMode&&(!queueSupported||selected.length>10000)||rooms.length<2||rooms.length>4||rooms.some(id=>{const z=zones.find(z=>z.id===id);return !z||!!reason(z);}))return;
  const first=selected[0];
  const plan:Saved={request:{...(monitorSync?{sync_policy:{max_known_error_us:syncBound,warmup_ms:syncWarmup}}:{}),request_id:clientId(),track_id:first.id,zone_ids:rooms,position:0,...(queueMode||outputTiming?{queue:selected.map(item=>item.id)}:{}),...(outputTiming?{measured_version:2 as const}:{}),...(automaticRate?{automatic_rate_adjustment:true,pcm_rate_adjustment:true}:{})},title:queueMode?`${first.title} · ${selected.length} 首`:first.title};
  try{sessionStorage.setItem(storageKey,JSON.stringify(plan));}catch{setNotice("无法保存本次请求，请检查会话存储后重试。");return;}
  savedRef.current=plan;setSaved(plan);setJob(null);setMissing(false);setOpen(false);void mutate("start",plan);
 }
 function clear(selectSong=true) {
  try{sessionStorage.removeItem(storageKey);}catch{setNotice("无法清除本次请求，请检查会话存储后重试。");return;}
  savedRef.current=null;setSaved(null);setJob(null);setSeek(null);setMemberEdit(false);setClockDeadline(undefined);setNotice("");setMissing(false);setTrack(null);setQueue([]);setOutputTiming(false);setAutomaticRate(false);setOpen(selectSong);if(!selectSong)setListOpen(true);
 }
 function follow(value:SyncStart) {
  if(busy||savedRef.current?.control)return;const plan:Saved={request:{request_id:value.id,track_id:value.track_id,zone_ids:value.members.map(m=>m.zone_id),position:value.position},title:value.title||"分组播放",external:true};
  try{sessionStorage.setItem(storageKey,JSON.stringify(plan));}catch{setNotice("无法保存查看记录，请检查会话存储后重试。");return;}
  savedRef.current=plan;setSaved(plan);setJob(value);setSeek(null);setMemberEdit(false);setClockDeadline(undefined);setMissing(false);setNotice("");setOpen(false);setListOpen(false);
 }
 function changeMode(enabled:boolean){setQueueMode(enabled);if(enabled&&!queue.length&&track)setQueue([{key:clientId(),track}]);}
 async function chooseTrack(item:Track){
  if(formatRequest.current)return;
  let selected=item;
  if(!pcm(item)){
   const controller=new AbortController();formatRequest.current=controller;setResolving(item.id);setNotice("正在解析实际音频格式…");
   try{const info=await api.request<Partial<Track>>(`/library/tracks/${encodeURIComponent(item.id)}/audio-info`,{signal:controller.signal});
    if(controller.signal.aborted||current.current!==api)return;
    selected={...item,...Object.fromEntries(['format','sample_rate','bit_depth','channels','bitrate'].filter(key=>info[key as keyof Track]!=null).map(key=>[key,info[key as keyof Track]]))};
    setTracks(old=>old.map(track=>track.id===item.id?selected:track));
    if(!pcm(selected)){setNotice("尚未取得可用于分组的实际 PCM 格式，请稍后重试。");return;}setNotice("");
   }catch(error){if(!controller.signal.aborted&&current.current===api)setNotice(error instanceof Error?error.message:"音频格式解析失败，请重试");return;}
   finally{if(formatRequest.current===controller){formatRequest.current=null;setResolving("");}}
  }
  if(queueMode){const entry={key:clientId(),track:selected};setQueue(old=>old.length<10000?[...old,entry]:old);}else setTrack(selected);
 }
 function move(index:number,offset:number){setQueue(old=>{const next=[...old],target=index+offset;if(target<0||target>=next.length)return old;[next[index],next[target]]=[next[target],next[index]];return next;});}
 const selectionValid=queueMode?queueSupported&&queue.length>0&&queue.length<=10000&&queue.every(item=>pcm(item.track)):track&&pcm(track);
 const editable=!!job&&!active(job)||!!saved?.rejected&&missing;
 const controlLocked=busy||!!saved?.control||!!job&&["pausing","seeking"].includes(job.phase);
 const seekTarget=seek?(queueJumpSupported&&job?.queue?Number(seek.target)-1:seek.queue_index):0;
 const seekValid=!!seek&&!!job&&active(job)&&seek.target.trim()!==""&&Number.isSafeInteger(seekTarget)&&seekTarget>=0&&seekTarget<(job.queue?.length||1)&&seek.position.trim()!==""&&Number.isFinite(Number(seek.position))&&Number(seek.position)>=0&&Number(seek.position)<=86400&&job.playhead?.queue_index===seek.queue_index;
 function jumpBy(offset:number){if(job?.queue&&job.playhead)beginControl({request_id:clientId(),action:"jump",from_queue_index:job.playhead.queue_index,queue_index:job.playhead.queue_index+offset,position:0});}
 return <section className="group-playback" aria-label="分组播放">
  <div className="section-heading"><div><h2>分组播放</h2><p className="muted">让 2–4 个音区一起播放，统一控制歌曲和进度。</p></div>
   <div className="group-actions">{definitionsSupported&&<SavedOutputGroups api={api} zones={zones} canChoose={!saved&&!busy} onChoose={ids=>{setRooms(ids);setNotice("");setOpen(true);}}/>}{!saved&&<button className="group-create" disabled={!supported} onClick={()=>{setOpen(true);setNotice("");}}>选择音区和歌曲</button>}{listSupported&&<button disabled={busy} onClick={()=>setListOpen(old=>!old)}>{listOpen?"收起分组列表":"查看其他分组"}</button>}</div>
  </div>
  {!supported&&<p className="fine-print">此核心版本暂不支持分组播放。</p>}
  {listSupported&&listOpen&&<div className="group-list"><p className="fine-print">切换查看不会改变正在播放的分组。</p>{listNotice&&<p role="alert">{listNotice}</p>}{listLoading?<p className="muted">正在读取分组…</p>:groups.length?groups.map(value=><button key={value.id} disabled={busy||!!saved?.control||value.id===saved?.request.request_id} onClick={()=>follow(value)}><span><strong>{value.title||"分组播放"}</strong><small>{value.members.map(m=>zones.find(z=>z.id===m.zone_id)?.name||"音区").join("、")}</small></span><span>{phases[value.phase]} · 查看</span></button>):<p className="muted">当前没有活动分组。</p>}</div>}
  {saved&&<div className="group-status">
   <strong>{saved.title}</strong><p className="group-phase" role="status">{busy?"正在提交请求…":job?(job.phase==="seeking"&&job.transport?.action==="members"?"正在调整分组成员":job.phase==="seeking"&&job.transport?.action==="resync"?"正在重新同步整组":phases[job.phase]):saved.rejected&&missing?"起播请求未被接受":"正在确认起播结果…"}</p>
   {job?.queue&&<p>队列共 {job.queue.length} 首</p>}
   {job?.playhead&&<p className="fine-print">整组位置：{job.queue?`第 ${job.playhead.queue_index+1} 首 · `:""}{job.playhead.position.toLocaleString("zh-CN",{maximumFractionDigits:3})} 秒{typeof job.listened_seconds==="number"?` · 已听 ${job.listened_seconds.toLocaleString("zh-CN",{maximumFractionDigits:3})} 秒`:""}</p>}
   {job&&<ul>{job.members.map(member=><li key={member.zone_id}><span>{zones.find(z=>z.id===member.zone_id)?.name||"音区"} · {job.phase==="interrupted"&&member.phase!=="ended"?"已中断":memberPhases[member.phase]||member.phase}</span><small>{job.queue&&typeof member.queue_index==="number"?`第 ${member.queue_index+1} / ${job.queue.length} 首 · `:""}{member.sample_rate?`${member.sample_rate/1000} kHz · ${member.frames.toLocaleString()} 帧`:member.phase==="paused"?`${member.position.toLocaleString("zh-CN",{maximumFractionDigits:3})} 秒`:"尚未输出"}</small></li>)}</ul>}

   {job?.sync_monitor&&<p role="status">同步监测 · {({warming:"等待时钟收敛",within_known_bound:"已知误差在设定范围内",violated:"超限，已停止整组",inactive:"已停止监测"})[job.sync_monitor.state]}{job.sync_monitor.observed_known_bound_ns!=null?` · ${(job.sync_monitor.observed_known_bound_ns/1000).toFixed(1)} µs / ${job.sync_monitor.max_known_error_us} µs`:""}</p>}
   {job?.error&&<p role="alert">{job.error}</p>}
   {job?.transport?.error&&job.transport.error!==job.error&&<p role="alert">{job.transport.error}</p>}
   {job&&!active(job)&&job.settled===false&&<p className="fine-print">正在等待各输出的最终消费帧…</p>}
   {job&&!active(job)&&job.settled===true&&job.members.some(m=>m.final_report_received===false)&&<p className="fine-print">部分输出未确认最终帧；统计仅包含已确认的播放。</p>}
   <div className="group-actions">
    {transportSupported&&job&&active(job)&&<>
     {job.phase==="paused"?<button className="primary" disabled={controlLocked} onClick={()=>beginControl({request_id:clientId(),action:"resume"})}>继续整组</button>:<button className="primary" disabled={controlLocked} onClick={()=>beginControl({request_id:clientId(),action:"pause"})}>暂停整组</button>}
     {resyncSupported&&job.phase==="playing"&&<button disabled={controlLocked} onClick={()=>beginControl({request_id:clientId(),action:"resync"})}>重新同步</button>}
     {queueJumpSupported&&job.queue&&<><button disabled={controlLocked||!job.playhead||job.playhead.queue_index<=0} onClick={()=>jumpBy(-1)}>整组上一首</button><button disabled={controlLocked||!job.playhead||job.playhead.queue_index+1>=job.queue!.length} onClick={()=>jumpBy(1)}>整组下一首</button></>}
     {membershipSupported&&<button disabled={controlLocked} onClick={()=>setMemberEdit(true)}>调整分组成员</button>}
     <button disabled={controlLocked||!job.playhead} onClick={()=>{if(job.playhead)setSeek({queue_index:job.playhead.queue_index,target:String(job.playhead.queue_index+1),position:String(job.playhead.position)});}}>定位整组</button>
    </>}
    {job&&active(job)&&<button disabled={busy} onClick={()=>void mutate("cancel",saved)}>撤销整组</button>}
    {!job&&(saved.external?<button disabled={busy} onClick={()=>setCheck(old=>old+1)}>重新查询</button>:<button disabled={busy||!supported||!!saved.control||(saved.request.measured_version===2&&(!outputTimingSupported||!queueSupported)||saved.request.automatic_rate_adjustment===true&&!automaticRateSupported)} onClick={()=>void mutate("start",saved)}>重试本次请求</button>)}
    {editable&&<button disabled={busy||!!saved.control} onClick={()=>clear()}>{saved.request.queue?.length||job?.queue?"选择新队列":"选择另一首歌"}</button>}
    {saved.external&&listSupported&&<button disabled={busy||!!saved.control} onClick={()=>clear(false)}>返回分组列表</button>}
   </div>
   {saved.control&&<div>
    <p className="fine-print">{saved.control.rejected?"本次控制请求未被接受。":saved.control.confirmed?"正在等待整组控制结果…":"尚未收到本次控制结果，刷新只会查询状态。"}</p>
    {saved.control.error&&<p role="alert">{saved.control.error}</p>}
    <div className="group-actions">
     {!saved.control.confirmed&&!saved.control.rejected&&<button disabled={busy||!transportSupported||saved.control.request.action==="jump"&&!queueJumpSupported||saved.control.request.action==="members"&&!membershipSupported} onClick={()=>void mutateControl(saved)}>重试本次控制</button>}
     <button disabled={busy} onClick={()=>setCheck(old=>old+1)}>重新查询控制结果</button>
     {saved.control.rejected&&<button disabled={busy} onClick={()=>{if(saveControl()){setNotice("");setCheck(old=>old+1);}}}>返回当前状态</button>}
    </div>
   </div>}
   {recoverySupported&&job?.phase==="interrupted"&&job.measured_version===2&&<GroupRecovery api={api} job={job} storageKey={storageKey} onBusy={setBusy} onRecovered={value=>{receiveJob(value);void refreshRef.current();}}/>}
   <details className="group-technical"><summary>同步状态与技术详情</summary><p className="fine-print">硬件与声学同步精度尚未验收。</p>
   {(job?.measured_version??saved.request.measured_version)===2&&<p className="fine-print">新版分组计时：按实际输出时长统计，整组计一次。{recoverySupported?"中断后须手动恢复或重新起播":"中断后须重新起播"}；{(job?.rate_adjustment?.enabled??saved.request.pcm_rate_adjustment)?"此组选择了 PCM 样本处理，不走原样路径。":"变速和漂移补偿未启用。"}</p>}
   {job?.buffer_recovery?.active&&<p className="fine-print" role="status">输出欠载，正在等待整组最终帧并重新预缓冲；从最早已确认的位置继续，可能短暂重播，不跳过未听到的片段，不开启变速。</p>}
   {!!job?.buffer_recovery?.count&&!job.buffer_recovery.active&&<p className="fine-print">本组已触发 {job.buffer_recovery.count} 次整组重新预缓冲；恢复本身不修改采样。</p>}
   {job?.rate_adjustment?.automatic&&<div className="fine-print" aria-label="实验自动校正状态"><p>实验自动校正 · 最近 Core 报告：{({inactive:"当前未运行",warming:"等待有效时钟反馈",tracking:"正在追踪",held:"反馈不足，目标归零"} as const)[job.rate_adjustment.controller?.state??"inactive"]}</p>{job.rate_adjustment.controller?.reason&&<p>{job.rate_adjustment.controller.reason}</p>}<p>参考音区：{zones.find(zone=>zone.id===job.members[0]?.zone_id)?.name||"尚未选择参考音区"}。目标最多 ±500 ppm；反馈失效或本机租约到期后回到标称速率，已排队样本不会被重写。严格同步尚未验证。</p>{job.members.map(member=><p key={member.zone_id}>{zones.find(zone=>zone.id===member.zone_id)?.name||"音区"} · {typeof member.requested_rate_ppm==="number"&&Number.isFinite(member.requested_rate_ppm)?`请求目标 ${member.requested_rate_ppm.toLocaleString("zh-CN",{maximumFractionDigits:2})} ppm`:"未设置变速目标"}</p>)}</div>}
   {clockDiagnosticsSupported&&job&&active(job)&&<GroupClockDiagnostics value={job.clock_diagnostics} deadline={clockDeadline} zones={zones}/>}
   {transportSupported&&job&&active(job)&&<p className="fine-print">继续时以进度最靠前的音区为准。继续和定位可能有短暂间隔。</p>}</details>
   {job&&active(job)&&<p className="fine-print">修改任一成员音区的播放或设置会撤销整组。{job.queue?"将按所选队列顺序播放。":"当前分组仅播放这一首歌曲。"}</p>}
  </div>}
  {notice&&!open&&<p role="alert">{notice}</p>}
  {memberEdit&&membershipSupported&&job&&<GroupMembers api={api} job={job} zones={zones} locked={controlLocked} onClose={()=>setMemberEdit(false)} onApply={(from,ids)=>beginControl({request_id:clientId(),action:"members",from_zone_ids:from,zone_ids:ids})}/>}
  {seek&&job&&<Modal title="整组定位" onClose={()=>setSeek(null)}><form onSubmit={event=>{event.preventDefault();if(seekValid&&!controlLocked&&beginControl(seekTarget===seek.queue_index?{request_id:clientId(),action:"seek",queue_index:seek.queue_index,position:Number(seek.position)}:{request_id:clientId(),action:"jump",from_queue_index:seek.queue_index,queue_index:seekTarget,position:Number(seek.position)}))setSeek(null);}}>
   <p className="fine-print">{queueJumpSupported&&job.queue?"可选择队列中的另一首曲目；返回已听曲目会开始新一次播放。跳过的部分不计听音。":"选择当前歌曲中的位置。"}所有音区一起定位，播放中会继续播放，暂停中保持暂停。</p>
   {queueJumpSupported&&job.queue&&<label>队列曲序（从 1 开始）<input aria-label="队列曲序（从 1 开始）" type="number" min="1" max={job.queue.length} step="1" value={seek.target} onChange={event=>setSeek({...seek,target:event.target.value})}/></label>}
   <label>曲内位置（秒）<input aria-label="曲内位置（秒）" type="number" min="0" max="86400" step="any" value={seek.position} onChange={event=>setSeek({...seek,position:event.target.value})}/></label>
   {job.playhead?.queue_index!==seek.queue_index&&<p role="alert">歌曲已切换，请关闭后重新定位。</p>}
   {!active(job)&&<p role="alert">分组已结束，请关闭此窗口。</p>}
   <button type="submit" disabled={!seekValid||controlLocked}>定位到此处</button>
  </form></Modal>}
  {open&&!saved&&<Modal title="分组起播" onClose={()=>setOpen(false)} wide><form onSubmit={start} className="group-form">
   {notice&&<p role="alert">{notice}</p>}
   <fieldset><legend>选择 2–4 个已停止的音区</legend>{zones.map(zone=>{const checked=rooms.includes(zone.id),why=reason(zone)||(!checked&&rooms.length>=4?"最多选择 4 个音区":"");return <label className="group-room" key={zone.id}><input type="checkbox" checked={checked} disabled={!checked&&!!why} onChange={e=>setRooms(old=>e.target.checked?[...old,zone.id]:old.filter(id=>id!==zone.id))}/><span>{zone.name}<small>{why||outputs.find(e=>e.id===zone.endpoint_id)?.name}</small></span></label>;})}</fieldset>
   {queueSupported&&<label className="group-room"><input name="group-queue-mode" type="checkbox" checked={queueMode} onChange={event=>changeMode(event.target.checked)}/><span>连续队列<small>按顺序播放，可重复添加同一首歌</small></span></label>}
   {queueSupported&&outputTimingSupported&&<details className="group-technical"><summary>高级同步选项{automaticRate?" · 自动校正已开启":outputTiming?" · 实际输出计时已开启":""}</summary>
   {queueSupported&&outputTimingSupported&&<label className="group-room"><input name="group-output-timing" type="checkbox" checked={outputTiming} onChange={event=>{setOutputTiming(event.target.checked);if(!event.target.checked)setAutomaticRate(false);}}/><span>新版分组计时（实验）<small>按实际输出时长统计，整组计一次；中断后须手动恢复或重播，单独开启不启用变速补偿</small></span></label>}
   {queueSupported&&outputTimingSupported&&<label className="group-room"><input name="group-automatic-rate" type="checkbox" checked={automaticRate} disabled={!automaticRate&&!!automaticReason()} onChange={event=>{setAutomaticRate(event.target.checked);if(event.target.checked)setOutputTiming(true);}}/><span>实验自动校正<small>{automaticReason()||"会启用新版计时并处理 PCM 样本，不走原样路径；限幅 ±500 ppm，不保证严格同步。"}</small></span></label>}
   {automaticRate&&<p className="fine-print" role={automaticReason()?"alert":undefined}>{automaticReason()||"已选择实验自动校正。各音区按实际输出时长整组计一次；没有有效反馈时回到标称速率。"}</p>}
   </details>}
   {syncPolicySupported&&<details className="group-technical"><summary>同步误差保护{monitorSync?" · 已开启":""}</summary><label className="group-room"><input type="checkbox" checked={monitorSync} onChange={event=>setMonitorSync(event.target.checked)}/><span>超限停止整组<small>依据实际设备与节目时钟；不代表硬件或声学精度已验收。</small></span></label>{monitorSync&&<div className="sync-policy-fields"><label>已知误差上限 · µs<input type="number" min="100" max="50000" step="1" value={syncBound} onChange={event=>setSyncBound(Number(event.target.value))}/></label><label>时钟收敛时间 · ms<input type="number" min="1000" max="10000" step="1" value={syncWarmup} onChange={event=>setSyncWarmup(Number(event.target.value))}/></label></div>}</details>}
   <label>搜索要播放的歌曲<input value={query} onChange={e=>setQuery(e.target.value)} placeholder="歌曲或艺术家"/></label>
   <div className="group-tracks" aria-label="选择歌曲">{loading?<p className="muted">正在读取歌曲…</p>:tracks.length?tracks.map(item=><button type="button" key={item.id} disabled={!!resolving||!pcm(item)&&!resolvable(item)||queueMode&&queue.length>=10000} aria-pressed={queueMode?undefined:track?.id===item.id} onClick={()=>void chooseTrack(item)}><Cover api={api} track={item}/><span className="group-track-title"><strong>{item.title}</strong><small><CatalogLink track={item}/></small></span><small>{pcm(item)?`${queueMode?"添加 · ":""}${quality(item)}`:(item.bit_depth===1||['dsf','dff'].includes(item.format.toLowerCase()))?"DSD 暂不支持分组":resolving===item.id?"正在解析…":resolvable(item)?"格式待解析 · 点击解析":"音源不可用于分组"}</small></button>):<p className="muted">没有找到歌曲。</p>}</div>
   {queueMode?<div><p>队列 · {queue.length} 首</p>{queue.length?<ol className="group-queue" aria-label="播放顺序">{queue.map((entry,index)=><li key={entry.key}><span>{index+1}. {entry.track.title}<small><CatalogLink track={entry.track}/> · {quality(entry.track)}</small></span><div><button type="button" aria-label={`上移第 ${index+1} 首`} disabled={index===0} onClick={()=>move(index,-1)}>上移</button><button type="button" aria-label={`下移第 ${index+1} 首`} disabled={index===queue.length-1} onClick={()=>move(index,1)}>下移</button><button type="button" aria-label={`移除第 ${index+1} 首`} onClick={()=>setQueue(old=>old.filter(item=>item.key!==entry.key))}>移除</button></div></li>)}</ol>:<p className="muted">从上方添加歌曲到队列。</p>}</div>:track&&<p>已选歌曲：{track.title}</p>}
   <p className="fine-print">{queueMode?"按顺序播放；采样率、声道或位深变化时，各音区一起重新准备。默认不升频。":"只播放所选单曲。"}保留各音区的音量与 DSP，输出失败会撤销整组。严格同步尚未验收。</p>
   <button className="primary group-start-button" disabled={!supported||busy||monitorSync&&!policyValid||!selectionValid||automaticRate&&!!automaticReason()||rooms.length<2||rooms.length>4||rooms.some(id=>{const z=zones.find(z=>z.id===id);return !z||!!reason(z);})}>同时起播</button><small className="group-selection-status" role="status">已选 {rooms.length} 个音区 · {queueMode?queue.length:track?1:0} 首歌曲{rooms.length<2?" · 请至少选择两个音区":""}</small>
  </form></Modal>}
 </section>;
}
