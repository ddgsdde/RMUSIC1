import {useEffect} from "react";
import {ApiError,type RMusicClient,type User} from "@r-music/sdk";
import {listenJournal,rememberListenOwner,type ListenResult} from "./offlineStats";
export function useOfflineStats(api:RMusicClient,user:User|null,coreId:string,online:boolean,native:boolean,onError:(error:unknown)=>void,measured=true){
  useEffect(()=>{
    if(!user||!coreId)return;
    const owner={core_id:coreId,user_id:user.id};rememberListenOwner(api.baseUrl,owner);
    if(native||!online)return;
    let active=true,running=false,controller:AbortController|undefined;
    async function sync(){
      if(running||!active)return;running=true;
      try{
        const rows=await listenJournal.pending(owner,measured);if(!active||!rows.length)return;
        controller=new AbortController();const timeout=setTimeout(()=>controller?.abort(),10000);
        let result:ListenResult;
        try{result=await api.syncOfflineStats({...owner,client_id:rows[0].client_id,sessions:rows.map(row=>row.snapshot)},controller.signal);}
        finally{clearTimeout(timeout);}
        if(!active)return;await listenJournal.acknowledge(owner,result);
        if(result.rejected.length)onError(new Error("部分离线聆听记录存在冲突，已保留在本机，请检查设备时间。"));
      }catch(error){if(active&&error instanceof ApiError&&[401,403].includes(error.status))onError(error);}
      finally{running=false;}
    }
    void sync();const timer=setInterval(sync,15000);window.addEventListener("online",sync);
    return()=>{active=false;clearInterval(timer);controller?.abort();window.removeEventListener("online",sync);};
  },[api,user,coreId,online,native,onError,measured]);
}
