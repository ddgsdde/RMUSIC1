import type {Recommendations} from "@r-music/sdk";
export type DiscoverySnapshot={value:Recommendations;expires:number};
const prefix="rmusic.discovery.v1:";
export function readDiscovery(key:string):DiscoverySnapshot|undefined{
 try {const value=JSON.parse(localStorage.getItem(prefix+key)||"null");if(value&&Array.isArray(value.value?.items)&&typeof value.expires==="number")return value;}catch{}
}
export function saveDiscovery(key:string,value:Recommendations){
 try {
  const keys=Object.keys(localStorage).filter(key=>key.startsWith(prefix));
  if(keys.length>=8&&!keys.includes(prefix+key))localStorage.removeItem(keys.sort((a,b)=>(readDiscovery(a.slice(prefix.length))?.expires||0)-(readDiscovery(b.slice(prefix.length))?.expires||0))[0]);
  localStorage.setItem(prefix+key,JSON.stringify({value,expires:value.fallback?Date.now()+60000:new Date().setHours(24,0,0,0)}));
 }catch{}
}
