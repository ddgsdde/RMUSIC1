import {useExternalAccount} from "./AccountAccess";
import {useEffect,useState} from "react";
import {Download,LoaderCircle} from "lucide-react";
import type {RMusicClient,Track} from "@r-music/sdk";
import {IconButton} from "./components";
import {saveApiFile} from "./fileDownload";
export function CacheSongButton({api,track,onError}:{api:RMusicClient;track:Track;onError:(error:unknown)=>void}){
 const external=useExternalAccount();
 const [pending,setPending]=useState(false),[ready,setReady]=useState(false),[cacheId,setCacheId]=useState(""),[exporting,setExporting]=useState(false);
 const path=`/library/tracks/${encodeURIComponent(track.id)}/cache`;
 useEffect(()=>{setPending(false);setReady(false);},[api,track.id]);
 useEffect(()=>{if(!pending)return;let active=true,inFlight=false;
  const timer=setInterval(async()=>{if(inFlight||document.hidden)return;inFlight=true;try{const result=await api.get<{status:string;cache_id?:string;error?:string}>(path);if(!active)return;if(result.status==="ready"){setPending(false);setReady(true);setCacheId(result.cache_id||"");window.dispatchEvent(new CustomEvent("rmusic-notice",{detail:"完整音乐文件已加入缓存库"}));}else if(["failed","cancelled","unable"].includes(result.status)){setPending(false);throw new Error(result.error||"缓存任务未完成，请在库管理中查看或重试");}}catch(error){if(active){setPending(false);onError(error);}}finally{inFlight=false;}},2500);return()=>{active=false;clearInterval(timer);};
 },[api,path,pending,onError]);
 return <IconButton label={`${pending?"正在下载到缓存库":external||ready||track.source==="local"?"导出音乐文件":"下载到缓存库"} · ${track.title}`} disabled={pending||exporting} onClick={()=>{if(external||ready||track.source==="local"){const cache=external?undefined:cacheId||(track.provider_data?.cached_from as {cache_id?:string}|undefined)?.cache_id;setExporting(true);void saveApiFile(api,cache?`/cache/${cache}/file`:`/media/${encodeURIComponent(track.id)}`,`${track.title}.${track.format}`).then(()=>window.dispatchEvent(new CustomEvent("rmusic-notice",{detail:"音乐文件已保存到设备下载目录"}))).catch(onError).finally(()=>setExporting(false));return;}setPending(true);void api.post<{status:string;cache_id?:string}>(path,{}).then(result=>{if(result.status==="ready"){setPending(false);setReady(true);setCacheId(result.cache_id||"");}window.dispatchEvent(new CustomEvent("rmusic-notice",{detail:result.status==="ready"?"这首歌已有本地完整文件":"已排队下载到核心缓存库，进度可在库管理查看"}));}).catch(error=>{setPending(false);onError(error);});}}>{pending||exporting?<LoaderCircle className="spin" size={16}/>:<Download size={16}/>}</IconButton>;
}
