import {useTrackSort} from "./TrackSort";
import {useLayoutEffect,useState} from "react";
import type {RMusicClient,Zone} from "@r-music/sdk";
import {Cover} from "./components";
import {CatalogLink,hasAlbum} from "./CatalogLinks";
import {isAndroidClient} from "./native";
import {SongMenuButton} from "./SongCollection";
import {CacheSongButton} from "./CacheSongButton";
import {formatTime} from "./lyrics";
import {moveOrder,useTrackReorder} from "./useTrackReorder";
export function PlaybackQueueTracks({api,zone,play,updated,onError}:{api:RMusicClient;zone:Zone;play:(index:number)=>void;updated:(zone:Zone)=>void;onError:(e:unknown)=>void}){
 const [saving,setSaving]=useState(false),sorted=useTrackSort(zone.queue);
 const reorder=useTrackReorder({version:JSON.stringify([zone.id,zone.playback_id,zone.queue.map(t=>t.id)]),count:zone.queue.length,disabled:saving,onMove:(from,to)=>{setSaving(true);void api.post<Zone>(`/zones/${encodeURIComponent(zone.id)}/queue/reorder`,{expected_queue:zone.queue.map(t=>t.id),expected_playback_id:zone.playback_id,order:moveOrder(zone.queue.length,from,to)}).then(updated).catch(onError).finally(()=>setSaving(false));}});
 useLayoutEffect(()=>{
  const container=reorder.container.current,row=container?.querySelector<HTMLElement>(`[data-reorder-index="${zone.queue_index}"]`);
  if(!container||!row)return;
  container.scrollTop+=row.getBoundingClientRect().top-container.getBoundingClientRect().top-(container.clientHeight-row.clientHeight)/2;
 },[zone.id]);
 return <>{sorted.control}<div className="queue-tracks" ref={reorder.container} aria-busy={saving}>{sorted.indices.map(index=>{const track=zone.queue[index];return <div className={`queue-track ${zone.queue_index===index?"current":""}`} key={`${track.id}-${index}`} {...reorder.rowProps(index)}>{sorted.order==="default"&&reorder.handle(index,track.title)}<button className="queue-play" onClick={()=>play(index)}><Cover api={api} track={track}/><div><strong title={track.title}>{track.title}</strong><span><CatalogLink track={track}/>{hasAlbum(track)&&<> · <CatalogLink track={track} album/></>}</span></div></button><time>{formatTime(track.duration)}</time><CacheSongButton api={api} track={track} onError={onError}/>{!isAndroidClient&&<SongMenuButton track={track} current={zone.queue_index===index}/>}</div>;})}</div></>;
}
