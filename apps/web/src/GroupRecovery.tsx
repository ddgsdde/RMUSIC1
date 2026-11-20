import {useEffect,useRef,useState} from "react";
import {ApiError,type GroupRecoveryRequest,type GroupRecoveryState,type RMusicClient,type SyncStart} from "@r-music/sdk";
import {clientId} from "./clientId";

type Saved={request:GroupRecoveryRequest;rejected?:boolean};
function read(key:string):Saved|undefined {
 try{const value=JSON.parse(sessionStorage.getItem(key)||"null"),r=value?.request;
  if(typeof r?.request_id==="string"&&/^[0-9a-f-]{36}$/i.test(r.request_id)&&typeof r.checkpoint_id==="string"&&/^[0-9a-f]{64}$/.test(r.checkpoint_id))return {request:{request_id:r.request_id,checkpoint_id:r.checkpoint_id},rejected:value.rejected===true};
 }catch{}return undefined;
}
export function GroupRecovery({api,job,storageKey,onRecovered,onBusy}:{api:RMusicClient;job:SyncStart;storageKey:string;onRecovered:(value:SyncStart)=>void;onBusy:(busy:boolean)=>void}) {
 const key=storageKey+":manual-recovery.v1:"+job.id;
 const [saved,setSaved]=useState<Saved|undefined>(()=>read(key)),[point,setPoint]=useState<GroupRecoveryState>();
 const [busy,setBusy]=useState(false),[notice,setNotice]=useState(""),[check,setCheck]=useState(0);
 const pending=useRef<AbortController|null>(null),current=useRef(api),callbacks=useRef({onRecovered,onBusy});current.current=api;callbacks.current={onRecovered,onBusy};
 useEffect(()=>()=>{pending.current?.abort();callbacks.current.onBusy(false);},[api]);
 useEffect(()=>{
  if(busy)return;let live=true,controller:AbortController|undefined,timer:ReturnType<typeof setTimeout>|undefined;setPoint(undefined);
  const poll=async()=>{controller=new AbortController();const timeout=setTimeout(()=>controller?.abort(),10000);
   try{const value=await api.getGroupRecovery(job.id,controller.signal);if(live&&current.current===api)setPoint(value);}
   catch(error){if(live&&current.current===api)setNotice(error instanceof ApiError?error.message:"暂时无法读取恢复状态，请检查连接。");}
   finally{clearTimeout(timeout);if(live)timer=setTimeout(poll,2000);}
  };void poll();return()=>{live=false;clearTimeout(timer);controller?.abort();};
 },[api,job.id,busy,check]);
 function store(value:Saved|undefined):boolean {
  try{if(value)sessionStorage.setItem(key,JSON.stringify(value));else sessionStorage.removeItem(key);setSaved(value);return true;}
  catch{setNotice("无法保存恢复请求，请检查会话存储后重试。");return false;}
 }
 async function recover() {
  if(pending.current||saved?.rejected||!saved&&!point?.available)return;
  const record=saved||{request:{request_id:clientId(),checkpoint_id:(point as Extract<GroupRecoveryState,{available:true}>).checkpoint_id}};
  if(!saved&&!store(record))return;
  const controller=new AbortController();pending.current=controller;setBusy(true);callbacks.current.onBusy(true);setNotice("");const timeout=setTimeout(()=>controller.abort(),10000);
  try{const value=await api.recoverGroupStart(job.id,record.request,controller.signal);
   if(current.current!==api||pending.current!==controller||controller.signal.aborted)return;
   if(value.id!==job.id||value.measured_version!==2)throw new Error("恢复响应身份不符");
   store(undefined);callbacks.current.onRecovered(value);
  }catch(error){if(current.current===api&&pending.current===controller){if(error instanceof ApiError&&error.status===409)store({...record,rejected:true});setNotice(error instanceof ApiError?error.message:"尚未收到恢复结果，请重试本次恢复；将沿用同一请求。");}}
  finally{clearTimeout(timeout);if(pending.current===controller){pending.current=null;setBusy(false);callbacks.current.onBusy(false);}}
 }
 return <div className="group-recovery">
  <p className="fine-print">重新连接原输出并确认最终帧后，可手动恢复整组。恢复会重新准备全部输出，不会自动起播。</p>
  {point?.available?<p className="fine-print">从第 {point.queue_index+1} 首 · {point.position.toFixed(3)} 秒继续。</p>:<p role="status" className="fine-print">{point?.reason||"正在查询恢复状态…"}</p>}
  {saved&&<p className="fine-print">{saved.rejected?"本次恢复请求已被拒绝，可放弃后重新查询。":"尚未收到恢复结果；刷新只查询状态，不会重复提交。"}</p>}
  {notice&&<p role="alert">{notice}</p>}
  <div className="group-actions">
   <button disabled={busy||!!saved?.rejected||!saved&&!point?.available} onClick={()=>void recover()}>{saved?"重试本次恢复":"手动恢复整组"}</button>
   {saved?.rejected&&<button disabled={busy} onClick={()=>{if(store(undefined)){setNotice("");setCheck(old=>old+1);}}}>放弃被拒绝的请求</button>}
  </div>
 </div>;
}
