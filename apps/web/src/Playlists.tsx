import {SmartPlaylistEditor} from "./music/SmartPlaylistEditor";
import { useEffect, useRef, useState } from "react";
import { ListMusic, Pencil, Play, Plus, Trash2, RefreshCw, LayoutGrid, List, MoreHorizontal } from "lucide-react";
import { ApiError, type Playlist, type ProviderManifest, type ProviderPlaylist, type RMusicClient, type Track, type Zone } from "@r-music/sdk";
import { Cover, Empty, IconButton, Modal } from "./components";
import {PlaylistEditor} from "./PlaylistEditor";
import {LocalPlaylistView} from "./LocalPlaylistView";
import { ProviderPlaylistsView } from "./ProviderPlaylists";
import {PlaylistImport,PlaylistExport} from "./PlaylistExchange";
import {FavoritesBinding,PlaylistConflictResolution} from "./FavoritesBinding";
import {isNative} from "./native";
export function PlaylistsView({
  api,
  zone,
  refreshZones,
  onError,
  notify,
  plugins,
  play: playTrack,
  enqueue,
  goSettings,
  favoritesOnly=false,
  backToPlaylists,
}: {
  api: RMusicClient;
  zone?: Zone;
  refreshZones: () => Promise<unknown>;
  onError: (error: unknown) => void;
  notify: (message: string) => void;
  plugins: ProviderManifest[];
  play: (track: Track, list?: Track[]) => Promise<void>;
  enqueue: (track: Track) => Promise<void>;
  goSettings: () => void;
  favoritesOnly?:boolean;
  backToPlaylists?:()=>void;
  editSmart?: (playlist:Playlist) => void;
}) {
  const [smartEditor,setSmartEditor]=useState<Playlist|true|null>(null);
  const [source, setSource] = useState("all");
  const [view,setView]=useState(()=>localStorage.getItem("rmusic.playlists.view")==="list"?"list":"grid");
  const [selectedLocal,setSelectedLocal]=useState<Playlist|null>(null);
  const providers = plugins.filter(
    (item) =>
      item.enabled &&
      item.capabilities.some((capability) =>
        ["playlists", "favorites"].includes(capability),
      ),
  );
  const [items, setItems] = useState<Playlist[]>([]);
  const [favorite,setFavorite]=useState<Playlist>();
  const [cloud,setCloud]=useState<(ProviderPlaylist&{account_context:string})[]>([]),[selectedCloud,setSelectedCloud]=useState<{provider:ProviderManifest;item:ProviderPlaylist}|null>(null),[editing,setEditing]=useState<Playlist|null>(null),[deleting,setDeleting]=useState<Playlist|null>(null),[conflict,setConflict]=useState<Playlist|null>(null),[busy,setBusy]=useState("");
  const [importing,setImporting]=useState(false);
  const [adding, setAdding] = useState(false);
  const [favoritesBinding,setFavoritesBinding]=useState(false);
  const [name, setName] = useState("");
  const [includeQueue, setIncludeQueue] = useState(true);
  const pendingPlay=useRef<AbortController|null>(null);
  useEffect(()=>()=>{pendingPlay.current?.abort();pendingPlay.current=null;},[api]);
  const refresh=async()=>{const [local,remote]=await Promise.all([api.get<{items:Playlist[]}>("/playlists"),api.get<{items:(ProviderPlaylist&{account_context:string})[]}>("/playlist-directory")]);setItems(local.items);setCloud(remote.items);};
  useEffect(()=>{
    let active=true,pending=false,again=false;
    const load=async()=>{
      if(document.hidden)return;
      if(pending){again=true;return;}
      pending=true;
      try{do{
        again=false;
        if(favoritesOnly){
          const current=await api.get<Playlist>("/favorites");
          if(active&&!again)setFavorite(current);
        }else{
          const [local,remote]=await Promise.all([api.get<{items:Playlist[]}>("/playlists"),api.get<{items:(ProviderPlaylist&{account_context:string})[]}>("/playlist-directory")]);
          if(!active)return;
          if(again)continue;
          setItems(local.items);setCloud(remote.items);
          setSelectedLocal(current=>current?local.items.find(item=>item.id===current.id)||null:null);
        }
      }while(again);}catch(error){if(active)onError(error);}finally{pending=false;}
    };
    void load();
    const changed=(event:Event)=>{if((event as CustomEvent).detail===api)void load();},wake=()=>{void load();};
    window.addEventListener("rmusic-playlists-changed",changed);window.addEventListener("focus",wake);window.addEventListener("rmusic-activity-visibility",wake);document.addEventListener("visibilitychange",wake);
    const timer=setInterval(load,60000);
    return()=>{active=false;clearInterval(timer);window.removeEventListener("rmusic-playlists-changed",changed);window.removeEventListener("focus",wake);window.removeEventListener("rmusic-activity-visibility",wake);document.removeEventListener("visibilitychange",wake);};
  },[api,onError,favoritesOnly]);
  const sync=async(item:Playlist,body:unknown={})=>{setBusy(item.id);try{await api.post(`/playlists/${encodeURIComponent(item.id)}/sync`,body);await refresh();notify("同步任务已排队，可在核心后台任务中查看");}catch(error){onError(error);}finally{setBusy("");}};
  async function resolveConflict(item:Playlist,resolve:"cloud"|"local"){
    setBusy(item.id);
    const path=`/playlists/${encodeURIComponent(item.id)}`;
    try{
      await api.post(path+"/sync",{resolve,conflict_id:item.sync?.conflict?.id});
      setConflict(null);await refresh();notify("同步任务已排队");
    }catch(error){
      if(error instanceof ApiError&&error.code==="playlist_conflict_changed"){
        await api.get<Playlist>(path).then(current=>setConflict(current.sync?.conflict?current:null)).catch(onError);
      }
      onError(error);
    }finally{setBusy("");}
  }
  const bind=async(item:ProviderPlaylist)=>{setBusy(`${item.source}:${item.id}`);try{await api.post("/playlists/bind",{provider:item.source,playlist_id:item.id,account_context:item.account_context});await refresh();notify("已安排歌单同步，不会自动下载整张歌单");}catch(error){onError(error);}finally{setBusy("");}};
  async function play(item: Playlist) {
    if (!zone) {
      notify("请先选择一个音区");
      return;
    }
    if(pendingPlay.current)return;
    const controller=new AbortController();pendingPlay.current=controller;
    try {
      const current=item.smart_query?await api.request<Playlist>(`/playlists/${encodeURIComponent(item.id)}`,{signal:controller.signal}):item;
      if(controller.signal.aborted)return;
      if(!current.track_ids.length){notify(item.smart_query?"智能歌单当前没有匹配的歌曲":"歌单中还没有歌曲");return;}
      if(current.truncated)notify("匹配超过一万首，将播放前一万首");
      await api.request(`/zones/${zone.id}/queue`,{method:"PUT",body:JSON.stringify({track_ids:current.track_ids}),signal:controller.signal});
      if(controller.signal.aborted)return;
      await api.request(`/zones/${zone.id}/play`,{method:"POST",body:JSON.stringify({queue_index:0}),signal:controller.signal});
      if(!controller.signal.aborted)await refreshZones();
    } catch (error) {
      if(!controller.signal.aborted)onError(error);
    }finally{if(pendingPlay.current===controller)pendingPlay.current=null;}
  }
  const providerName=(id:string)=>providers.find(p=>p.id===id)?.name||id;
  const sourcePicker=<div className="source-tabs playlist-sources"><button className={source==="all"?"selected":""} onClick={()=>setSource("all")}>全部歌单</button><button className={source==="local"?"selected":""} onClick={()=>setSource("local")}>本地</button>{providers.map(p=><button key={p.id} className={source===p.id?"selected":""} onClick={()=>setSource(p.id)}>{p.name}</button>)}</div>;
  if(favoritesOnly)return favorite?<LocalPlaylistView api={api} playlist={favorite} back={backToPlaylists??(()=>{})} play={playTrack} enqueue={enqueue} showError={onError} playAll={()=>void play(favorite)} onChanged={setFavorite}/>:<p className="muted" role="status">正在读取我喜欢</p>;
  if(selectedLocal)return <LocalPlaylistView onChanged={saved=>{setSelectedLocal(saved);setItems(current=>current.map(item=>item.id===saved.id?{...item,...saved}:item));}} api={api} playlist={selectedLocal} back={()=>setSelectedLocal(null)} play={playTrack} enqueue={enqueue} showError={onError} playAll={()=>void play(selectedLocal)}/>;
  if(selectedCloud)return <ProviderPlaylistsView api={api} provider={selectedCloud.provider} initialPlaylist={selectedCloud.item} onBack={()=>setSelectedCloud(null)} zone={zone} play={playTrack} enqueue={enqueue} refreshZones={refreshZones} onError={onError} notify={notify} goSettings={goSettings} onImported={()=>{setSelectedCloud(null);void refresh();}}/>;
  const localItems=items.filter(item=>source==="all"||source==="local"||item.sync?.provider===source);
  const cloudItems=cloud.filter(item=>(source==="all"||source===item.source)&&!items.some(local=>local.sync?.provider===item.source&&local.sync.playlist_id===item.id&&local.sync.account_context===item.account_context));
  const syncLabels:Record<string,string>={pending:"等待同步",synced:"已同步",partial:"部分未同步",conflict:"需要处理冲突",failed:"同步失败"};
  return (
    <div className="playlists-view">
      <div className="playlist-toolbar">{sourcePicker}
      <div className="playlist-toolbar-actions">
        <button onClick={()=>setImporting(true)}>导入播放列表</button>
        <button onClick={()=>setSmartEditor(true)}>创建智能歌单</button>
        <button onClick={()=>setFavoritesBinding(true)}>我喜欢 · 云端绑定</button>
        <div className="playlist-view-options" role="group" aria-label="歌单显示方式">{[{id:"grid",icon:LayoutGrid,label:"卡片视图"},{id:"list",icon:List,label:"列表视图"}].map(({id,icon:Icon,label})=><IconButton key={id} label={label} aria-pressed={view===id} onClick={()=>{setView(id);localStorage.setItem("rmusic.playlists.view",id);}}><Icon size={18}/></IconButton>)}</div>
        <IconButton label="刷新云端歌单目录" onClick={()=>void api.post("/playlist-directory/refresh",{}).then(()=>notify("云端目录刷新已排队")).catch(onError)}><RefreshCw size={16}/></IconButton>
        <button onClick={() => setAdding(true)}>
          <Plus size={17} />
          创建列表
        </button>
      </div>
      </div>
      {localItems.length||cloudItems.length ? (
        <div className={`playlist-zone-list playlist-${view}`}>
          {localItems.map((item) => (
            <div className="playlist-row" key={item.id}>
              <button className="playlist-art" aria-label={`打开歌单 ${item.name}`} onClick={()=>setSelectedLocal(item)}><Cover api={api} track={{cover_url:item.cover_url,title:item.name,album:item.name}}/></button>
              <div className="playlist-summary">
                <button className="playlist-name-button" onClick={()=>setSelectedLocal(item)}><strong>{item.name}</strong></button>
                {item.sync&&<small>{providerName(item.sync.provider)} · {syncLabels[item.sync.status]||item.sync.status}{!item.sync.capabilities.length?" · 云端只读":""}</small>}
                <span>{item.smart_query?"智能歌单 · 播放时按最新曲库更新":`${item.track_ids?.length || 0} 首歌曲`}</span>
              </div>
              <IconButton
                label={`播放 ${item.name}`}
                disabled={!item.smart_query&&!item.track_ids?.length}
                onClick={() => play(item)}
              >
                <Play fill="currentColor" size={19} />
              </IconButton>
              <details className="playlist-item-menu"><summary aria-label={`管理 ${item.name}`}><MoreHorizontal size={20}/></summary><div>
              {!(isNative&&/Android/i.test(navigator.userAgent))&&<PlaylistExport api={api} playlist={item}/>}
              {!item.smart_query&&!item.id.startsWith("favorites-")&&<button onClick={()=>setEditing(item)}><Pencil size={17}/>编辑歌单</button>}
              {item.sync&&<><button disabled={busy===item.id} onClick={()=>void sync(item)}>同步</button>{item.sync.status==="conflict"&&<button onClick={()=>setConflict(item)}>处理冲突</button>}<label className="playlist-auto-sync"><input className="toggle" type="checkbox" aria-label={`${item.name}每小时同步`} checked={item.sync.auto_sync} onChange={e=>void sync(item,{auto_sync:e.target.checked})}/>每小时拉取</label></>}
              <button
                aria-label={`${item.sync?"解除绑定并删除本地副本":"删除"} ${item.name}`}
                onClick={()=>setDeleting(item)}
              >
                <Trash2 size={17} />{item.sync?"解除绑定并删除本地副本":"删除歌单"}
              </button>
              {item.sync?.error&&<p className="playlist-sync-notice">{item.sync.error}</p>}
              {item.smart_query&&<button onClick={()=>setSmartEditor(item)}><Pencil size={17}/>编辑智能歌单</button>}
              </div></details>
            </div>
          ))}
          {cloudItems.map(item=><div className="playlist-row playlist-cloud-row" key={`${item.source}:${item.id}`}><button className="playlist-art" aria-label={`打开歌单 ${item.name}`} onClick={()=>{const provider=providers.find(p=>p.id===item.source);if(provider)setSelectedCloud({provider,item});}}><Cover api={api} track={{cover_url:item.cover_url,title:item.name,album:item.name}}/></button><button className="playlist-name-button" onClick={()=>{const provider=providers.find(p=>p.id===item.source);if(provider)setSelectedCloud({provider,item});}}><strong>{item.name}</strong><span>{providerName(item.source)} · {item.track_count} 首 · {item.writable?"云端歌单":"只读收藏"}</span></button><button disabled={busy===`${item.source}:${item.id}`} onClick={()=>void bind(item)}>同步到本地</button></div>)}
        </div>
      ) : (
        <Empty
          icon={<ListMusic />}
          headline="把某个时刻，收进歌单"
          description="保存当前音区的队列，留给下一次聆听。"
          action={
            <button className="primary" onClick={() => setAdding(true)}>
              创建播放列表
            </button>
          }
        />
      )}
      {smartEditor&&<SmartPlaylistEditor api={api} providers={plugins} playlist={smartEditor===true?undefined:smartEditor} close={()=>setSmartEditor(null)} saved={()=>{setSmartEditor(null);void refresh();notify("智能歌单已保存");}}/>}
      {importing&&<PlaylistImport api={api} close={()=>setImporting(false)} saved={()=>{setImporting(false);void refresh();notify("播放列表已导入");}}/>}
      {editing&&<PlaylistEditor api={api} playlist={editing} onClose={()=>setEditing(null)} onSaved={()=>{setEditing(null);void refresh();notify("歌单已保存");}} onError={onError}/>}
      {deleting&&<Modal title={`删除歌单 · ${deleting.name}`} onClose={()=>setDeleting(null)}><p>{deleting.sync?"只解除同步绑定并删除本地副本，云端歌单会保留。":"删除这份本地歌单，音乐文件会保留。"}</p><div className="form-actions"><button onClick={()=>setDeleting(null)}>取消</button><button className="primary" onClick={()=>void api.delete(`/playlists/${encodeURIComponent(deleting.id)}`).then(()=>{setDeleting(null);return refresh();}).catch(onError)}>删除本地歌单</button></div></Modal>}
      {conflict?.sync?.conflict&&<Modal title={`处理同步冲突 · ${conflict.name}`} onClose={()=>{if(!busy)setConflict(null);}}><PlaylistConflictResolution playlist={conflict} busy={busy===conflict.id} resolve={choice=>resolveConflict(conflict,choice)}/></Modal>}
      {favoritesBinding&&<FavoritesBinding api={api} cloud={cloud} plugins={plugins} onClose={()=>setFavoritesBinding(false)} onChanged={refresh} onError={onError} notify={notify}/>}
      {adding && (
        <Modal title="创建播放列表" onClose={() => setAdding(false)}>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              api
                .post("/playlists", {
                  name,
                  track_ids: includeQueue
                    ? zone?.queue.map((t) => t.id) || []
                    : [],
                })
                .then(() => {
                  setAdding(false);
                  setName("");
                  refresh();
                })
                .catch(onError);
            }}
          >
            <label>
              名称
              <input
                required
                value={name}
                maxLength={100}
                onChange={(e) => setName(e.target.value)}
                placeholder="留给今晚的音乐"
              />
            </label>
            <label className="setting-row">
              <span>包含当前音区队列 ({zone?.queue.length || 0} 首)</span>
              <input
                type="checkbox"
                checked={includeQueue}
                onChange={(e) => setIncludeQueue(e.target.checked)}
              />
            </label>
            <button className="primary">创建</button>
          </form>
        </Modal>
      )}
    </div>
  );
}
