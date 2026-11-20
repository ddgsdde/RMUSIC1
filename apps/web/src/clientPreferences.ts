import type { Preferences } from "./App";
function withoutRemovedEffects(value:Partial<Preferences>):Partial<Preferences>{
 const {mineradio:_,...rest}=value as Partial<Preferences>&{mineradio?:unknown};
 return {...rest,...(rest.theme==="mineradio"?{theme:"graphite"}:{}),...((rest.player_skin as string)==="mineradio"?{player_skin:"silk" as const}:{})};
}
const key="rmusic.client-preferences.v1";
function stored(name:string):Partial<Preferences>|null {
 try {const value=JSON.parse(localStorage.getItem(name)||"null");return value&&typeof value==="object"&&!Array.isArray(value)?value:null;}catch{return null;}
}
export function readClientPreferences(defaults:Preferences):Preferences {
 return {...defaults,...withoutRemovedEffects(stored(key)||stored("rmusic.preferences")||{})};
}
export function migrateClientPreferences(legacy:Partial<Preferences>,defaults:Preferences):Preferences {
 const value={...defaults,...withoutRemovedEffects(stored(key)||stored("rmusic.preferences")||legacy)};
 saveClientPreferences(value);return value;
}
export function saveClientPreferences(value:Preferences){localStorage.setItem(key,JSON.stringify(withoutRemovedEffects(value)));}
