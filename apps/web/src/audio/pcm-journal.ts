import type {SequenceBoundary,SequenceRecoverRequest} from "@r-music/sdk";
import {clientId} from "../clientId";
import {listenTransaction,listenJournal,putListen,type ListenOwner,type ListenSnapshot,type ListenRow} from "../offlineStats";

export interface PcmCursor {
  id:string;owner:ListenOwner;endpoint:string;zone:string;run:string;tab:string;version:number;
  plan:string;rate:number;heard:SequenceBoundary[];terminal:boolean;retired?:boolean;request?:SequenceRecoverRequest;
}
function key(owner:ListenOwner,endpoint:string,zone:string){return JSON.stringify([owner.core_id,owner.user_id,endpoint,zone]);}
export function pcmTab(){let tab=sessionStorage.getItem("rmusic.pcm-tab");if(!tab){tab=clientId();sessionStorage.setItem("rmusic.pcm-tab",tab);}return tab;}
const closedKey=(cursor:PcmCursor)=>`rmusic.pcm-closed:${cursor.id}`;
export function closePcmPage(cursor:PcmCursor){localStorage.setItem(closedKey(cursor),cursor.run);}
export function canResumePcmPage(cursor:PcmCursor){return cursor.tab===pcmTab()&&(localStorage.getItem(closedKey(cursor))!==cursor.run||performance.getEntriesByType("navigation").some(entry=>(entry as PerformanceNavigationTiming).type==="reload"));}
export const pcmJournal={
  begin(value:Omit<PcmCursor,"id"|"run"|"tab"|"version">){
    const cursor:PcmCursor={...value,id:key(value.owner,value.endpoint,value.zone),run:clientId(),tab:pcmTab(),version:1};
    return listenTransaction<PcmCursor>(["cursors"],"readwrite",(tx,done)=>{
      const store=tx.objectStore("cursors"),read=store.getAll();read.onsuccess=()=>{for(const old of read.result as PcmCursor[])if(old.owner.core_id===cursor.owner.core_id&&old.owner.user_id===cursor.owner.user_id&&old.endpoint===cursor.endpoint)store.put({...old,retired:true});store.put(cursor);done(cursor);};
    });
  },
  checkpoint(cursor:PcmCursor,snapshots:ListenSnapshot[]){
    const client=listenJournal.client();return listenTransaction<void>(["cursors","listens"],"readwrite",tx=>{
      for(const snapshot of snapshots)putListen(tx.objectStore("listens"),cursor.owner,snapshot,client);
      const store=tx.objectStore("cursors"),read=store.get(cursor.id);read.onsuccess=()=>{const old=read.result as PcmCursor|undefined;if(old?.run===cursor.run&&!old.retired&&!old.request&&old.version<cursor.version)store.put(cursor);};
    });
  },
  load(owner:ListenOwner,endpoint:string,zone:string){return listenTransaction<PcmCursor|undefined>(["cursors"],"readonly",(tx,done)=>{const read=tx.objectStore("cursors").get(key(owner,endpoint,zone));read.onsuccess=()=>{const cursor=read.result as PcmCursor|undefined;done(cursor&&!cursor.retired?cursor:undefined);};});},
  retire(cursor:PcmCursor){return listenTransaction<void>(["cursors"],"readwrite",tx=>{const store=tx.objectStore("cursors"),read=store.get(cursor.id);read.onsuccess=()=>{const old=read.result as PcmCursor|undefined;if(old?.run===cursor.run)store.put({...old,retired:true});};});},
  freeze(cursor:PcmCursor){return listenTransaction<PcmCursor|undefined>(["cursors","listens"],"readwrite",(tx,done)=>{
    const store=tx.objectStore("cursors"),read=store.get(cursor.id);read.onsuccess=()=>{
      const old=read.result as PcmCursor|undefined;if(!old||old.run!==cursor.run||old.retired){done(undefined);return;}
      const records=tx.objectStore("listens").getAll();records.onsuccess=()=>{
        if((records.result as ListenRow[]).some(row=>row.owner.core_id===old.owner.core_id&&row.owner.user_id===old.owner.user_id&&row.error&&old.heard.some(source=>source.playback_id===row.snapshot.measured?.playback_id))){tx.abort();return;}
        old.request||={endpoint_id:old.endpoint,request_id:clientId(),plan_id:old.plan,sample_rate:old.rate,heard:old.heard,terminal:old.terminal};store.put(old);done(old);
      };
    };
  });},
};
