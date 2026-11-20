import {useEffect,useRef,useState,type FormEvent} from "react";
import type {Playlist,RMusicClient,SmartPlaylistQuery} from "@r-music/sdk";
import {Modal} from "./components";

export function SaveSmartPlaylist({api,query,playlist,onError,onSaved}:{api:RMusicClient;query:SmartPlaylistQuery;playlist?:Pick<Playlist,"id"|"name">;onError:(error:unknown)=>void;onSaved:()=>void}) {
 const [open,setOpen]=useState(false),[name,setName]=useState(playlist?.name||""),[busy,setBusy]=useState(false);
 const pending=useRef<AbortController|null>(null);
 useEffect(()=>{pending.current=null;setBusy(false);setOpen(false);setName(playlist?.name||"");return()=>pending.current?.abort();},[api,playlist?.id]);
 async function save(event:FormEvent) {
  event.preventDefault();if(pending.current)return;
  const controller=new AbortController();pending.current=controller;setBusy(true);
  try {
   await api.request(playlist?`/playlists/${encodeURIComponent(playlist.id)}`:"/playlists",{method:playlist?"PUT":"POST",body:JSON.stringify({name,smart_query:query}),signal:controller.signal});
   if(!controller.signal.aborted){setOpen(false);setName("");onSaved();}
  }catch(error){if(!controller.signal.aborted)onError(error);}
  finally {if(pending.current===controller){pending.current=null;if(!controller.signal.aborted)setBusy(false);}}
 }
 function close(){pending.current?.abort();pending.current=null;setBusy(false);setOpen(false);}
 return <><button onClick={()=>setOpen(true)}>{playlist?"更新智能歌单":"保存为智能歌单"}</button>{open&&<Modal title={playlist?"更新智能歌单":"保存智能歌单"} onClose={close}><form onSubmit={save}>
  <p className="muted">保存当前搜索、音源、专辑和艺术家条件。播放时按最新入库曲目更新，不包含未入库的云端搜索结果。</p>
  <label>名称<input required maxLength={100} value={name} onChange={event=>setName(event.target.value)}/></label>
  <button className="primary" disabled={busy}>{busy?"正在保存…":"保存"}</button>
 </form></Modal>}</>;
}
