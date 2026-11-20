import {useEffect,useRef,useState} from 'react';
import type {RMusicClient,ArtworkCacheStatus} from '@r-music/sdk';
import {formatBytes} from './browserCache';
import {ArtworkFailureHistoryView} from './ArtworkFailureHistory';
import {errorMessage} from './errorRecovery';
const states:Record<string,string>={ready:'可用',stored:'已保存',unavailable:'平台未提供图片',not_requested:'尚未请求',missing:'文件缺失',corrupt:'文件损坏',failed:'加载失败'};
const failure=(key:string)=>({artwork_source_expired:'平台地址已失效',artwork_rate_limited:'平台限流，等待重试',artwork_budget_exceeded:'图片预算不足',artwork_corrupt:'图片文件损坏',artwork_storage_unavailable:'图片存储不可用',not_found:'文件缺失'}[key]||'暂时无法加载，可重试');
export function ArtworkMaintenance({api}:{api:RMusicClient}){
 const pending=useRef<AbortController|null>(null);
 useEffect(()=>()=>pending.current?.abort(),[api]);
 const[data,setData]=useState<ArtworkCacheStatus|null>(null),[offset,setOffset]=useState(0),[failed,setFailed]=useState(false),[revision,setRevision]=useState(0),[selected,setSelected]=useState<Set<string>>(new Set()),[budget,setBudget]=useState(512),[busy,setBusy]=useState(false),[loading,setLoading]=useState(true),[error,setError]=useState(''),[notice,setNotice]=useState('');
 useEffect(()=>{const c=new AbortController();setLoading(true);api.artworkCache(offset,failed,c.signal).then(v=>{if(!c.signal.aborted){setData(v);setBudget(v.limit_bytes/1024**2);setError('');}}).catch(e=>{if(!c.signal.aborted)setError(errorMessage(e));}).finally(()=>{if(!c.signal.aborted)setLoading(false);});return()=>c.abort();},[api,offset,failed,revision]);
 useEffect(()=>{if(!data?.jobs.some(j=>['pending','running','waiting','interrupted'].includes(j.status)))return;const timer=setTimeout(()=>{if(!document.hidden)setRevision(v=>v+1);},3000);return()=>clearTimeout(timer);},[data]);
 useEffect(()=>{setSelected(new Set());setData(null);},[api,offset,failed]);
 const pageIDs=data?.items.map(item=>item.id)||[];
 const allSelected=pageIDs.length>0&&pageIDs.every(id=>selected.has(id));
 async function run(work:(signal:AbortSignal)=>Promise<unknown>){pending.current?.abort();const c=new AbortController();pending.current=c;setBusy(true);setError('');try{await work(c.signal);if(!c.signal.aborted){setNotice('已受理，可在下方或后台任务查看进度');setRevision(v=>v+1);}}catch(e){if(!c.signal.aborted)setError(errorMessage(e));}finally{if(!c.signal.aborted)setBusy(false);}}
 return <section className="settings-section artwork-maintenance" aria-label="图片维护" aria-busy={loading}><h2>图片缓存与维护</h2>
  {loading&&!data&&<p role="status">正在读取图片…</p>}
  {error&&<p role="alert">{error}<button onClick={()=>setRevision(v=>v+1)}>重新读取</button></p>}{notice&&<p role="status">{notice}</p>}
  {data&&<><div className="artwork-metrics"><span>占用<strong>{formatBytes(data.used_bytes)}</strong></span><span>图片<strong>{data.file_count}</strong></span><span>本次发送正文<strong>{formatBytes(data.metrics.response_bytes)}</strong></span><span>本次上游正文<strong>{formatBytes(data.metrics.upstream_bytes)}</strong></span><span>内容未变响应<strong>{data.metrics.not_modified}</strong></span></div>
   <form className="group-actions" onSubmit={e=>{e.preventDefault();void run(signal=>api.request('/settings',{method:'PATCH',body:JSON.stringify({artwork_limit_bytes:Math.round(budget*1024**2)}),signal}));}}><label>独立图片预算 · MiB<input aria-label="图片预算 MiB" type="number" min={1/1024**2} max={1024**2} step="any" value={budget} onChange={e=>setBudget(Number(e.target.value))}/></label><button disabled={busy||!Number.isFinite(budget)||budget<=0}>保存图片预算</button></form>
   <p className="fine-print">已有基图和人工图片永久保留；预算不足时停止新增。检查只标记，不下载图片。</p>
   <div className="group-actions"><label><input type="checkbox" checked={failed} onChange={e=>{setFailed(e.target.checked);setOffset(0);}} disabled={busy||loading}/>仅显示异常</label><button disabled={busy||loading||!pageIDs.length} onClick={()=>setSelected(allSelected?new Set():new Set(pageIDs))}>{allSelected?'取消全选':'全选本页'}</button><button disabled={busy} onClick={()=>void run(signal=>api.inspectArtwork(signal))}>检查并标记</button><button disabled={busy||loading||!selected.size} onClick={()=>void run(signal=>api.refreshArtwork([...selected],signal))}>刷新所选 · {selected.size}</button><button disabled={busy} onClick={()=>setRevision(v=>v+1)}>更新内容</button></div>
   <div className="artwork-file-list">{data.items.map(item=><div className="artwork-file-row" key={item.id}><label><input type="checkbox" aria-label={`选择 ${item.name}`} disabled={busy||loading} checked={selected.has(item.id)} onChange={e=>setSelected(old=>{const next=new Set(old);if(e.target.checked)next.add(item.id);else next.delete(item.id);return next;})}/><strong>{item.name}</strong><small>{item.host}</small></label><span>{states[item.state]||item.state}</span>{item.error&&<small className="error">{failure(item.error)}</small>}<button disabled={busy||loading} onClick={()=>void run(signal=>api.refreshArtwork([item.id],signal))}>刷新图片</button></div>)}</div>
   {!loading&&!data.items.length&&<p role="status">{failed?'没有异常图片':'暂无图片记录'}</p>}
   <div className="pagination-controls"><button disabled={busy||loading||offset===0} onClick={()=>setOffset(v=>Math.max(0,v-50))}>上一页</button><span>{data.total} 张 · 第 {Math.floor(offset/50)+1} 页</span><button disabled={busy||loading||offset+50>=data.total} onClick={()=>setOffset(v=>v+50)}>下一页</button></div>
   <ArtworkFailureHistoryView api={api} revision={revision}/><details><summary>最近维护任务 · 最多5条</summary>{data.jobs.map(j=><p key={j.id}>{j.phase||j.kind} · {j.status} · {j.processed}{j.error&&` · ${j.error}`}</p>)}</details></>}
 </section>;
}
