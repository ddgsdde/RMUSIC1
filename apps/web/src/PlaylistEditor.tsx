import {moveOrder,useTrackReorder} from "./useTrackReorder";
import {CatalogLink} from "./CatalogLinks";
import {useEffect,useState,useRef} from "react";
import type {Playlist,RMusicClient,Track} from "@r-music/sdk";
import {Modal} from "./components";
export function PlaylistEditor({api,playlist,onClose,onSaved,onError}:{api:RMusicClient;playlist:Playlist;onClose:()=>void;onSaved:()=>void;onError:(error:unknown)=>void}){
 const [name,setName]=useState(playlist.name),[ids,setIds]=useState(playlist.track_ids),[tracks,setTracks]=useState<Record<string,Track>>({}),[page,setPage]=useState(1),[query,setQuery]=useState(""),[results,setResults]=useState<Track[]>([]),[busy,setBusy]=useState(false);
 const reorder=useTrackReorder({version:JSON.stringify([playlist.id,ids]),count:ids.length,disabled:busy,onMove:(from,to)=>setIds(old=>moveOrder(old.length,from,to).map(index=>old[index]))});
 const total=Math.max(1,Math.ceil(ids.length/50)),current=Math.min(page,total),offset=(current-1)*50;
 const searchRequest=useRef<AbortController|null>(null),saveRequest=useRef<AbortController|null>(null),pageTracks=useRef<Record<string,Track>>({});
 const visibleKey=ids.slice(offset,offset+50).join("\n");
 useEffect(()=>()=>{searchRequest.current?.abort();saveRequest.current?.abort();},[api,playlist.id]);
 useEffect(()=>{
  const controller=new AbortController(),visible=visibleKey?visibleKey.split("\n"):[];
  void (async()=>{
   const page=await api.request<{items:Track[]}>(`/playlists/${encodeURIComponent(playlist.id)}/tracks?limit=50&offset=${offset}`,{signal:controller.signal});
   const next:Record<string,Track>={};for(const track of page.items)if(visible.includes(track.id))next[track.id]=track;
   for(const id of visible)if(!next[id]&&pageTracks.current[id])next[id]=pageTracks.current[id];
   const missing=[...new Set(visible.filter(id=>!next[id]))];
   for(let start=0;start<missing.length;start+=4){
    const results=await Promise.allSettled(missing.slice(start,start+4).map(id=>api.request<Track>(`/library/tracks/${encodeURIComponent(id)}`,{signal:controller.signal})));
    controller.signal.throwIfAborted();for(const result of results)if(result.status==='fulfilled')next[result.value.id]=result.value;
   }
   if(!controller.signal.aborted){pageTracks.current=next;setTracks(next);}
  })().catch(error=>{if(!controller.signal.aborted)onError(error);});return()=>controller.abort();
 },[api,playlist.id,offset,visibleKey,onError]);
 const move=(index:number,direction:number)=>setIds(old=>{const next=[...old],target=index+direction;if(target>=0&&target<next.length)[next[index],next[target]]=[next[target],next[index]];return next;});
 return <Modal title={`编辑歌单 · ${playlist.name}`} onClose={onClose} wide><form onSubmit={e=>{e.preventDefault();setBusy(true);const controller=new AbortController();saveRequest.current=controller;void api.request(`/playlists/${encodeURIComponent(playlist.id)}`,{method:'PUT',signal:controller.signal,body:JSON.stringify({name,track_ids:ids,expected:{name:playlist.name,track_ids:playlist.track_ids}})}).then(()=>{if(!controller.signal.aborted)onSaved();}).catch(error=>{if(!controller.signal.aborted)onError(error);}).finally(()=>{if(!controller.signal.aborted)setBusy(false);});}}>
  <label>名称<input value={name} required maxLength={100} onChange={e=>setName(e.target.value)}/></label>
  {playlist.sync&&<p className="fine-print">本地修改会自动排队同步。云端不支持的重命名、顺序或本地歌曲会明确标记为未同步。</p>}
  <div className="playlist-edit-tracks" ref={reorder.container}>{ids.slice(offset,offset+50).map((id,index)=><div key={`${id}-${offset+index}`} {...reorder.rowProps(offset+index)}>{reorder.handle(offset+index,tracks[id]?.title||id)}<span>{offset+index+1}</span><strong>{tracks[id]?.title||id}<small>{tracks[id]?.artist}</small></strong><button type="button" aria-label={`上移第 ${offset+index+1} 首`} disabled={offset+index===0} onClick={()=>move(offset+index,-1)}>上移</button><button type="button" aria-label={`下移第 ${offset+index+1} 首`} disabled={offset+index===ids.length-1} onClick={()=>move(offset+index,1)}>下移</button><button type="button" onClick={()=>setIds(old=>old.filter((_,i)=>i!==offset+index))}>移除</button></div>)}</div>
  <div className="pagination-controls"><button type="button" disabled={current===1} onClick={()=>setPage(current-1)}>上一页</button><span>第 {current} / {total} 页 · {ids.length} 首</span><button type="button" disabled={current===total} onClick={()=>setPage(current+1)}>下一页</button></div>
  <label>从资料库添加歌曲<input value={query} onChange={e=>setQuery(e.target.value)} placeholder="歌曲或艺术家"/></label><button type="button" disabled={!query.trim()||busy} onClick={()=>{searchRequest.current?.abort();const controller=new AbortController();searchRequest.current=controller;setBusy(true);void api.request<{items:Track[]}>(`/library/tracks?q=${encodeURIComponent(query)}&limit=20`,{signal:controller.signal}).then(value=>{if(!controller.signal.aborted)setResults(value.items);}).catch(error=>{if(!controller.signal.aborted)onError(error);}).finally(()=>{if(!controller.signal.aborted)setBusy(false);});}}>搜索资料库</button>
  {results.length>0&&<div className="playlist-search-results">{results.map(track=><button type="button" key={track.id} onClick={()=>{setIds(old=>[...old,track.id]);if(ids.length-offset<50)setTracks(old=>({...old,[track.id]:track}));}}>{track.title} · <CatalogLink track={track}/><span>添加</span></button>)}</div>}
  <div className="form-actions"><button type="button" onClick={onClose}>取消</button><button className="primary" disabled={busy}>保存歌单</button></div>
 </form></Modal>;
}
