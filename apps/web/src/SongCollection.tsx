import {useSimilarSongs} from "./PersonalRecommendations";
import {HeartButton} from "./HeartButton";
import {useTrackContextMenu} from "./trackMenuContext";
import {disposeArtwork} from "./artworkCache";
import {createContext,useContext,useEffect,useRef,useState,type ReactNode} from "react";
import {ListPlus,Plus,ListStart,MoreHorizontal} from "lucide-react";
import type {Playlist,ProviderManifest,ProviderPlaylist,RMusicClient,Track} from "@r-music/sdk";
import {Cover,IconButton,Modal} from "./components";
type Cloud=ProviderPlaylist&{account_context:string};
export function sourceTrack(track:Track):Track {const origin=track.provider_data?.cached_from as {track_id?:unknown;source?:unknown}|undefined;return typeof origin?.track_id==="string"&&typeof origin.source==="string"?{...track,id:origin.track_id,source:origin.source}:track;}
const Collection=createContext<{liked:Set<string>;busy:Set<string>;like:(track:Track)=>Promise<void>;add:(track:Track)=>void;playNext?:(track:Track)=>void}|null>(null);
export function SongMenuButton({track,current=false,remove}:{track?:Track|null;current?:boolean;remove?:()=>Promise<void>}){
 const open=useTrackContextMenu();return track&&open?<IconButton className="icon-button song-options song-menu-trigger" label={`更多歌曲操作 ${track.title}`} aria-haspopup="menu" onClick={event=>open(event,track,remove,{current})}><MoreHorizontal size={19}/></IconButton>:null;
}
export function SongButtons({track,mode="all",current=false,remove}:{track?:Track|null;mode?:"all"|"like"|"playlist";current?:boolean;remove?:()=>Promise<void>}){
 const collection=useContext(Collection);if(!collection||!track)return null;
 const original=sourceTrack(track),liked=collection.liked.has(original.id)||collection.liked.has(track.id),busy=collection.busy.has(original.id);
 return <>{mode!=="playlist"&&<HeartButton key={track.id} liked={liked} busy={busy} title={track.title} toggle={()=>collection.like(original)}/>}{mode!=="like"&&(mode==="playlist"?<IconButton label={`添加 ${track.title} 到歌单`} onClick={()=>collection.add(track)}><ListPlus size={19}/></IconButton>:<SongMenuButton track={track} current={current} remove={remove}/>)}</>;
}
export function SongCollectionProvider({api,plugins,notify,onError,playNext,children}:{api:RMusicClient;plugins:ProviderManifest[];notify:(message:string)=>void;onError:(error:unknown)=>void;children:ReactNode;playNext?:(track:Track)=>void;enqueue?:(track:Track)=>void}){
 const [items,setItems]=useState<Playlist[]>([]),[cloud,setCloud]=useState<Cloud[]>([]),[favorite,setFavorite]=useState<Playlist|null>(null),[selected,setSelected]=useState<Track|null>(null),[name,setName]=useState("");
 const [busy,setBusy]=useState(new Set<string>()),[saving,setSaving]=useState(false);const current=useRef<RMusicClient|null>(api);current.current=api;
 useEffect(()=>{
  current.current=api;
  return ()=>{if(current.current===api)current.current=null;disposeArtwork(api);};
 },[api]);
 const similar=useSimilarSongs();
 const pendingRefresh=useRef<Promise<void>|null>(null),refreshAgain=useRef(false);
 const refresh=()=>{
  if(pendingRefresh.current){refreshAgain.current=true;return pendingRefresh.current;}
  const owner=api;
  const task=(async()=>{
   do{
    refreshAgain.current=false;
    const [local,remote,fav]=await Promise.allSettled([api.get<{items:Playlist[]}>("/playlists"),api.get<{items:Cloud[]}>("/playlist-directory"),api.get<Playlist>("/favorites")]);
    if(current.current!==owner)return;
    if(refreshAgain.current)continue;
    if(local.status==="fulfilled")setItems(local.value.items);
    if(remote.status==="fulfilled")setCloud(remote.value.items);
    if(fav.status==="fulfilled")setFavorite(fav.value);
   }while(refreshAgain.current);
  })().finally(()=>{if(pendingRefresh.current===task)pendingRefresh.current=null;});
  pendingRefresh.current=task;return task;
 };
 useEffect(()=>{pendingRefresh.current=null;refreshAgain.current=false;setItems([]);setCloud([]);setFavorite(null);setSelected(null);void refresh().catch(()=>{});},[api]);
 const likedIds=new Set(favorite?.alias_track_ids||favorite?.track_ids||[]);
 useEffect(()=>{const changed=(event:Event)=>{if((event as CustomEvent).detail===api)void refresh().catch(()=>{});};window.addEventListener("rmusic-playlists-changed",changed);const timer=setInterval(()=>{if(!document.hidden)void refresh().catch(()=>{});},60000);const visible=()=>{if(!document.hidden)void refresh().catch(()=>{});};document.addEventListener("visibilitychange",visible);window.addEventListener("rmusic-activity-visibility",visible);window.addEventListener("focus",visible);return()=>{clearInterval(timer);window.removeEventListener("rmusic-playlists-changed",changed);document.removeEventListener("visibilitychange",visible);window.removeEventListener("rmusic-activity-visibility",visible);window.removeEventListener("focus",visible);};},[api]);
 async function addTo(track:Track,target:{local?:Playlist;remote?:Cloud},remove=false){
  let id=target.local?.id;
  if(!id&&target.remote){id=(await api.post<{id:string}>("/playlists/bind",{provider:target.remote.source,playlist_id:target.remote.id,account_context:target.remote.account_context})).id;}
  if(!id)throw new Error("请选择目标歌单");
  const original=sourceTrack(track);const provider=target.remote?.source||target.local?.sync?.provider;
  await api.post(`/playlists/${encodeURIComponent(id)}/tracks`,{track_id:provider===original.source?original.id:track.id,remove});
 }
 async function like(track:Track){
  if(busy.has(track.id))return;setBusy(old=>new Set(old).add(track.id));
  const liked=!likedIds.has(track.id);
  try {
   const saved=await api.request<Playlist>(`/favorites/${encodeURIComponent(track.id)}`,{method:"PUT",body:JSON.stringify({liked})});
   if(current.current!==api)return;setFavorite(saved);
   if(liked)similar?.find(track,true);else similar?.unliked(track);
   notify(saved.sync&&saved.sync.provider===track.source?`${liked?"已喜欢":"已取消喜欢"}，云端同步已排队`:liked?"已加入本地我喜欢":"已取消本地喜欢");
   void refresh();
  }catch(error){onError(error);}finally{if(current.current===api)setBusy(old=>{const next=new Set(old);next.delete(track.id);return next;});}
 }
 async function choose(target:{local?:Playlist;remote?:Cloud}){
  if(!selected||saving)return;setSaving(true);
  try{await addTo(selected,target);notify(target.remote||target.local?.sync?"已添加，同步任务已排队":"已添加到歌单");setSelected(null);await refresh();}catch(error){onError(error);}finally{setSaving(false);}
 }
 const targets=items.filter(item=>!item.smart_query&&(!item.sync||item.sync.capabilities.includes("add")));
 const remotes=cloud.filter(item=>item.writable&&item.write_capabilities?.includes("add")&&!items.some(local=>local.sync?.provider===item.source&&local.sync.playlist_id===item.id&&local.sync.account_context===item.account_context));
 return <Collection.Provider value={{liked:likedIds,busy,playNext,like:track=>like(sourceTrack(track)),add:track=>{setSelected(track);setName("");void refresh().catch(onError);}}}>{children}
  {selected&&<Modal title="添加到歌单" onClose={()=>!saving&&setSelected(null)}><div className="collection-song"><Cover api={api} track={selected}/><div><strong>{selected.title}</strong><p>{selected.artist}</p></div></div><div className="collection-targets">
   {targets.map(item=><button disabled={saving} key={item.id} onClick={()=>void choose({local:item})}><Cover api={api} track={{cover_url:item.cover_url,title:item.name,album:item.name}}/><span><strong>{item.name}</strong><small>{item.sync?plugins.find(p=>p.id===item.sync?.provider)?.name:"本地"} · {item.track_ids.length} 首</small></span><Plus size={17}/></button>)}
   {remotes.map(item=><button disabled={saving||item.source!==sourceTrack(selected).source} key={`${item.source}:${item.id}`} title={item.source!==sourceTrack(selected).source?"此歌单只支持相同音源的歌曲":undefined} onClick={()=>void choose({remote:item})}><Cover api={api} track={{cover_url:item.cover_url,title:item.name,album:item.name}}/><span><strong>{item.name}</strong><small>{plugins.find(p=>p.id===item.source)?.name} · {item.track_count} 首</small></span><Plus size={17}/></button>)}
  </div><form className="collection-create" onSubmit={async event=>{event.preventDefault();if(!name.trim()||saving)return;setSaving(true);try{await api.post("/playlists",{name:name.trim(),track_ids:[selected.id]});notify("已创建歌单并添加歌曲");setSelected(null);await refresh();}catch(error){onError(error);}finally{setSaving(false);}}}><input aria-label="新歌单名称" placeholder="新歌单名称" maxLength={100} value={name} onChange={e=>setName(e.target.value)}/><button disabled={!name.trim()||saving}><Plus size={16}/>新建并添加</button></form></Modal>}
 </Collection.Provider>;
}

export function PlayNextButton({track}:{track?:Track|null}){const actions=useContext(Collection);return track&&actions?.playNext?<button type="button" disabled={!track.available} onClick={()=>actions.playNext?.(track)}><ListStart size={18}/>下一首播放</button>:null;}
export function useSongCollectionActions(){return useContext(Collection);}
