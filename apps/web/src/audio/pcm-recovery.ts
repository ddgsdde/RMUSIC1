import type {RMusicClient,Zone,OfflineStatsResult,SequenceRecoverResult} from "@r-music/sdk";
import {listenCore,listenJournal} from "../offlineStats";
import {pcmJournal,canResumePcmPage} from "./pcm-journal";

/** The caller has stopped old output and serialized all final journal writes. */
export async function recoverPcm(api:RMusicClient,zone:Zone,signal:AbortSignal,manual=false):Promise<Zone|undefined>{
  async function request<T>(path:string,body?:unknown):Promise<T>{
    signal.throwIfAborted();const controller=new AbortController(),abort=()=>controller.abort();signal.addEventListener("abort",abort,{once:true});const timeout=setTimeout(abort,5000);
    try{return await api.request<T>(path,{...(body===undefined?{}:{method:"POST",body:JSON.stringify(body)}),signal:controller.signal});}
    finally{clearTimeout(timeout);signal.removeEventListener("abort",abort);}
  }
  const [status,user]=await Promise.all([request<{core_id:string}>("/status"),request<{id:string}>("/auth/me")]);
  const expected=listenCore(api.baseUrl);if(expected&&expected!==status.core_id)throw new Error("连接的核心身份已改变，请重新连接");
  const owner={core_id:status.core_id,user_id:user.id};let cursor=await pcmJournal.load(owner,zone.endpoint_id,zone.id);signal.throwIfAborted();
  if(cursor&&!cursor.request&&zone.playback_id&&!cursor.heard.some(source=>source.playback_id===zone.playback_id)){await pcmJournal.retire(cursor);cursor=undefined;}
  if(cursor&&!manual&&!canResumePcmPage(cursor))return undefined;
  if(!cursor)return (await request<{zone:Zone}>(`/zones/${zone.id}/recover/unavailable`,{endpoint_id:zone.endpoint_id})).zone;
  // Import durable EOF/positions before the Core chooses which authorized source remains.
  while(true){
    const rows=await listenJournal.pending(owner);signal.throwIfAborted();if(!rows.length)break;
    const result=await request<OfflineStatsResult>("/stats/offline",{...owner,client_id:rows[0].client_id,sessions:rows.map(row=>row.snapshot)});
    await listenJournal.acknowledge(owner,result);
    if(!result.accepted.length&&!result.rejected.length)throw new Error("核心未确认本机聆听记录");
  }
  const frozen=await pcmJournal.freeze(cursor);signal.throwIfAborted();if(!frozen?.request)return zone;
  const result=await request<SequenceRecoverResult>(`/zones/${zone.id}/recover`,frozen.request);signal.throwIfAborted();
  if(!result.accepted||result.finished)await pcmJournal.retire(frozen);
  // A successful active receipt remains durable until the new output's begin replaces it.
  return result.zone;
}
