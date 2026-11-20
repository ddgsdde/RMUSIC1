import {createContext,useContext,useEffect,useRef,useState,type ReactNode} from "react";
import type {RMusicClient,Track} from "@r-music/sdk";
import {Modal} from "./components";
export type CatalogTrack=Pick<Track,"artist"|"album">&Partial<Track>;
export function AlbumArtworkLink({track,children,coverMenu=false}:{track:CatalogTrack;children:ReactNode;coverMenu?:boolean}){
 const context=useContext(Context);
 return <button className="recommendation-cover" aria-label={`打开专辑 ${track.album}`} aria-haspopup={coverMenu?"menu":undefined} aria-description={coverMenu?"长按打开推荐操作":undefined} disabled={!context||!hasAlbum(track)} onClick={()=>context?.open(track,"album",0)}>{children}</button>;
}
type Kind="artist"|"album";
type Link={id:string;name?:string;title?:string;artist?:string;source?:string};
const Context=createContext<{browse:(kind:Kind,id:string,name?:string)=>void;open:(track:CatalogTrack,kind:Kind,index:number,name?:string)=>void;api:RMusicClient}|null>(null);
export function artistNames(track:CatalogTrack):string[]{
 const explicit=track.provider_data?.artist_credits as {name?:string}[]|undefined;
 if(Array.isArray(explicit)&&explicit.length>0&&explicit.length<=64&&explicit.every(x=>typeof x.name==="string"&&x.name.trim()))return explicit.map(x=>x.name!);
 const provider=track.source==="local"?(track.provider_data?.cached_from as {source?:string})?.source||track.source:track.source;
 for(const key of ["provider_metadata","qqmusic_metadata"]){
  const metadata=track.provider_data?.[key] as {matched?:boolean;source?:string;track?:{artist?:string};artists?:{name?:string;id?:string;mid?:string}[]}|undefined;
  const people=metadata?.artists;
  if(metadata?.matched&&metadata.source===provider&&metadata.track?.artist===track.artist&&Array.isArray(people)&&people.length>0&&people.length<=64&&people.every(p=>p.name&&(p.id||p.mid))){
   const names=people.map(p=>p.name!);if(names.join(" / ")===track.artist)return names;
  }
 }
 const ids=track.provider_data?.artist_mids,names=track.artist.split(" / ");
 return (track.source==="qqmusic"||(track.provider_data?.cached_from as {source?:string})?.source==="qqmusic")&&Array.isArray(ids)&&ids.length===names.length&&ids.length<=64&&ids.every(x=>typeof x==="string"&&x)?names:[track.artist];
}
export function CatalogNavigation({api,location,navigate,onError,children}:{api:RMusicClient;location:string;navigate:(kind:Kind,id:string,name?:string)=>void;onError:(error:unknown)=>void;children:ReactNode}){
 const pending=useRef<AbortController|null>(null),[choices,setChoices]=useState<{kind:Kind;items:Link[]}|null>(null);
 useEffect(()=>()=>{pending.current?.abort();},[api,location]);
 const open=async(track:CatalogTrack,kind:Kind,index:number,selectedName?:string)=>{
  pending.current?.abort();const controller=new AbortController();pending.current=controller;
  try{
   let items:Link[];
   {
    const links=await api.request<{resolved:boolean;artists:Link[];album:Link}>("/library/links",{method:"POST",body:JSON.stringify({resolve_credits:kind==="artist",track_id:track.id,artist:track.artist,album:track.album,album_artist:track.album_artist,source:track.source,provider_data:track.provider_data||{}}),signal:controller.signal});
    items=kind==="album"?[links.album]:selectedName?links.artists.filter(person=>person.name===selectedName):artistNames(track).length===1&&links.artists.length>1?links.artists:links.artists.filter(person=>person.name===artistNames(track)[index]);
   if(!links.resolved){
    const name=kind==="album"?track.album:artistNames(track)[index];
    const page=await api.request<{items:Link[]}>(`/library/${kind==="album"?"albums":"artists"}?q=${encodeURIComponent(name)}&limit=200`,{signal:controller.signal});
    items=page.items.filter(item=>kind==="album"?item.title===name&&(!track.album_artist||item.artist===track.album_artist):item.name===name);
   }
   }
   if(controller.signal.aborted)return;
   if(items.length===1){setChoices(null);navigate(kind,items[0].id,items[0].name||items[0].title);}
   else if(items.length>1)setChoices({kind,items});
   else throw new Error("没有可确认身份的曲库资料，请先打开歌曲资料查看关联目录。");
  }catch(error){if(!controller.signal.aborted)onError(error);}
 };
 return <Context.Provider value={{api,browse:navigate,open:(track,kind,index,name)=>void open(track,kind,index,name)}}>{children}{choices&&<Modal title="选择对应资料" onClose={()=>setChoices(null)}>{choices.items.map(item=><button key={item.id} onClick={()=>{navigate(choices.kind,item.id,item.name||item.title);setChoices(null);}}>{item.name||item.title} · {item.artist||item.source||item.id.slice(0,8)}</button>)}</Modal>}</Context.Provider>;
}
export function hasAlbum(track:CatalogTrack):boolean {
 const name=track.album.trim().toLowerCase();return !!name&&(!['none','null','unknown','unknown album','未知专辑','未提供专辑信息'].includes(name)||!!track.provider_data?.album_mid||!!track.provider_data?.release_id);
}
export function CatalogLink({track,album=false,separator=" / "}:{track:CatalogTrack;album?:boolean;separator?:string}){
 const context=useContext(Context),[resolved,setResolved]=useState<string[]>([]);
 const api=context?.api;
 useEffect(()=>{setResolved([]);if(!api||album||artistNames(track).length!==1||!track.artist.includes(" / "))return;const abort=new AbortController();
  void api.request<{artists:Link[]}>("/library/links",{method:"POST",body:JSON.stringify({resolve_credits:true,track_id:track.id,artist:track.artist,album:track.album,source:track.source||"",provider_data:track.provider_data||{}}),signal:abort.signal}).then(v=>{const names=v.artists.map(p=>p.name||"");if(!abort.signal.aborted&&names.length>1&&names.every(Boolean)&&new Set(names).size===names.length)setResolved(names);}).catch(()=>{});return()=>abort.abort();
 },[api,album,track.id,track.artist,track.source]);
 if(album&&!hasAlbum(track))return <span className="muted">未归属专辑</span>;
 const names=album?[track.album]:resolved.length?resolved:artistNames(track);
 const displayName=(name:string)=>album||separator===" / "?name:name.replaceAll(" / ",separator);
 if(!context)return <>{album?track.album:displayName(track.artist)}</>;
 return <span className="catalog-links">{names.map((name,index)=><span key={`${index}:${name}`}>{index>0&&separator}<span className="catalog-link" role="link" tabIndex={0} title={`打开${album?"专辑":"歌手"} ${name}`} onClick={e=>{e.stopPropagation();context.open(track,album?"album":"artist",index,name);}} onDoubleClick={e=>e.stopPropagation()} onKeyUp={e=>{if(e.key==="Enter"||e.key===" ")e.stopPropagation();}} onKeyDown={e=>{if(e.key==="Enter"||e.key===" "){e.preventDefault();e.stopPropagation();context.open(track,album?"album":"artist",index,name);}}}>{displayName(name)}</span></span>)}</span>;
}

export function useCatalogNavigation(){return useContext(Context);}
