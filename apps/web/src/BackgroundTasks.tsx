import {useEffect,useState} from "react";
import type {RMusicClient} from "@r-music/sdk";
import {CacheRepair} from "./CacheRepair";
import {formatBytes} from "./browserCache";

interface Resources {budget_bytes:number;used_bytes:number;playback_reserved_bytes:number;process_rss_bytes?:number;active_tasks:number}
interface RequestLimits {search_ms:number;resolve_ms:number;lyrics_ms:number;playlists_ms:number;recommendations_ms:number;metadata_ms:number;comments_ms:number;cooldown_seconds:number}
interface Limits {mobile_opus_enabled?:boolean;cache_enabled?:boolean;cache_forced_off?:boolean;cache_limit_bytes?:number;managed_memory_bytes:number;background_concurrency:number;analysis_threads:number;playback_cache_mode:"full"|"partial";resources:Resources;song_fetch_concurrency:number;provider_request_limits:RequestLimits}
interface Job {id:string;kind:string;status:string;phase?:string;target?:string;processed:number;total:number;updated_at?:number;error?:string}
const kinds:Record<string,string>={cache_repair:"补齐所有缺失",library_files:"曲库文件管理",scan:"曲库扫描",metadata:"歌曲资料补全",loudness:"响度分析",cache_fill:"完整缓存补齐",cache_metadata:"缓存资料整理",download:"音乐文件下载",playlist_sync:"歌单同步",playlist_directory:"云端歌单目录",playlist_page:"云端歌单曲目刷新"};
const statuses:Record<string,string>={pending:"排队中",running:"正在执行",interrupted:"等待恢复",waiting:"等待资源",paused:"已暂停",cancelled:"已取消",completed:"已完成",failed:"失败",unable:"无法分析",idle:"空闲",waiting_provider:"等待音源",error:"出错"};
type Props={api:RMusicClient;onError:(error:unknown)=>void;notify:(message:string)=>void};
export function CoreResourceSettings({api,onError,notify}:Props) {
 const [value,setValue]=useState<Limits|null>(null),[resources,setResources]=useState<Resources|null>(null),[saving,setSaving]=useState(false);
 useEffect(()=>{let active=true;let pending=false;const load=async()=>{if(pending||document.hidden)return;pending=true;try{const result=await api.get<Limits>("/settings");if(active){setValue(old=>old||result);setResources(result.resources);}}catch(error){if(active)onError(error);}finally{pending=false;}};void load();const timer=setInterval(load,5000);return()=>{active=false;clearInterval(timer);};},[api,onError]);
 if(!value)return <p className="muted">正在读取核心资源限制…</p>;
 return <section className="settings-section core-resources"><h2>核心缓存与后台资源</h2>

  {value.cache_limit_bytes!==undefined&&<label>音频缓存预算 (GiB)<input type="number" min={1/1024**3} step="any" value={value.cache_limit_bytes/1024**3} onChange={e=>setValue({...value,cache_limit_bytes:Number(e.target.value)*1024**3})}/></label>}
  <label>播放缓存策略<select value={value.playback_cache_mode} onChange={e=>setValue({...value,playback_cache_mode:e.target.value as Limits["playback_cache_mode"]})}><option value="full">播放后补齐完整缓存（实听超过 30%）</option><option value="partial">仅缓存播放读取的部分</option></select></label>
  <p className="fine-print">播放立即开始；完整补齐任务在切歌后保留，资源不足时排队。部分模式不主动下载未读取的区间。</p>
  <div className="resource-fields"><label>受控内存预算 (MiB)<input type="number" min={32} max={131072} step={32} value={value.managed_memory_bytes/1024**2} onChange={e=>setValue({...value,managed_memory_bytes:Number(e.target.value)*1024**2})}/></label>
  <label>后台总并发<input type="number" min={1} max={16} value={value.background_concurrency} onChange={e=>setValue({...value,background_concurrency:Number(e.target.value)})}/></label>
  <label>单个分析线程<input type="number" min={1} max={16} value={value.analysis_threads} onChange={e=>setValue({...value,analysis_threads:Number(e.target.value)})}/></label></div>
  {resources&&<p className="resource-summary">受控分配与预留 {formatBytes(resources.used_bytes)} / {formatBytes(resources.budget_bytes)} · 播放预留 {formatBytes(resources.playback_reserved_bytes)} · 运行任务 {resources.active_tasks}{resources.process_rss_bytes!=null&&<> · Core 进程 {formatBytes(resources.process_rss_bytes)}</>}</p>}
  <p className="fine-print">预算管理缓存缓冲与后台任务，不是整个 Core 进程的硬上限。播放优先，响度分析同时最多一项；单核设备进一步收紧并发。</p>
  <label><input type="checkbox" checked={value.mobile_opus_enabled===true} onChange={e=>setValue({...value,mobile_opus_enabled:e.target.checked})}/>允许移动端 Opus 省流转码</label>
  <p className="fine-print">仅接受客户端明确选择的有损请求；不修改音频原件或网页无损策略。</p>
  <h3>音源请求与歌曲拉取</h3>
  <label>同时拉取歌曲数量<input type="number" min={1} max={16} value={value.song_fetch_concurrency??2} onChange={e=>setValue({...value,song_fetch_concurrency:Number(e.target.value)})}/></label>
  <p className="fine-print">按歌曲数量限制并发，不限制每秒下载次数。同一歌曲的多个读取共享名额；后台按队列等待，前台播放优先，后台让路后保留任务和已缓存进度。</p>
  {value.provider_request_limits&&<><div className="resource-fields">{([["resolve_ms","播放地址解析"],["search_ms","搜索"],["lyrics_ms","歌词"],["playlists_ms","歌单读取与同步"],["recommendations_ms","推荐"],["metadata_ms","资料补全"],["comments_ms","乐评与评论"]] as const).map(([key,label])=><label key={key}>{label}最小间隔 (ms)<input type="number" min={50} max={60000} step={50} value={value.provider_request_limits[key]} onChange={e=>setValue({...value,provider_request_limits:{...value.provider_request_limits,[key]:Number(e.target.value)}})}/></label>)}<label>上游限流冷却 (秒)<input type="number" min={1} max={3600} value={value.provider_request_limits.cooldown_seconds} onChange={e=>setValue({...value,provider_request_limits:{...value.provider_request_limits,cooldown_seconds:Number(e.target.value)}})}/></label></div><p className="fine-print">对每个连接器分别计时；等待队列优先放行播放解析和前台操作。上游返回限流时暂停请求，设置保存后生效。</p></>}
  <button className="primary" disabled={saving} onClick={()=>{setSaving(true);const {managed_memory_bytes,background_concurrency,analysis_threads,playback_cache_mode,provider_request_limits,song_fetch_concurrency}=value;const patch={mobile_opus_enabled:value.mobile_opus_enabled===true,...(value.cache_limit_bytes===undefined?{}:{cache_limit_bytes:Math.round(value.cache_limit_bytes)}),managed_memory_bytes,background_concurrency,analysis_threads,playback_cache_mode,provider_request_limits,song_fetch_concurrency};void api.patch("/settings",patch).then(()=>notify("核心资源设置已保存")).catch(onError).finally(()=>setSaving(false));}}>{saving?"正在保存…":"保存核心资源设置"}</button>
  <CacheRepair api={api} onError={onError}/>
 </section>;
}
export function BackgroundTasks({api,onError}:Omit<Props,"notify">) {
 return <section className="settings-section"><h2>后台任务</h2><TaskPage api={api} onError={onError} category="active" title="进行中" initial/><TaskPage api={api} onError={onError} category="history" title="历史记录"/></section>;
}
function TaskPage({api,onError,category,title,initial=false}:Omit<Props,"notify">&{category:string;title:string;initial?:boolean}) {
 const [open,setOpen]=useState(initial),[jobs,setJobs]=useState<Job[]>([]),[busy,setBusy]=useState(""),[page,setPage]=useState(0),[total,setTotal]=useState(0),[revision,setRevision]=useState(0);
 useEffect(()=>{if(!open)return;let active=true,pending=false;const abort=new AbortController();const load=async()=>{if(pending||document.hidden)return;pending=true;try{const result=await api.request<{items:Job[];total:number}>(`/jobs?status=${category}&limit=5&offset=${page*5}`,{signal:abort.signal});if(active){setJobs(result.items);setTotal(result.total);if(!result.items.length&&page>0)setPage(Math.max(0,Math.ceil(result.total/5)-1));}}catch(error){if(active)onError(error);}finally{pending=false;}};void load();const timer=setInterval(load,5000);return()=>{active=false;clearInterval(timer);abort.abort();};},[api,onError,open,category,page,revision]);
 const action=async(job:Job,status:string)=>{setBusy(job.id);try{await api.patch(`/jobs/${encodeURIComponent(job.id)}`,{status});setRevision(v=>v+1);}catch(error){onError(error);}finally{setBusy("");}};
 return <details open={open} onToggle={e=>setOpen(e.currentTarget.open)}><summary>{title}{open&&` · ${total.toLocaleString()}`}</summary>{open&&<>
  {jobs.length?<div className="background-jobs">{jobs.map(job=>{const ended=["completed","cancelled","failed","unable","error"].includes(job.status),bytes=["cache_fill","download"].includes(job.kind);return <article key={job.id}>
   <div className="job-heading"><strong>{kinds[job.kind]||job.kind}</strong><span>{statuses[job.status]||job.status}</span></div>
   {job.target&&<p className="job-target">{job.target}</p>}{job.phase&&<p className="muted">{job.phase}</p>}
   {job.total>0&&<progress aria-label={`${kinds[job.kind]||job.kind}进度`} value={Math.min(job.processed,job.total)} max={job.total}/>}
   <small>{bytes?formatBytes(job.processed):job.processed.toLocaleString()}{job.total>0&&` / ${bytes?formatBytes(job.total):job.total.toLocaleString()}`}</small>
   {job.error&&<details><summary>失败详情</summary><p className="error-text">{job.error}</p></details>}
   <div className="job-actions">{!ended&&<><button disabled={busy===job.id} onClick={()=>void action(job,job.status==="paused"?"running":"paused")}>{job.status==="paused"?"继续":"暂停"}</button><button disabled={busy===job.id} onClick={()=>void action(job,"cancelled")}>取消</button>{job.kind!=="cache_metadata"&&<button disabled={busy===job.id} onClick={()=>void action(job,"priority")}>优先处理</button>}</>}{["failed","unable","cancelled","error"].includes(job.status)&&<button disabled={busy===job.id} onClick={()=>void action(job,"retry")}>重试</button>}</div>
  </article>;})}</div>:<p className="muted">暂无任务</p>}
  <div className="job-actions"><button disabled={page===0} onClick={()=>setPage(v=>v-1)}>上一页</button><span>{page+1} / {Math.max(1,Math.ceil(total/5))}</span><button disabled={(page+1)*5>=total} onClick={()=>setPage(v=>v+1)}>下一页</button></div>
 </>}</details>;
}
