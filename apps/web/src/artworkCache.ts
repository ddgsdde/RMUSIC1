import type {RMusicClient} from '@r-music/sdk';
import {isNative,nativeCover} from './native';
type Picture={src:string;bytes:number;refs:number;cached:boolean};
type Entry={revision?:number;users:number;at:number;expires:number;etag?:string;picture?:Picture;job?:Promise<Picture>;controller?:AbortController;failedUntil?:number;retire?:ReturnType<typeof setTimeout>};
export type ArtworkLease={src:string;release:()=>void;invalidate:()=>void};
const stores=new WeakMap<RMusicClient,Map<string,Entry>>();
const owners=new WeakMap<RMusicClient,string>();
const MAX_BYTES=32*1024*1024,MAX_ITEMS=80;
function dispose(p:Picture){if(!p.cached&&!p.refs&&p.src.startsWith('blob:'))URL.revokeObjectURL(p.src);}
function trim(entries:Map<string,Entry>){
 let bytes=[...entries.values()].reduce((n,e)=>n+(e.picture?.bytes||0),0);
 for(const [key,e]of [...entries].sort((a,b)=>a[1].at-b[1].at)){
  if(entries.size<=MAX_ITEMS&&bytes<=MAX_BYTES)break;
  if(e.users||e.job)continue;entries.delete(key);if(e.picture){bytes-=e.picture.bytes;e.picture.cached=false;dispose(e.picture);}
 }
}
export function disposeArtwork(api:RMusicClient){const entries=stores.get(api);stores.delete(api);owners.delete(api);if(entries)for(const e of entries.values()){clearTimeout(e.retire);e.controller?.abort();if(e.picture){e.picture.cached=false;dispose(e.picture);}}entries?.clear();}
let running=0;
async function slot(signal:AbortSignal){while(running>=6){await new Promise<void>((resolve,reject)=>{const done=()=>{clearTimeout(timer);signal.removeEventListener('abort',abort);resolve();};const abort=()=>{clearTimeout(timer);signal.removeEventListener('abort',abort);reject(new DOMException('Cancelled','AbortError'));};const timer=setTimeout(done,30);signal.addEventListener('abort',abort,{once:true});if(signal.aborted)abort();});}signal.throwIfAborted();running++;}
function delay(signal:AbortSignal){return new Promise<void>((resolve,reject)=>{const abort=()=>{clearTimeout(timer);reject(new DOMException('Cancelled','AbortError'));};const timer=setTimeout(()=>{signal.removeEventListener('abort',abort);resolve();},600);signal.addEventListener('abort',abort,{once:true});});}
async function fetchPicture(api:RMusicClient,target:URL,e:Entry,signal:AbortSignal,reload:boolean):Promise<Picture>{
 await slot(signal);
 try{
  if(isNative){
   for(let attempt=0;;attempt++){
    try {
     const src=await nativeCover(api,target.pathname,signal);
     signal.throwIfAborted();
     return {src,bytes:src.length,refs:0,cached:true};
    }catch(error){
     signal.throwIfAborted();
     const status=(error as {status?:number}).status;
     if(attempt || (status && ![408,429,500,502,503,504].includes(status)))throw error;
     await delay(signal);
    }
   }
  }
  const headers:Record<string,string>={};if(api.token)headers.Authorization=`Bearer ${api.token}`;
  if(!reload&&e.picture&&e.etag)headers['If-None-Match']=e.etag;
  for(let attempt=0;;attempt++){
   try{
    const response=await api.fetchResource(target,{headers,signal,...(reload?{cache:'reload' as const}:{})});
    if(response.status===304&&e.picture)return e.picture;
    if(!response.ok)throw Object.assign(new Error('封面暂不可用'),{retryable:[408,500,502,503,504].includes(response.status)});
    const blob=await response.blob();if(blob.size>20*1024*1024)throw Object.assign(new Error('封面过大'),{retryable:false});signal.throwIfAborted();
    e.etag=response.headers?.get('ETag')||undefined;return {src:URL.createObjectURL(blob),bytes:blob.size,refs:0,cached:true};
   }catch(error){if(signal.aborted)throw error;if(attempt||(error as {retryable?:boolean}).retryable===false)throw error;await delay(signal);}
  }
 }finally{running--;}
}
export async function acquireArtwork(api:RMusicClient,url:string,signal:AbortSignal,reload=false):Promise<ArtworkLease>{
 const core=new URL(api.baseUrl||location.href);const target=new URL(url.startsWith('/api/v1/')?api.url(url):url,core);
 if(!['http:','https:'].includes(target.protocol))throw new Error('图片协议无效');
 if(target.origin!==core.origin)return {src:target.href,release(){},invalidate(){}};
 if(api.token&&/\/api\/v1\/(artwork|covers)\/[a-f\d]{64}$/i.test(target.pathname))target.searchParams.delete('ticket');
 if(owners.get(api)!==api.token){disposeArtwork(api);owners.set(api,api.token);}
 const key=target.href;let entries=stores.get(api);if(!entries){entries=new Map();stores.set(api,entries);}let e=entries.get(key);
 if(!e){trim(entries);if(entries.size>=128)throw new Error('图片请求繁忙');e={users:0,at:Date.now(),expires:0};entries.set(key,e);}
 const entry=e;if(entry.revision!==api.artworkRevision){entry.expires=0;entry.revision=api.artworkRevision;}entry.users++;entry.at=Date.now();let released=false,picture:Picture|undefined;
 clearTimeout(entry.retire);
 const retire=()=>{
  if(entry.users||!entry.job)return;
  entry.controller?.abort();
  if(entries!.get(key)===entry){entries!.delete(key);if(entry.picture){entry.picture.cached=false;dispose(entry.picture);}}
 };
 const release=()=>{
  if(released)return;
  released=true;entry.users--;signal.removeEventListener('abort',release);
  if(picture){picture.refs--;dispose(picture);}
  if(!entry.users&&entry.job){
   if(isNative)entry.retire=setTimeout(retire,350);
   else retire();
  }
  trim(entries!);
 };
 signal.addEventListener('abort',release,{once:true});if(signal.aborted){release();signal.throwIfAborted();}
 try{
  if(entry.failedUntil&&entry.failedUntil>Date.now()&&!reload&&!entry.picture)throw new Error('图片暂不可用');
  if(!entry.picture||entry.expires<Date.now()||reload){
   if(!entry.job){const controller=new AbortController();entry.controller=controller;
    entry.job=fetchPicture(api,target,entry,controller.signal,reload).then(value=>{
     if(controller.signal.aborted){value.cached=false;dispose(value);controller.signal.throwIfAborted();}
     if(entry.picture&&entry.picture!==value){entry.picture.cached=false;dispose(entry.picture);}entry.picture=value;entry.expires=Date.now()+(target.pathname.includes('/artwork/')?60000:86400000);entry.failedUntil=undefined;return value;
    }).catch(error=>{entry.failedUntil=Date.now()+15000;if(entry.picture&&!reload){entry.expires=entry.failedUntil;return entry.picture;}throw error;}).finally(()=>{entry.job=undefined;entry.controller=undefined;trim(entries!);});
   }
   picture=await entry.job;
  }else picture=entry.picture;
  signal.throwIfAborted();if(released)throw new DOMException('Cancelled','AbortError');picture!.refs++;
  const leased=picture!;
  return {src:leased.src,release,invalidate:()=>{entry.expires=0;if(entry.picture===leased){entry.picture=undefined;entry.etag=undefined;leased.cached=false;}release();}};
 }catch(error){picture=undefined;release();throw error;}
}
