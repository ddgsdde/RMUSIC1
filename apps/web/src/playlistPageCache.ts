import type {RMusicClient} from "@r-music/sdk";
type Entry={value:unknown;at:number};
const clients=new WeakMap<RMusicClient,{token:string;pages:Map<string,Entry>}>();
function pages(api:RMusicClient){let saved=clients.get(api);if(!saved||saved.token!==api.token){saved={token:api.token,pages:new Map()};clients.set(api,saved);}return saved.pages;}
export function playlistPage<T>(api:RMusicClient,key:string):{value:T;fresh:boolean}|undefined {const saved=pages(api).get(key);if(saved)return {value:saved.value as T,fresh:Date.now()-saved.at<30000};}
export function rememberPlaylistPage(api:RMusicClient,key:string,value:unknown){const cache=pages(api);cache.delete(key);cache.set(key,{value,at:Date.now()});if(cache.size>40)cache.delete(cache.keys().next().value!);}

export function invalidatePlaylistPages(api:RMusicClient){for(const page of pages(api).values())page.at=0;}
