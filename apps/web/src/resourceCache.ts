import type {RMusicClient} from "@r-music/sdk";
import {useEffect,useState} from "react";
type Entry={value?:unknown;expires:number;pending?:Promise<unknown>};
const resources=new WeakMap<RMusicClient,{token:string;map:Map<string,Entry>}>();
function entries(api:RMusicClient){let saved=resources.get(api);if(!saved||saved.token!==api.token){saved={token:api.token,map:new Map()};resources.set(api,saved);}return saved.map;}
// Mounted views revalidate their own TTLs, without polling hidden pages or bypassing pacing.
export function useResourceRefresh(interval=30000){
 const [revision,setRevision]=useState(0);
 useEffect(()=>{const update=()=>{if(!document.hidden)setRevision(v=>v+1);};const timer=setInterval(update,interval);window.addEventListener("focus",update);window.addEventListener("online",update);document.addEventListener("visibilitychange",update);return()=>{clearInterval(timer);window.removeEventListener("focus",update);window.removeEventListener("online",update);document.removeEventListener("visibilitychange",update);};},[interval]);return revision;
}
export function cachedResource<T>(api:RMusicClient,key:string):T|undefined{return entries(api).get(key)?.value as T|undefined;}
export function invalidateResources(api:RMusicClient){resources.delete(api);}
export async function loadResource<T>(api:RMusicClient,key:string,load:()=>Promise<T>,ttl:number,force=false):Promise<T>{
 const map=entries(api),old=map.get(key);
 if(old?.pending)return old.pending as Promise<T>;
 if(!force&&old?.value!==undefined&&old.expires>Date.now())return old.value as T;
 const entry:Entry={value:old?.value,expires:0};map.set(key,entry);
 if(map.size>128){for(const [name,value] of map){if(name!==key&&!value.pending){map.delete(name);break;}}}
 const pending=load().then(value=>{if(map.get(key)===entry){entry.value=value;entry.expires=Date.now()+ttl;}return value;}).finally(()=>{if(map.get(key)===entry)entry.pending=undefined;});entry.pending=pending;return pending;
}
