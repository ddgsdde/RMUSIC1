import {useTrackSort} from "../TrackSort";
import {CatalogLink} from "../CatalogLinks";
import {useEffect,useRef,useState} from "react";
import type {ProviderManifest,RMusicClient,SmartPlaylistQuery,Track,SmartPlaylistPreview as Preview} from "@r-music/sdk";
import {Cover,quality} from "../components";
import {errorMessage} from "../errorRecovery";
import {SmartPlaylistEditor} from "./SmartPlaylistEditor";

const collections:{id:string;title:string;rule:SmartPlaylistQuery}[]=[
  {id:"unheard",title:"还没听过",rule:{max_plays:0,sort:"recent_added",max_results:100}},
  {id:"rediscover",title:"久违的音乐",rule:{min_plays:1,not_played_days:30,sort:"least_played",max_results:100}},
  {id:"hires",title:"高解析曲库",rule:{min_sample_rate:88200,min_bit_depth:24,max_results:100}},
  {id:"favorites",title:"我的收藏",rule:{favorites_only:true,sort:"recent",max_results:100}},
];
export function DiscoveryCollections({api,plugins,play,showDetails}:{api:RMusicClient;plugins:ProviderManifest[];play:(t:Track,list?:Track[])=>Promise<void>;showDetails:(t:Track)=>void}) {
  const [selected,setSelected]=useState<(typeof collections)[number]|null>(null),[result,setResult]=useState<Preview|null>(null);
  const [error,setError]=useState(""),[notice,setNotice]=useState(""),[busy,setBusy]=useState(false),[shown,setShown]=useState(6),[saving,setSaving]=useState(false);
  const sorted=useTrackSort(result?.items||[],"精选排序");
  const pending=useRef<AbortController|null>(null);
  useEffect(()=>{setResult(null);setSelected(null);return()=>pending.current?.abort();},[api]);
  async function load(collection:(typeof collections)[number],offset=0) {
    pending.current?.abort();const c=new AbortController();pending.current=c;setSelected(collection);setBusy(true);setError("");setShown(6);
    try{const result=await api.request<Preview>("/smart-playlists/preview",{method:"POST",body:JSON.stringify({rule:collection.rule,offset}),signal:c.signal});if(!c.signal.aborted)setResult(result);}
    catch(e){if(!c.signal.aborted)setError(errorMessage(e));}finally{if(!c.signal.aborted)setBusy(false);}
  }
  return <section className="discovery-section discovery-collections">
    <div className="section-heading"><h2>探索曲库</h2>{selected&&result&&<button disabled={busy} onClick={()=>setSaving(true)}>保存为智能歌单</button>}</div>
    <div className="source-tabs" aria-label="曲库发现分类">{collections.map(c=><button key={c.id} disabled={busy&&selected?.id===c.id} aria-pressed={selected?.id===c.id} onClick={()=>{setResult(null);void load(c);}}>{c.title}</button>)}</div>
    {error&&<p role="alert">{error}{selected&&<button onClick={()=>void load(selected,result?.offset||0)}>重试精选</button>}</p>}{notice&&<p role="status">{notice}</p>}
    {busy&&<p role="status">正在挑选曲库中的音乐…</p>}
    {result&&!busy&&<>
      <p className="muted">{result.total} 首{result.truncated?" · 按规则选取前 100 首":""} · 聆听条件使用共享 Core 统计</p>
      {!result.total&&<p>当前没有匹配的歌曲，可以尝试其他分类。</p>}
      {sorted.control}<div className="recommendation-rail">{sorted.tracks.slice(0,shown).map(t=><article className="recommendation-album" key={t.id}><button className="recommendation-cover" aria-label={`播放曲库精选 ${t.title}`} onClick={()=>void play(t,sorted.tracks)}><Cover api={api} track={t}/></button><button className="recommendation-title" onClick={()=>showDetails(t)}>{t.title}</button><p><CatalogLink track={t}/></p><small>{quality(t)}</small></article>)}</div>
      <div className="group-actions">{shown<result.items.length&&<button onClick={()=>setShown(v=>v+6)}>继续显示</button>}{result.offset>0&&<button onClick={()=>void load(selected!,Math.max(0,result.offset-30))}>上一批</button>}{result.offset+30<result.total&&<button onClick={()=>void load(selected!,result.offset+30)}>下一批</button>}</div>
    </>}
    {saving&&selected&&<SmartPlaylistEditor api={api} providers={plugins} initialRule={selected.rule} initialName={selected.title} close={()=>setSaving(false)} saved={()=>{setSaving(false);setNotice("已保存，后续曲库变化会自动反映到歌单。");window.dispatchEvent(new CustomEvent("rmusic-playlists-changed",{detail:api}));}}/>}
  </section>;
}
