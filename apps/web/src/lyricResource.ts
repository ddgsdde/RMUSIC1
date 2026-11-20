import type {LyricDocument,RMusicClient} from "@r-music/sdk";
import {cachedResource,loadResource} from "./resourceCache";
const key=(id:string)=>`lyrics-v2:${id}`;
export function cachedLyrics(api:RMusicClient,id:string){return cachedResource<LyricDocument>(api,key(id));}
export function loadLyrics(api:RMusicClient,id:string,force=false){
 return loadResource(api,key(id),async()=>{
  const controller=new AbortController(),timeout=setTimeout(()=>controller.abort(),20000);
  try{return await api.request<LyricDocument>(`/library/tracks/${encodeURIComponent(id)}/lyrics`,{signal:controller.signal});}
  finally{clearTimeout(timeout);}
 },3600000,force);
}
