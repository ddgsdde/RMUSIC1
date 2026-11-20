import {useEffect,useRef,useState} from "react";
import {ApiError,type RMusicClient,type SyncControlRequest,type SyncStart,type Zone} from "@r-music/sdk";
import {clientId} from "./clientId";
type Pending={group:string;title:string;request:SyncControlRequest;confirmed?:boolean;rejected?:boolean;error?:string};
const uuid=/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const index=(n:unknown):n is number=>Number.isInteger(n)&&Number(n)>=0&&Number(n)<10000;
const position=(n:unknown):n is number=>typeof n==="number"&&Number.isFinite(n)&&n>=0&&n<=86400;
const active=(job:SyncStart)=>["preparing","committed","playing","pausing","paused","seeking"].includes(job.phase);
function read(key:string):Pending|undefined {
 try{const value=JSON.parse(sessionStorage.getItem(key)||"null"),r=value?.request;if(!uuid.test(value?.group)||!uuid.test(r?.request_id))return;
  let request:SyncControlRequest;
  if(r.action==="pause"||r.action==="resume")request={request_id:r.request_id,action:r.action};
  else if(r.action==="seek"&&index(r.queue_index)&&position(r.position))request={request_id:r.request_id,action:"seek",queue_index:r.queue_index,position:r.position};
  else if(r.action==="jump"&&index(r.from_queue_index)&&index(r.queue_index)&&position(r.position))request={request_id:r.request_id,action:"jump",from_queue_index:r.from_queue_index,queue_index:r.queue_index,position:r.position};else return;
  return {group:value.group,title:typeof value.title==="string"?value.title.slice(0,200):"分组播放",request,confirmed:value.confirmed===true,rejected:value.rejected===true,error:typeof value.error==="string"?value.error:undefined};
 }catch{return;}
}
export function useGroupTransport({api,coreId,userId,list,supported,jump,recovery,onError,onChange}:{api:RMusicClient;coreId:string;userId?:string;list:boolean;supported:boolean;jump:boolean;recovery:boolean;onError:(error:unknown)=>void;onChange:()=>unknown}) {
 const key=coreId&&userId?"rmusic.group-transport.v1:"+JSON.stringify([coreId,userId,api.baseUrl]):"";
 const [pending,setPending]=useState<Pending|undefined>(()=>key?read(key):undefined),[working,setWorking]=useState(false),[owner,setOwner]=useState(key);
 const saved=useRef(pending),request=useRef<AbortController|null>(null),current=useRef<{api:RMusicClient|null;key:string}>({api,key}),callbacks=useRef({onError,onChange});current.current={api,key};callbacks.current={onError,onChange};
 const valid=()=>current.current.api===api&&current.current.key===key;
 useEffect(()=>{current.current={api,key};const next=key?read(key):undefined;saved.current=next;setPending(next);setOwner(key);setWorking(false);
  return()=>{request.current?.abort();request.current=null;if(current.current.api===api&&current.current.key===key)current.current={api:null,key:""};};
 },[api,key]);
 function store(next:Pending|undefined):boolean {
  try{if(next)sessionStorage.setItem(key,JSON.stringify(next));else sessionStorage.removeItem(key);}
  catch{callbacks.current.onError(new Error("无法保存整组操作，请检查会话存储后重试。"));if(next)return false;}
  saved.current=next;setPending(next);return true;
 }
 function receive(value:SyncStart,record:Pending,accepted=false) {
  if(value.id!==record.group)throw new Error("整组操作响应身份不符");
  const echo=value.transport?.request_id===record.request.request_id;
  if(!active(value)||echo&&value.transport?.state!=="pending"||!echo&&(accepted||record.confirmed)){store(undefined);if(value.transport?.error)callbacks.current.onError(new Error(value.transport.error));}
  else if((accepted||echo)&&!record.confirmed)store({...record,confirmed:true,error:undefined});
  void callbacks.current.onChange();
 }
 useEffect(()=>{
  if(owner!==key||!pending||pending.rejected||working)return;let live=true,timer:ReturnType<typeof setTimeout>|undefined,controller:AbortController|undefined;
  const poll=async()=>{controller=new AbortController();const timeout=setTimeout(()=>controller?.abort(),10000);
   try{const value=await api.getGroupStart(pending.group,controller.signal);if(live&&valid())receive(value,pending);}
   catch{/* A query never resubmits the frozen command. */}
   finally{clearTimeout(timeout);if(live)timer=setTimeout(poll,1000);}
  };void poll();return()=>{live=false;clearTimeout(timer);controller?.abort();};
 },[api,key,owner,pending,working]);
 async function run(action:(controller:AbortController)=>Promise<void>):Promise<true> {
  if(request.current)return true;const controller=new AbortController();request.current=controller;setWorking(true);const timeout=setTimeout(()=>controller.abort(),10000);
  try{await action(controller);}catch(error){if(valid())callbacks.current.onError(error);}
  finally{clearTimeout(timeout);if(request.current===controller){request.current=null;if(valid())setWorking(false);}}return true;
 }
 async function send(record:Pending,controller:AbortController) {
  try{const value=await api.controlGroupStart(record.group,record.request,controller.signal);if(valid()&&!controller.signal.aborted)receive(value,record,true);}
  catch(error){if(valid()){
   const rejected=error instanceof ApiError&&[400,401,403,404,409,422].includes(error.status);
   store({...record,rejected,error:error instanceof ApiError?error.message:"整组尚未收到操作结果；重试会沿用同一请求。"});callbacks.current.onError(error);
  }}
 }
 function route(zone:Zone,action:string,body:unknown,eligible:boolean):false|Promise<true|false> {
  const input=(body&&typeof body==="object"?body:{}) as Record<string,unknown>;
  if(!eligible||!list||!key||!["pause","play","seek","previous","next"].includes(action)||action==="play"&&typeof input.track_id==="string")return false;
  if(owner!==key||request.current)return Promise.resolve(true);
  if(saved.current){callbacks.current.onError(new Error("请先确认或重试上一项整组操作。"));return Promise.resolve(true);}
  let found=false,lookedUp=false;
  return run(async controller=>{
   const groups=await api.listGroupStarts({active:true,limit:32,...(recovery?{recoverable:true}:{})},controller.signal);if(!valid()||controller.signal.aborted){found=true;return;}
   lookedUp=true;
   const group=groups.items.find(g=>active(g)&&g.members.some(m=>m.zone_id===zone.id))||groups.items.find(g=>g.phase==="interrupted"&&g.members.some(m=>m.zone_id===zone.id));if(!group)return;found=true;
   if(group.phase==="interrupted")throw new Error("分组已中断，请到音区页面手动恢复整组。");if(!supported)throw new Error("当前核心暂不支持整组控制。");
   const at=group.playhead?.queue_index??0;let command:SyncControlRequest;const request_id=clientId();
   if(group.queue&&["seek","previous","next"].includes(action)&&zone.queue_index!==at)throw new Error("当前曲序已更新，请稍后重试。");
   if(action==="pause")command={request_id,action:"pause"};
   else if(action==="play"&&input.queue_index===undefined)command={request_id,action:"resume"};
   else if(action==="seek"){if(!position(input.position))throw new Error("整组定位位置无效。");command={request_id,action:"seek",queue_index:at,position:input.position};}
   else{if(!jump||!group.queue)throw new Error("当前分组不支持切换曲序，请到音区页面选择队列。");const target=action==="next"?at+1:action==="previous"?at-1:input.queue_index;
    if(!index(target)||target>=group.queue.length)throw new Error("已到整组队列边界。");command=target===at?{request_id,action:"seek",queue_index:at,position:0}:{request_id,action:"jump",from_queue_index:at,queue_index:target,position:0};}
   const record:Pending={group:group.id,title:group.title||"分组播放",request:command};if(store(record))await send(record,controller);
  }).then(()=>found||!lookedUp||!valid());
 }
 return {route,pending:owner===key?pending:undefined,busy:owner!==key||working||!!pending?.confirmed&&!pending?.rejected,
  retry:()=>{const record=saved.current;if(owner!==key||!record||record.confirmed||record.rejected||request.current)return;if(!supported){callbacks.current.onError(new Error("当前核心暂不支持整组控制。"));return;}void run(controller=>send(record,controller));},
  dismiss:()=>{if(owner===key&&saved.current?.rejected)store(undefined);},
 };
}
