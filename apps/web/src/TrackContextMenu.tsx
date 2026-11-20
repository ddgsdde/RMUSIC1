import {useSimilarSongs} from "./PersonalRecommendations";
import {useEffect,useLayoutEffect,useRef,useState,type ReactNode} from "react";
import {Album,Download,Heart,Info,ScanSearch,ThumbsDown,ListEnd,ListStart,Pause,Play,Plus,Trash2,UserRound} from "lucide-react";
import type {RMusicClient,Track} from "@r-music/sdk";
import {hasAlbum,artistNames,useCatalogNavigation} from "./CatalogLinks";
import {sourceTrack,useSongCollectionActions} from "./SongCollection";
import {TrackMenuContext,type MenuTrack,type TrackMenuOptions} from "./trackMenuContext";
import {Cover} from "./components";
import {saveApiFile} from "./fileDownload";
export {useTrackContextMenu} from "./trackMenuContext";
type Menu={x:number;y:number;anchor?:HTMLElement;origin:HTMLElement;track:MenuTrack;remove?:()=>Promise<void>;current?:boolean;recommendationFeedback?:TrackMenuOptions["recommendationFeedback"];keyboard:boolean};
function complete(t:MenuTrack):t is Track{return typeof t.available==="boolean"&&typeof t.sample_rate==="number"&&typeof t.duration==="number";}
export function TrackContextMenuProvider({api,location,play,startRadio,enqueue,playNext,details,onError,downloadOffline,children,currentTrackId,currentPlaying,onCurrentToggle}:{api:RMusicClient;location:string;play:(t:Track)=>Promise<void>;startRadio?:(t:Track)=>Promise<void>;enqueue:(t:Track)=>Promise<void>;playNext?:(t:Track)=>Promise<void>;details:(t:Track)=>void;onError:(e:unknown)=>void;downloadOffline?:(t:Track)=>Promise<unknown>;children:ReactNode;currentTrackId?:string;currentPlaying?:boolean;onCurrentToggle?:()=>Promise<void>}){
 const[menu,setMenu]=useState<Menu>(),[full,setFull]=useState<Track>(),[people,setPeople]=useState<string[]>([]),[loading,setLoading]=useState(false),[position,setPosition]=useState({x:8,y:8});
 const catalog=useCatalogNavigation(),collection=useSongCollectionActions(),panel=useRef<HTMLDivElement>(null);
 const similar=useSimilarSongs();
 const close=(restore=false)=>{if(restore&&menu?.anchor?.isConnected)menu.anchor.focus({preventScroll:true});setMenu(undefined);};
 useEffect(()=>{setMenu(undefined);},[api,location]);
 useEffect(()=>{if(!menu)return;
  const outside=(event:Event)=>{if(!(event.target instanceof Node)||!panel.current?.contains(event.target))setMenu(undefined);};
  const key=(event:KeyboardEvent)=>{if(event.key==="Escape"){event.preventDefault();event.stopImmediatePropagation();close(true);return;}if(event.key==="Tab"){setMenu(undefined);return;}
   if(!["ArrowDown","ArrowUp","Home","End"].includes(event.key))return;event.preventDefault();const items=[...panel.current?.querySelectorAll<HTMLButtonElement>('button[role="menuitem"]:not(:disabled)')||[]];if(!items.length)return;const index=items.indexOf(document.activeElement as HTMLButtonElement);const next=event.key==="Home"?0:event.key==="End"?items.length-1:(index+(event.key==="ArrowDown"?1:-1)+items.length)%items.length;items[next].focus();};
  const scroll=(event:Event)=>{
   if(event.target instanceof Element&&!event.target.contains(menu.origin))return;
   outside(event);
  };
  window.addEventListener("pointerdown",outside);window.addEventListener("keydown",key,true);window.addEventListener("scroll",scroll,true);window.addEventListener("resize",outside);
  if(menu.keyboard)panel.current?.focus({preventScroll:true});
  return()=>{window.removeEventListener("pointerdown",outside);window.removeEventListener("keydown",key,true);window.removeEventListener("scroll",scroll,true);window.removeEventListener("resize",outside);};
 },[menu]);
 useEffect(()=>{if(!menu)return;const abort=new AbortController();setFull(complete(menu.track)?menu.track:undefined);setPeople(complete(menu.track)?artistNames(menu.track):[]);setLoading(!complete(menu.track));
  void(async()=>{try{const t=complete(menu.track)?menu.track:await api.request<Track>(`/library/tracks/${encodeURIComponent(menu.track.id)}`,{signal:abort.signal});if(abort.signal.aborted)return;setFull(t);setPeople(artistNames(t));
   if(artistNames(t).length===1&&t.artist.includes(" / ")){const links=await api.request<{artists:{name:string}[]}>("/library/links",{method:"POST",body:JSON.stringify({track_id:t.id,artist:t.artist,album:t.album,resolve_credits:true}),signal:abort.signal});if(!abort.signal.aborted)setPeople(links.artists.map(v=>v.name));}
  }catch(error){if(!abort.signal.aborted)onError(error);}finally{if(!abort.signal.aborted)setLoading(false);}})();return()=>abort.abort();
 },[api,menu]);
 const t=menu?(full?.id===menu.track.id?full:complete(menu.track)?menu.track:undefined):undefined;
 useLayoutEffect(()=>{if(!menu||!panel.current)return;const box=panel.current.getBoundingClientRect(),anchor=menu.anchor?.getBoundingClientRect(),gap=7;let x=anchor?anchor.right-box.width:menu.x,y=anchor?anchor.bottom+gap:menu.y;
  if(anchor&&y+box.height>innerHeight-8)y=anchor.top-box.height-gap;
  setPosition({x:Math.max(8,Math.min(x,innerWidth-box.width-8)),y:Math.max(8,Math.min(y,innerHeight-box.height-8))});
 },[menu,t,people,loading]);
 const current=!!menu&&(menu.current||menu.track.id===currentTrackId),available=!!t&&t.available!==false;
 const act=(action:()=>void|Promise<void>)=>{close(true);Promise.resolve().then(action).catch(onError);};
 const item=(label:string,icon:ReactNode,action:()=>void|Promise<void>,disabled=false)=><button role="menuitem" disabled={disabled} onClick={()=>act(action)}>{icon}<span>{label}</span></button>;
 return <TrackMenuContext.Provider value={(event,track,remove,options)=>{event.preventDefault();event.stopPropagation();const origin=event.currentTarget as HTMLElement,anchor=event.type==="contextmenu"?undefined:origin;setFull(complete(track)?track:undefined);setPeople(complete(track)?artistNames(track):[]);setMenu({x:event.clientX,y:event.clientY,anchor,origin,track,remove,current:options?.current,recommendationFeedback:options?.recommendationFeedback,keyboard:event.detail===0});}}>{children}{menu&&<>
  <div className="track-menu-backdrop" aria-hidden="true" onPointerDown={event=>{event.preventDefault();event.stopPropagation();close(true);}}/>
  <div ref={panel} tabIndex={-1} className="track-context-menu" role="menu" aria-label={`歌曲操作 ${menu.track.title}`} style={{left:position.x,top:position.y}}>
   <header className="track-menu-heading"><Cover api={api} track={{title:menu.track.title,album:menu.track.album||"",cover_url:menu.track.cover_url}}/><div><strong>{menu.track.title}</strong><small>{menu.track.artist}</small>{current&&<span>{currentPlaying?"正在播放":"当前歌曲"}</span>}</div></header>
   {loading&&<small role="status">正在读取歌曲资料…</small>}
   {current?(onCurrentToggle&&item(currentPlaying?"暂停":"继续播放",currentPlaying?<Pause/>:<Play/>,onCurrentToggle)): <>
    {item("播放",<Play/>,()=>t&&play(t),!available)}
    {playNext&&item("下一首播放",<ListStart/>,()=>t&&playNext(t),!available)}
    {item("添加到队列末尾",<ListEnd/>,()=>t&&enqueue(t),!available)}
   </>}
   {current&&menu.recommendationFeedback&&item("添加到队列末尾",<ListEnd/>,()=>t&&enqueue(t),!available)}
   {menu.track.id===currentTrackId&&similar&&item("找相似",<ScanSearch/>,()=>similar.find(menu.track))}
   {startRadio&&item("启动电台",<Play/>,()=>t&&startRadio(t),!available)}
   {menu.recommendationFeedback&&item(menu.recommendationFeedback.label,<ThumbsDown/>,menu.recommendationFeedback.apply)}
   {collection&&t&&<>{item(collection.liked.has(t.id)||collection.liked.has(sourceTrack(t).id)?"取消喜欢":"喜欢",<Heart/>,()=>collection.like(t))}{item("添加到歌单…",<Plus/>,()=>collection.add(t))}</>}
   <hr/>
   {item("转到专辑",<Album/>,()=>t&&catalog?.open(t,"album",0),!t||!hasAlbum(t))}
   {people.map((name,index)=><button key={`${index}:${name}`} role="menuitem" disabled={!t} onClick={()=>t&&act(()=>catalog?.open(t,"artist",index,name))}><UserRound/><span>转到歌手 · {name}</span></button>)}
   {item("歌曲资料与评论",<Info/>,()=>t&&details(t),!t)}
   {menu.remove&&<><hr/>{item("从此歌单移除",<Trash2/>,menu.remove)}</>}
   <hr/>
   {downloadOffline&&item("下载供离线播放",<Download/>,async()=>{if(t)await downloadOffline(t);},!available)}
   {item("下载原文件…",<Download/>,async()=>{if(t)await saveApiFile(api,`/media/${encodeURIComponent(t.id)}`,`${t.title}.${t.format==="unknown"?"audio":t.format}`);},!available)}
  </div>
 </>}</TrackMenuContext.Provider>;
}
