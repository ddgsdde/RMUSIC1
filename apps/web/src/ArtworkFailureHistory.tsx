import {useEffect,useState} from 'react';
import type {ArtworkFailureHistory,RMusicClient} from '@r-music/sdk';
import {errorMessage} from './errorRecovery';
const stage:Record<string,string>={upstream:'平台请求',decode:'图片解码',storage:'文件保存'};
export function ArtworkFailureHistoryView({api,revision}:{api:RMusicClient;revision:number}) {
 const[open,setOpen]=useState(false),[data,setData]=useState<ArtworkFailureHistory>(),[error,setError]=useState('');
 useEffect(()=>{if(!open)return;const c=new AbortController();api.artworkFailureHistory(undefined,c.signal).then(v=>{if(!c.signal.aborted){setData(v);setError('');}}).catch(e=>{if(!c.signal.aborted)setError(errorMessage(e));});return()=>c.abort();},[api,revision,open]);
 return <details open={open} onToggle={e=>setOpen(e.currentTarget.open)}><summary>最近图片失败 · 最多50条</summary>{error&&<p role="alert">{error}</p>}{data&&!data.items.length&&<p className="muted">暂无失败记录</p>}{data?.items.map((item,i)=><p className="fine-print" key={i}>{new Date(item.created_at*1000).toLocaleString()} · {stage[item.stage]||item.stage} · {item.code} · HTTP {item.status} · {item.elapsed_ms} ms · {item.source_id.slice(0,10)}</p>)}</details>;
}
