import {useEffect,useState} from "react";
import {ApiError,type Playlist,type ProviderManifest,type ProviderPlaylist,type RMusicClient} from "@r-music/sdk";
import {Modal} from "./components";

export function FavoritesBinding({api,cloud,plugins,onClose,onChanged,onError,notify}:{api:RMusicClient;cloud:ProviderPlaylist[];plugins:ProviderManifest[];onClose:()=>void;onChanged:()=>Promise<unknown>;onError:(error:unknown)=>void;notify:(message:string)=>void}){
 const [favorite,setFavorite]=useState<Playlist|null>(null),[selected,setSelected]=useState(""),[busy,setBusy]=useState(false),[transfer,setTransfer]=useState(false);
 useEffect(()=>{let active=true;void api.get<Playlist>("/favorites").then(value=>{if(active)setFavorite(value);}).catch(onError);return()=>{active=false;};},[api,onError]);
 const choices=cloud.filter(item=>item.writable&&["add","remove"].every(cap=>item.write_capabilities?.includes(cap))&&plugins.some(p=>p.enabled&&p.id===item.source&&["playlist_write_v1","account_context_v1"].every(cap=>p.capabilities.includes(cap))));
 const target=choices.find(item=>JSON.stringify([item.source,item.account_context,item.id])===selected);
 const provider=(id:string)=>plugins.find(p=>p.id===id)?.name||id;
 const current=cloud.find(item=>item.source===favorite?.sync?.provider&&item.id===favorite.sync.playlist_id&&item.account_context===favorite.sync.account_context);
 async function sync(resolve?:"cloud"|"local"){
  if(busy||!favorite?.sync)return;
  setBusy(true);
  const conflictId=favorite.sync.conflict?.id;
  try{
   await api.post(`/playlists/${encodeURIComponent(favorite.id)}/sync`,resolve?{resolve,conflict_id:conflictId}:{});
   setFavorite(await api.get<Playlist>("/favorites"));
   await onChanged();
   window.dispatchEvent(new CustomEvent("rmusic-playlists-changed",{detail:api}));
   notify("已排队同步云端歌单");
  }catch(error){
   if(error instanceof ApiError&&error.code==="playlist_conflict_changed"){
    await api.get<Playlist>("/favorites").then(setFavorite).catch(onError);
   }
   onError(error);
  }finally{setBusy(false);}
 }

 async function save(unbind=false,takeOver=false){
  if(busy||!favorite||(!unbind&&!target))return;setBusy(true);
  try{
   if(unbind)await api.delete("/favorites/binding");else await api.request("/favorites/binding",{method:"PUT",body:JSON.stringify({provider:target!.source,playlist_id:target!.id,account_context:target!.account_context,take_over:takeOver})});
   await onChanged();window.dispatchEvent(new CustomEvent("rmusic-playlists-changed",{detail:api}));
   notify(unbind?"已解除绑定，两边歌曲均保留":"已绑定，首次同步会合并两边歌曲；本地未匹配歌曲仍保留");onClose();
  }catch(error){if(error instanceof ApiError&&error.code==="binding_exists")setTransfer(true);else onError(error);}finally{setBusy(false);}
 }
 return <Modal title="我喜欢 · 云端绑定" onClose={()=>!busy&&onClose()}>
  <p>可以选择一张云端歌单双向同步。喜欢／取消喜欢会自动排队推送，云端修改每小时拉取，也可手动同步。</p>
  <p>{favorite?favorite.sync?`当前绑定：${provider(favorite.sync.provider)} · ${current?.name||favorite.sync.playlist_id}`:"当前仅保存在本地，尚未绑定云端歌单。":"正在读取绑定…"}</p>
  {favorite?.sync?.conflict?<PlaylistConflictResolution playlist={favorite} busy={busy} resolve={sync}/>:favorite?.sync&&<button disabled={busy} onClick={()=>void sync()}>立即同步</button>}
  <label className="favorites-binding-field">目标云端歌单<select aria-label="我喜欢绑定的云端歌单" value={selected} disabled={busy||!favorite} onChange={event=>{setSelected(event.target.value);setTransfer(false);}}><option value="">请选择一张可写歌单</option>{choices.map(item=>{const key=JSON.stringify([item.source,item.account_context,item.id]);return <option key={key} value={key}>{provider(item.source)} · {item.name}</option>;})}</select></label>
  {!choices.length&&<p className="fine-print">暂无支持双向同步的歌单。请登录音源并刷新云端歌单目录；只读收藏不能绑定。</p>}
  <p className="fine-print">首次绑定合并两边现有歌曲。只有相同音源的已匹配歌曲会推送；绑定不会自动下载歌曲，也不会修改云端歌单名称。更换或解除绑定保留两边内容。</p>
  {transfer&&<p role="alert">目标已有本地同步副本。转移后原副本和其中未同步的修改会保留在本地，原副本停止同步。</p>}
  <div className="form-actions">{favorite?.sync&&<button disabled={busy} onClick={()=>void save(true)}>解除绑定</button>}<button disabled={busy} onClick={onClose}>取消</button><button className="primary" disabled={busy||!favorite||!target} onClick={()=>void save(false,transfer)}>{transfer?"确认转移并绑定":"绑定并双向同步"}</button></div>
 </Modal>;
}


export function PlaylistConflictResolution({playlist,busy,resolve}:{playlist:Playlist;busy:boolean;resolve:(choice:"cloud"|"local")=>Promise<void>}){
 const conflict=playlist.sync?.conflict;
 if(!conflict)return null;
 return <div className="playlist-conflict" aria-busy={busy}>
  <p role="status">{conflict.reason}</p>
  <p>Core 版本 · {conflict.local.name} · {conflict.local.track_ids.length} 首</p>
  <p>云端版本 · {conflict.cloud.name} · {conflict.cloud.track_ids.length} 首</p>
  <p className="fine-print">保留 Core 版本会同步音源支持的修改。采用云端版本会更新 Core 副本，并保留本地文件和其他音源的歌曲。</p>
  <div className="form-actions">
   <button disabled={busy} onClick={()=>void resolve("cloud")}>采用云端版本</button>
   <button className="primary" disabled={busy||!playlist.sync?.capabilities.length} onClick={()=>void resolve("local")}>保留 Core 版本</button>
  </div>
 </div>;
}
