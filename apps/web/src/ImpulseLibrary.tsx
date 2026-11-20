import {useEffect,useRef,useState} from "react";
import {ApiError,type ImpulseAsset,type ImpulseAssets,type RMusicClient} from "@r-music/sdk";
import {Upload,Trash2,RefreshCw} from "lucide-react";
import {IconButton} from "./components";
import {formatBytes} from "./browserCache";

export function ImpulseLibrary({api,onError,selected,onSelect}:{api:RMusicClient;onError:(error:unknown)=>void;selected?:string[];onSelect?:(asset:ImpulseAsset,items:ImpulseAsset[])=>void}) {
  const [assets,setAssets]=useState<ImpulseAssets|null>(null),[error,setError]=useState(""),[message,setMessage]=useState(""),[busy,setBusy]=useState(false),[revision,setRevision]=useState(0);
  const context=useRef<AbortController|null>(null),operation=useRef(false),input=useRef<HTMLInputElement>(null);
  const report=useRef(onError);report.current=onError;
  function failure(error:unknown){setError(error instanceof ApiError&&error.status===404?"当前核心不支持响应文件库，请升级核心后重试。":error instanceof Error?error.message:"响应文件操作失败");if(error instanceof ApiError&&error.status===401)report.current(error);}
  useEffect(()=>{
    const controller=new AbortController();context.current=controller;operation.current=false;setAssets(null);setError("");setMessage("");setBusy(false);
    api.listImpulses(controller.signal).then(value=>{if(!controller.signal.aborted)setAssets(value);}).catch(error=>{if(!controller.signal.aborted)failure(error);});
    return()=>controller.abort();
  },[api,revision]);
  async function change(value:{file:File}|{id:string;name:string}) {
    const controller=context.current;if(!controller||controller.signal.aborted||operation.current||!assets)return;
    operation.current=true;setBusy(true);setError("");setMessage("");
    try {
      if("file" in value){
        const file=value.file;if(file.size===0)throw new Error("响应文件为空");if(file.size>assets.limits.max_upload_bytes)throw new Error(`响应文件不能超过 ${formatBytes(assets.limits.max_upload_bytes)}`);
        const bytes=await file.arrayBuffer();controller.signal.throwIfAborted();const imported=await api.importImpulse(bytes,file.name,controller.signal);
        if(!controller.signal.aborted){setAssets(current=>current?{...current,items:[...current.items.filter(item=>item.id!==imported.id),imported]}:current);setMessage(`已导入 ${imported.name}`);}
      }else{
        await api.deleteImpulse(value.id,controller.signal);
        if(!controller.signal.aborted){setAssets(current=>current?{...current,items:current.items.filter(item=>item.id!==value.id)}:current);setMessage(`已删除 ${value.name}`);}
      }
    }catch(error){if(!controller.signal.aborted)failure(error);}
    finally{if(!controller.signal.aborted){operation.current=false;setBusy(false);}}
  }
  return <section className="impulse-library" aria-label="卷积响应文件库" aria-busy={busy||(!assets&&!error)}>
    <div className="section-heading"><h3>卷积响应文件</h3><div className="impulse-actions">
      <IconButton label="刷新响应文件" disabled={busy} onClick={()=>setRevision(value=>value+1)}><RefreshCw size={15}/></IconButton>
      <button type="button" disabled={!assets||busy} onClick={()=>input.current?.click()}><Upload size={15}/>{busy?"正在处理…":"导入 WAV"}</button>
      <input ref={input} hidden type="file" accept=".wav,audio/wav,audio/x-wav" aria-label="导入 WAV 响应" onChange={event=>{const file=event.target.files?.[0];event.target.value="";if(file)void change({file});}}/>
    </div></div>
    <p className="fine-print">保留响应原始增益。{onSelect?"每个采样率选择一个响应，播放时精确匹配。":"该输出尚不能应用响应，可管理文件。"}</p>
    {error&&<p role="alert" className="notice">{error}</p>}{message&&<p role="status" className="fine-print">{message}</p>}
    {!assets&&!error&&<p role="status" className="fine-print">正在读取响应文件…</p>}
    {assets&&<><p className="fine-print">{assets.items.length} / {assets.limits.max_files} 个文件 · {formatBytes(assets.items.reduce((sum,item)=>sum+item.bytes,0))} / {formatBytes(assets.limits.max_storage_bytes)} · 单文件上限 {formatBytes(assets.limits.max_upload_bytes)}</p>
      {assets.items.length===0?<p className="fine-print">尚未导入响应文件</p>:<ul className="impulse-list">{assets.items.map(item=><li key={item.id}>{onSelect&&<input type="checkbox" aria-label={`使用响应 ${item.name}`} checked={selected?.includes(item.id)===true} onChange={()=>onSelect(item,assets.items)}/>}<span><strong>{item.name}</strong><small>{item.sample_rate/1000} kHz · {item.channels} 声道 · {item.frames} 采样点 · {(item.frames/item.sample_rate*1000).toFixed(2)} ms · {formatBytes(item.bytes)}</small></span><IconButton label={`删除响应 ${item.name}`} disabled={busy} onClick={()=>void change({id:item.id,name:item.name})}><Trash2 size={16}/></IconButton></li>)}</ul>}</>}
  </section>;
}
