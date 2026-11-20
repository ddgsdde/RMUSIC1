import {useEffect,useState} from "react";
import type {RMusicClient} from "@r-music/sdk";
interface RepairJob {id:string;status:string;phase:string;processed:number;total:number;error?:string;options:{concurrency:number;checked?:number;skipped?:number}}
interface RepairStatus {enabled:boolean;job:RepairJob|null}
export function CacheRepair({api,onError}:{api:RMusicClient;onError:(error:unknown)=>void}){
 const[data,setData]=useState<RepairStatus|null>(null),[concurrency,setConcurrency]=useState(1),[busy,setBusy]=useState(false),[revision,setRevision]=useState(0),[confirm,setConfirm]=useState(false),[unsupported,setUnsupported]=useState(false);
 useEffect(()=>{const controller=new AbortController();let pending=false;const load=async()=>{if(pending||document.hidden)return;pending=true;try{const result=await api.request<RepairStatus>("/library/cache-repair",{signal:controller.signal});if(!controller.signal.aborted)setData(result);}catch(error){if(!controller.signal.aborted){if((error as {status?:number}).status===404)setUnsupported(true);else onError(error);}}finally{pending=false;}};void load();const timer=setInterval(load,5000);return()=>{controller.abort();clearInterval(timer);};},[api,onError,revision]);
 async function act(operation:()=>Promise<unknown>){setBusy(true);try{await operation();setRevision(value=>value+1);setConfirm(false);}catch(error){onError(error);}finally{setBusy(false);}}
 if(unsupported)return <p className="fine-print">当前 Core 尚未提供缺失补齐功能，升级 Core 后可使用。</p>;
 const active=data?.job&&!['completed','failed','cancelled'].includes(data.job.status);
 return <section className="settings-section"><h3>补齐所有缺失</h3><p>只检查受管云端歌曲文件。缺失时保留记录，平时不会立即下载，后续播放时按需拉取。手动补齐会分页检查并下载缺失文件；离线目录和回收站会跳过。</p>
  {data&&!data.enabled&&<p className="notice">Core 音频缓存已关闭，不会下载或补齐文件；现有歌曲仍可直接拉流播放。</p>}
  <label>补齐并发<input aria-label="补齐并发" type="number" min={1} max={16} value={concurrency} onChange={event=>setConcurrency(Number(event.target.value))}/></label><p className="fine-print">实际并发同时受后台总并发、歌曲拉取上限和连接器限流约束。前台播放优先，上游限流时冷却，不会用重试打满接口。</p>
  {confirm?<div role="group" aria-label="确认补齐缺失"><p>补齐会占用 Core 所在磁盘空间，只处理缺失文件。确认按当前并发开始？</p><button disabled={busy} onClick={()=>void act(()=>api.post("/library/cache-repair",{concurrency}))}>确认开始补齐</button><button onClick={()=>setConfirm(false)}>取消</button></div>:<button disabled={!data?.enabled||busy||!!active||concurrency<1||concurrency>16} onClick={()=>setConfirm(true)}>补齐所有缺失</button>}
  {data?.job&&<div><p role="status">{data.job.phase||data.job.status} · {data.job.processed} / {data.job.total}</p>{data.job.error&&<p role="alert">{data.job.error}</p>}{active&&<><p className="fine-print">任务并发：{data.job.options.concurrency}</p><button disabled={busy} onClick={()=>void act(()=>api.request(`/library/cache-repair/${data.job!.id}`,{method:"PATCH",body:JSON.stringify({concurrency})}))}>应用并发</button><button disabled={busy} onClick={()=>void act(()=>api.patch(`/jobs/${data.job!.id}`,{status:data.job!.status==="paused"?"running":"paused"}))}>{data.job.status==="paused"?"继续":"暂停"}</button><button disabled={busy} onClick={()=>void act(()=>api.patch(`/jobs/${data.job!.id}`,{status:"cancelled"}))}>取消任务</button></>}</div>}
 </section>;
}
