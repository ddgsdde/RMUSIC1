import {clientId} from "./clientId";
import type {OfflineListenOwner as ListenOwner,OfflineListenSnapshot as ListenSnapshot,OfflineStatsResult as ListenResult} from "@r-music/sdk";
export type {OfflineListenOwner as ListenOwner,OfflineListenSnapshot as ListenSnapshot,OfflineStatsResult as ListenResult} from "@r-music/sdk";
export interface ListenRow {id:string;client_id:string;owner:ListenOwner;snapshot:ListenSnapshot;ack:number;error?:string}
interface Owners {bases:Record<string,string>;users:Record<string,string>}
function owners():Owners {try{return JSON.parse(localStorage.getItem("rmusic.listen-owners")||"null")||{bases:{},users:{}};}catch{return {bases:{},users:{}};}}
function baseKey(base:string){return new URL(base||"/",location.href).href.replace(/\/+$/,"");}
export function observeListenCore(base:string,core:string){
  const value=owners();value.bases[baseKey(base)]=core;localStorage.setItem("rmusic.listen-owners",JSON.stringify(value));
}
export function rememberListenOwner(base:string,owner:ListenOwner){
  const value=owners();const changed=value.users[owner.core_id]!==owner.user_id;
  value.bases[baseKey(base)]=owner.core_id;value.users[owner.core_id]=owner.user_id;
  localStorage.setItem("rmusic.listen-owners",JSON.stringify(value));
  if(changed)window.dispatchEvent(new Event("rmusic-listen-owner-changed"));
}
export function forgetListenOwner(base:string){
  const value=owners(),core=value.bases[baseKey(base)];if(core)delete value.users[core];
  localStorage.setItem("rmusic.listen-owners",JSON.stringify(value));window.dispatchEvent(new Event("rmusic-listen-owner-changed"));
}
export function listenOwner(core:string|undefined):ListenOwner|undefined {
  const user=core&&owners().users[core];return core&&user?{core_id:core,user_id:user}:undefined;
}
export function listenCore(base:string):string|undefined {return owners().bases[baseKey(base)];}
/** Credit measured progress only. Seek, pause and long suspension establish new baselines. */
export class ListenSampler {
  snapshot:ListenSnapshot;
  private baseline:{at:number;mono:number;position:number;playing:boolean}|undefined;
  private saved=0;
  constructor(track:{track_id:string;title?:string;artist?:string},duration:number,at=Date.now()){
    this.snapshot={id:clientId(),version:0,track_id:track.track_id,title:track.title||track.track_id,artist:track.artist||"",duration:Number.isFinite(duration)&&duration>0?duration:0,started_at:at,updated_at:at,position:0,days:{},reason:"active"};
  }
  sample(position:number,playing:boolean,reset=false,at=Date.now(),mono=performance.now()){
    if(this.snapshot.reason!=="active"||!Number.isFinite(position)||position<0)return;
    const prior=this.baseline,elapsed=prior?(mono-prior.mono)/1000:0,delta=prior?position-prior.position:0;
    // Clock rollback or a wall-clock jump must not fabricate a listening date or interval.
    if(!reset&&prior?.playing&&elapsed>0&&elapsed<=30&&delta>0&&delta<=elapsed+1&&at>=prior.at&&Math.abs((at-prior.at)/1000-elapsed)<2){
      let start=prior.at;const end=at;
      while(start<end){
        const date=new Date(start).toISOString().slice(0,10),boundary=Date.parse(`${date}T00:00:00Z`)+86400000,stop=Math.min(end,boundary);
        const seconds=delta*(stop-start)/(end-prior.at),day=this.snapshot.days[date]||{seconds:0,last_at:start};
        day.seconds+=seconds;day.last_at=stop===boundary?stop-1:stop;this.snapshot.days[date]=day;start=stop;
      }
    }
    this.snapshot.position=position;this.snapshot.updated_at=Math.max(this.snapshot.updated_at,at);
    this.baseline={at,mono,position,playing};
  }
  flush(force=false,at=Date.now()):ListenSnapshot|undefined {
    if(!force&&at-this.saved<5000)return;
    this.saved=at;this.snapshot.version++;
    return structuredClone(this.snapshot);
  }
  finish(reason:ListenSnapshot["reason"]){this.snapshot.reason=reason;return this.flush(true)!;}
}
function db():Promise<IDBDatabase>{return new Promise((resolve,reject)=>{
  const request=indexedDB.open("r-music-listens-v1",2);
  request.onupgradeneeded=()=>{for(const name of ["listens","cursors"])if(!request.result.objectStoreNames.contains(name))request.result.createObjectStore(name,{keyPath:"id"});};
  request.onsuccess=()=>resolve(request.result);request.onerror=()=>reject(request.error);
});}
export async function listenTransaction<T>(stores:string[],mode:IDBTransactionMode,action:(tx:IDBTransaction,done:(value:T)=>void)=>void):Promise<T>{
  const connection=await db();try{return await new Promise<T>((resolve,reject)=>{
    const tx=connection.transaction(stores,mode);let result:T;
    tx.oncomplete=()=>resolve(result);tx.onabort=()=>reject(tx.error||new Error("离线统计保存中断"));tx.onerror=()=>reject(tx.error);
    try{action(tx,value=>{result=value;});}catch(error){tx.abort();reject(error);}
  });}finally{connection.close();}
}
function transaction<T>(mode:IDBTransactionMode,action:(store:IDBObjectStore,done:(value:T)=>void)=>void){return listenTransaction<T>(["listens"],mode,(tx,done)=>action(tx.objectStore("listens"),done));}
export function putListen(store:IDBObjectStore,owner:ListenOwner,snapshot:ListenSnapshot,client:string){
  const row:ListenRow={id:snapshot.id,client_id:client,owner,snapshot,ack:0},read=store.get(row.id);
  read.onsuccess=()=>{const old=read.result as ListenRow|undefined;if(!old||old.owner.core_id===owner.core_id&&old.owner.user_id===owner.user_id&&old.snapshot.version<snapshot.version)store.put({...row,ack:old?.ack||0,error:old?.error});};
}
export const listenJournal={
  client(){let id=localStorage.getItem("rmusic.listen-client");if(!id){id=clientId();localStorage.setItem("rmusic.listen-client",id);}return id;},
  save(owner:ListenOwner,snapshot:ListenSnapshot){
    const client=this.client();return transaction<void>("readwrite",store=>putListen(store,owner,snapshot,client));
  },
  pending(owner:ListenOwner,measured=true){return transaction<ListenRow[]>("readonly",(store,done)=>{const read=store.getAll();read.onsuccess=()=>{const rows=(read.result as ListenRow[]).filter(row=>row.owner.core_id===owner.core_id&&row.owner.user_id===owner.user_id&&!row.error&&row.snapshot.version>row.ack&&(measured||!row.snapshot.measured)).sort((a,b)=>a.snapshot.updated_at-b.snapshot.updated_at);done(rows.filter(row=>row.client_id===rows[0]?.client_id).slice(0,32));};});},
  acknowledge(owner:ListenOwner,result:ListenResult){return transaction<void>("readwrite",(store,done)=>{
    for(const ack of [...result.accepted,...result.rejected]){const read=store.get(ack.id);read.onsuccess=()=>{const row=read.result as ListenRow|undefined;if(!row||row.owner.core_id!==owner.core_id||row.owner.user_id!==owner.user_id)return;
      if("code" in ack){if(row.snapshot.version===ack.version)store.put({...row,error:ack.code});}
      else store.put({...row,ack:Math.max(row.ack,ack.version)});
    };}done();
  });},
};
