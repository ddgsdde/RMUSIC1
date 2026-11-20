import {useTrackSort} from "../TrackSort";
import {CatalogLink} from "../CatalogLinks";
import {useEffect,useRef,useState} from "react";
import type {RMusicClient,Track,LlmRecommendations as Result} from "@r-music/sdk";
import {Cover} from "../components";
import {errorMessage} from "../errorRecovery";
import {clientId} from "../clientId";
export function LlmPicks({api,candidates,play,goSettings}:{api:RMusicClient;candidates:Track[];play:(t:Track,list?:Track[])=>Promise<void>;goSettings:()=>void}) {
  const [open,setOpen]=useState(false),[preference,setPreference]=useState(""),[result,setResult]=useState<Result|null>(null);
  const [error,setError]=useState(""),[busy,setBusy]=useState(false),[name,setName]=useState("LLM 精选"),[notice,setNotice]=useState("");
  const sorted=useTrackSort(result?.items||[],"精选排序");
  const pending=useRef<AbortController|null>(null),intent=useRef(clientId());
  useEffect(()=>{setResult(null);setError("");return()=>pending.current?.abort();},[api]);
  return <section className="discovery-section llm-picks">
    <div className="section-heading"><h2>按心情挑选</h2><button onClick={()=>{setOpen(v=>!v);if(open){pending.current?.abort();setBusy(false);}}}>{open?"收起":"可选 LLM 推荐"}</button></div>
    {open&&<>
      <p className="fine-print">会向 Core 中配置的服务发送当前候选的歌曲资料和下方偏好，仅对可核验的歌曲排序。<button className="text-button" onClick={goSettings}>Core 设置</button></p>
      <form className="llm-request" onSubmit={async e=>{e.preventDefault();pending.current?.abort();const c=new AbortController();pending.current=c;setBusy(true);setError("");try{const value=await api.request<Result>("/llm/recommend",{method:"POST",signal:c.signal,body:JSON.stringify({track_ids:candidates.slice(0,50).map(t=>t.id),preference})});if(!c.signal.aborted){setResult(value);intent.current=clientId();setNotice("");}}catch(e){if(!c.signal.aborted)setError(errorMessage(e));}finally{if(!c.signal.aborted)setBusy(false);}}}>
        <label>这次想听什么<input disabled={busy} maxLength={1000} placeholder="例如：晚上安静听，优先器乐" value={preference} onChange={e=>setPreference(e.target.value)}/></label><button disabled={busy||!candidates.length}>{busy?"正在挑选…":"使用 LLM 挑选"}</button>
        {busy&&<button type="button" onClick={()=>{pending.current?.abort();setBusy(false);}}>取消请求</button>}
      </form>
      {error&&<p role="alert">{error} · 原推荐保留。</p>}{notice&&<p role="status">{notice}</p>}
      {result&&<><p className="muted">模型 {result.model}{result.ignored?` · 已忽略 ${result.ignored} 个无效或重复引用`:""}</p>
        {sorted.control}<div className="recommendation-rail">{sorted.tracks.slice(0,12).map(t=><article className="recommendation-album" key={t.id}><button className="recommendation-cover" onClick={()=>void play(t,sorted.tracks)} aria-label={`播放 LLM 推荐 ${t.title}`}><Cover api={api} track={t}/></button><strong>{t.title}</strong><p><CatalogLink track={t}/></p><small>{result.reasons[t.id]}</small></article>)}</div>
        <form className="llm-request" onSubmit={async e=>{e.preventDefault();const c=new AbortController();pending.current=c;setBusy(true);setError("");try{await api.request("/playlists/import",{method:"POST",signal:c.signal,body:JSON.stringify({name,track_ids:sorted.tracks.map(t=>t.id),request_id:intent.current})});if(!c.signal.aborted){setNotice("已保存为本地播放列表");window.dispatchEvent(new CustomEvent("rmusic-playlists-changed",{detail:api}));}}catch(e){if(!c.signal.aborted)setError(errorMessage(e));}finally{if(!c.signal.aborted)setBusy(false);}}}><label>保存为歌单<input disabled={busy} required maxLength={100} value={name} onChange={e=>{setName(e.target.value);intent.current=clientId();}}/></label><button disabled={busy||!name.trim()}>保存精选</button></form>
      </>}
    </>}
  </section>;
}
