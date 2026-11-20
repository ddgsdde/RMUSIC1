import {useEffect,useRef,useState} from "react";
import {invokeNative} from "./native";
import {formatBytes} from "./browserCache";
type Storage={tree?:string|null;name:string;available:boolean;error?:string;pending:number;progress?:{busy?:boolean;file?:string;received?:number;total?:number;moved?:number;error?:string}};
export function NativeStorageSettings({notify,onError,changed}:{notify:(message:string)=>void;onError:(error:unknown)=>void;changed:()=>void}){
 const [storage,setStorage]=useState<Storage>(),[choosing,setChoosing]=useState(false);
 const callbacks=useRef({notify,onError,changed});callbacks.current={notify,onError,changed};
 const read=()=>invokeNative<Storage>("native_storage_status").then(setStorage);
 useEffect(()=>{
  let active=true,remove:(()=>void)|undefined;
  void invokeNative<Storage>("native_storage_status").then(value=>{if(active)setStorage(value);}).catch(error=>{if(active)callbacks.current.onError(error);});
  void import("@tauri-apps/api/event").then(({listen})=>listen<NonNullable<Storage["progress"]>>("native-storage-state",event=>{
   if(!active)return;setStorage(old=>old?{...old,progress:event.payload}:old);
   if(!event.payload.busy)void read().then(()=>callbacks.current.changed()).catch(callbacks.current.onError);
  })).then(off=>{if(active)remove=off;else off();}).catch(error=>{if(active)callbacks.current.onError(error);});
  return()=>{active=false;remove?.();};
 },[]);
 async function choose(){setChoosing(true);try{const result=await invokeNative<{cancelled?:boolean}>("native_storage_choose");if(!result.cancelled){await read();notify("离线下载位置已保存");}}catch(error){onError(error);}finally{setChoosing(false);}}
 async function action(action:string){try{await invokeNative("native_storage_action",{action});await read();}catch(error){onError(error);}}
 const progress=storage?.progress,busy=choosing||!!progress?.busy;
 return <section className="settings-section" aria-label="离线下载位置">
  <div className="section-heading"><h3>离线下载位置</h3><span>{storage?.available===false?"需要重新授权":""}</span></div>
  <p><strong>{storage?.name||"应用内存储"}</strong></p>
  <div className="inline-form"><button disabled={busy||!storage} onClick={()=>void choose()}>{choosing?"正在选择…":storage?.available===false?"重新授权目录":"选择目录"}</button>
   {storage?.tree&&<button disabled={busy} onClick={()=>void action("reset")}>使用应用内存储</button>}
   {!!(storage?.pending||progress?.error)&&<button disabled={busy||storage?.available===false} onClick={()=>void action("retry")}>继续移动</button>}
  </div>
  <p className="fine-print">下载直接保存到所选目录。已有下载在前台自动移动，正在播放的文件稍后处理。</p>
  {progress?.busy&&<div role="status">正在移动 {progress.file} · {formatBytes(progress.received||0)} / {formatBytes(progress.total||0)}<progress aria-label="离线文件移动进度" value={progress.received||0} max={Math.max(1,progress.total||0)}/></div>}
  {!progress?.busy&&!!storage?.pending&&<p className="fine-print">{storage.pending} 个文件等待移动</p>}
  {(storage?.error||progress?.error)&&<p role="alert">{storage?.error||progress?.error}。原文件与下载记录保留。</p>}
 </section>;
}
