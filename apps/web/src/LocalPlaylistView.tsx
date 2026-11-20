import {CatalogSort} from "./CatalogControls";
import {trackSortOptions} from "./TrackSort";
import {moveOrder,useTrackReorder} from "./useTrackReorder";
import {useTrackContextMenu} from "./TrackContextMenu";
import {invalidatePlaylistPages} from "./playlistPageCache";
import {CatalogLink} from "./CatalogLinks";
import {PlaylistExport} from "./PlaylistExchange";
import {playlistPage,rememberPlaylistPage} from "./playlistPageCache";
import {useEffect,useState,type ReactNode} from "react";
import {ArrowLeft,ListPlus,Play} from "lucide-react";
import type {Playlist,RMusicClient,Track} from "@r-music/sdk";
import {Cover,IconButton,quality} from "./components";
import {SongButtons} from "./SongCollection";
import {isAndroidClient} from "./native";
import {Pagination,usePaging} from "./Pagination";
import {formatTime} from "./lyrics";
import type {Picks} from "./PersonalRecommendations";
function PlaylistDetailHeader({api,title,summary,artwork,back,backLabel,playAll,canPlay,mainHeading=false,children}:{
 api:RMusicClient;title:string;summary:string;artwork?:Pick<Track,"cover_url"|"title"|"album">;
 back:()=>void;backLabel:string;playAll:()=>void;canPlay:boolean;mainHeading?:boolean;children?:ReactNode;
}){
 const Heading=mainHeading?"h1":"h2";
 return <>
  <button className="text-button" onClick={back}><ArrowLeft size={17}/>{backLabel}</button>
  <div className="playlist-detail-heading">
   <Cover api={api} track={artwork}/>
   <div><Heading>{title}</Heading><p className="muted">{summary}</p>
    <button className="primary" disabled={!canPlay} onClick={playAll}><Play size={17} fill="currentColor"/>播放全部</button>
    {children}
   </div>
  </div>
 </>;
}

export function LocalPlaylistView({api,playlist,back,play,enqueue,showError,playAll,onChanged}:{api:RMusicClient;playlist:Playlist;back:()=>void;play:(track:Track,list?:Track[])=>Promise<void>;enqueue:(track:Track)=>Promise<void>;showError:(error:unknown)=>void;playAll:()=>void;onChanged:(playlist:Playlist)=>void}){
 const contextMenu=useTrackContextMenu();
 const [order,setOrder]=useState("default"),favorite=playlist.id.startsWith("favorites-");
 const paging=usePaging(`rmusic.playlist-page:${api.baseUrl}:${playlist.id}:${order}`),[items,setItems]=useState<Track[]>([]),[total,setTotal]=useState(playlist.track_ids.length),[busy,setBusy]=useState(false);
 const [sorting,setSorting]=useState(false);
 const reorder=useTrackReorder({version:JSON.stringify([playlist.id,playlist.track_ids]),count:playlist.track_ids.length,disabled:favorite||order!=="default"||sorting||busy||!!playlist.smart_query||items.length!==Math.min(paging.size,playlist.track_ids.length-(paging.page-1)*paging.size),onMove:(from,to)=>{setSorting(true);const order=moveOrder(playlist.track_ids.length,from,to);void api.put<Playlist>(`/playlists/${encodeURIComponent(playlist.id)}`,{track_ids:order.map(i=>playlist.track_ids[i]),expected:{name:playlist.name,track_ids:playlist.track_ids}}).then(saved=>{invalidatePlaylistPages(api);const byId=new Map(items.map(track=>[track.id,track]));const ids=saved.track_ids.slice((paging.page-1)*paging.size,paging.page*paging.size);const reordered=ids.map(id=>byId.get(id)).filter((track):track is Track=>!!track);if(reordered.length===ids.length){setItems(reordered);rememberPlaylistPage(api,JSON.stringify([saved.id,saved.track_ids.join("|"),paging.page,paging.size]),{items:reordered,total:saved.track_ids.length});}onChanged(saved);window.dispatchEvent(new CustomEvent("rmusic-playlists-changed",{detail:api}));}).catch(showError).finally(()=>setSorting(false));}});
 const version=playlist.smart_query?JSON.stringify(playlist.smart_query):playlist.track_ids.join("|");
 useEffect(()=>{let active=true;const key=JSON.stringify([playlist.id,version,paging.page,paging.size,order]);const controller=new AbortController();const cached=playlistPage<{items:Track[];total:number}>(api,key);if(cached){setItems(cached.value.items);setTotal(cached.value.total);}setBusy(!cached);if(cached?.fresh)return;api.request<{items:Track[];total:number}>(`/playlists/${encodeURIComponent(playlist.id)}/tracks?limit=${paging.size}&offset=${(paging.page-1)*paging.size}&sort=${encodeURIComponent(order)}&view=compact`,{signal:controller.signal}).then(value=>{if(active){rememberPlaylistPage(api,key,value);setItems(value.items);setTotal(value.total);}}).catch(error=>{if(active)showError(error);}).finally(()=>{if(active)setBusy(false);});return()=>{active=false;controller.abort();};},[api,playlist.id,version,paging.page,paging.size,order]);
 async function removeTrack(track:Track){await api.post(`/playlists/${encodeURIComponent(playlist.id)}/tracks`,{track_id:track.id,remove:true});invalidatePlaylistPages(api);setItems(old=>old.filter(t=>t.id!==track.id));setTotal(n=>Math.max(0,n-1));window.dispatchEvent(new CustomEvent("rmusic-playlists-changed",{detail:api}));}
 return <div className="local-playlist-view">
  <PlaylistDetailHeader api={api} title={playlist.name} summary={`${total} 首歌曲 · ${playlist.sync?"云端同步":"本地歌单"}`}
    artwork={{cover_url:playlist.cover_url,title:playlist.name,album:playlist.name}}
    back={back} backLabel="全部歌单" playAll={playAll} canPlay={!!total}>
    <PlaylistExport api={api} playlist={playlist}/>
  </PlaylistDetailHeader>
  <CatalogSort label="排序整个歌单" value={order} change={setOrder} options={trackSortOptions} defaultLabel={favorite?"最近加入我喜欢":undefined}/>
  <div className="discovery-track-list" ref={reorder.container} aria-busy={sorting}>{items.map((track,index)=>{const remove=playlist.smart_query?undefined:()=>removeTrack(track);return <div className={`discovery-track ${favorite||playlist.smart_query||order!=="default"?"":"reorderable-track"}`} {...reorder.rowProps((paging.page-1)*paging.size+index)} key={`${track.id}-${index}`} onContextMenu={e=>contextMenu?.(e,track,remove)}>{!favorite&&!playlist.smart_query&&order==="default"&&reorder.handle((paging.page-1)*paging.size+index,track.title)}<button className="discovery-track-title" onClick={()=>void play(track)}><Cover api={api} track={track}/><strong>{track.title}</strong></button><span className="discovery-artist"><CatalogLink track={track}/></span><span className="source-label">{track.source==="local"?"本地":track.source}</span><span className="discovery-quality">{quality(track)}</span><time>{formatTime(track.duration)}</time><div className="row-actions"><SongButtons track={track} remove={isAndroidClient?undefined:remove}/><IconButton label={`追加 ${track.title}`} onClick={()=>void enqueue(track)}><ListPlus size={17}/></IconButton></div></div>;})}</div>
  <Pagination page={paging.page} size={paging.size} total={total} busy={busy} onChange={paging.change}/>
 </div>;
}

export function DailyPlaylistPage({api,picks,canPlay,play,back,backLabel,onError}:{
  api:RMusicClient; picks:Picks; canPlay:boolean; play:(track:Track,tracks:Track[])=>Promise<void>; back:()=>void; backLabel:string;
  onError:(error:unknown)=>void;
}) {
  const menu=useTrackContextMenu();
  const tracks=picks.items;
  const start=(track:Track)=>void play(track,tracks).catch(onError);
  const summary=[picks.date,`${tracks.length} 首歌曲`].filter(Boolean).join(" · ");
  const showScores=tracks.some(track=>picks.scores?.[track.id]!==undefined);
  return <section className="local-playlist-view daily-playlist-page" aria-labelledby="daily-tracks">
    <PlaylistDetailHeader api={api} title="每日歌单" mainHeading summary={summary}
      artwork={tracks.find(track=>track.cover_url)??tracks[0]} canPlay={canPlay&&tracks.length>0}
      playAll={()=>{if(tracks[0])start(tracks[0]);}} back={back} backLabel={backLabel}/>
    <div className="daily-page-columns">
      <h2 id="daily-tracks">曲目</h2>
      {showScores&&<span>偏好分</span>}
    </div>
    <ol className="discovery-track-list">
      {tracks.map(track=>{
        const score=picks.scores?.[track.id];
        return <li className="discovery-track" key={track.id} onContextMenu={event=>menu?.(event,track)}>
          <button className="discovery-track-title" onClick={()=>start(track)} disabled={!canPlay||!track.available}
            aria-label={`播放 ${track.title}`}>
            <Cover api={api} track={track}/>
            <span className="daily-track-label"><strong title={track.title}>{track.title}</strong>
              <small title={track.artist}>{track.artist}</small></span>
          </button>
          <span className="daily-preference-score" aria-label={score===undefined?undefined:`偏好分 ${score}`}>{score}</span>
          <time>{formatTime(track.duration)}</time>
          <div className="row-actions"><SongButtons track={track}/></div>
        </li>;
      })}
    </ol>
  </section>;
}
