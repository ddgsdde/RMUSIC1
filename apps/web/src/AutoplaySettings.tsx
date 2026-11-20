import {useEffect,useState} from "react";
import {Radio} from "lucide-react";
import type {AutoplayConfig,RMusicClient,Track,Zone} from "@r-music/sdk";

type Preview={items:Track[];status?:string;reasons?:Record<string,string>};

export function AutoplaySettings({api,zone,refresh,onError}:{
  api:RMusicClient;zone:Zone;refresh:()=>Promise<unknown>;onError:(error:unknown)=>void;
}){
  const [saving,setSaving]=useState(false);
  const [open,setOpen]=useState(false),[preview,setPreview]=useState<Preview>();
  const path="/zones/"+encodeURIComponent(zone.id)+"/radio";
  const config:AutoplayConfig=zone.autoplay??{enabled:true,source:"mixed",fallback_local:true};
  useEffect(()=>{
    setPreview(undefined);
    if(!open||!config.enabled)return;
    let active=true;
    const load=()=>void api.get<Preview>(path).then(result=>{if(active)setPreview(result);}).catch(()=>{});
    load();
    window.addEventListener("rmusic-radio-ready",load);
    return()=>{active=false;window.removeEventListener("rmusic-radio-ready",load);};
  },[api,path,open,config.enabled,zone.playback_id,zone.queue.length]);
  async function save(enabled:boolean){
    setSaving(true);
    try{
      await api.patch("/zones/"+encodeURIComponent(zone.id),{autoplay:{...config,enabled}});
      await refresh();
    }catch(error){onError(error);}
    finally{setSaving(false);}
  }
  const next=preview?.items[0];
  return <section className="queue-radio">
    <label className="queue-radio-switch">
      <Radio size={18} aria-hidden="true"/>
      <span><strong>队列电台</strong><small>队列播完后继续播放相似歌曲</small></span>
      <input className="toggle" type="checkbox" aria-label="开启队列电台" checked={config.enabled} disabled={saving} onChange={e=>void save(e.target.checked)}/>
    </label>
    {config.enabled&&<details open={open} onToggle={event=>setOpen(event.currentTarget.open)}>
      <summary>续播推荐</summary>
      {next?<div className="radio-next">
        <span><strong>{next.title}</strong><small>{next.source==="local"?"本地":"云端"} · {preview?.reasons?.[next.id]||"延续队列风格"}</small></span>
        <button type="button" onClick={()=>void api.post<Preview>(path,{track_id:next.id}).then(setPreview).catch(onError)}>跳过</button>
      </div>:<p className="muted" role="status">{preview?.status==="waiting"||preview?.status==="unavailable"?"暂无可播候选，等待电台准备。":preview?.status==="preparing"?"正在准备下一批歌曲…":"接近队列末尾时准备相似歌曲。循环和手动加入的歌曲优先。"}</p>}
    </details>}
  </section>;
}
