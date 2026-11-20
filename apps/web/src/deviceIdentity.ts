import type {Zone} from "@r-music/sdk";
export function preferredZone(zones:Zone[],current:string,localEndpoint:string,explicit:string):string {
 if(explicit&&zones.some(zone=>zone.id===explicit))return explicit;
 return zones.find(zone=>zone.endpoint_id===localEndpoint)?.id||zones.find(zone=>zone.id===current)?.id||zones[0]?.id||"";
}
export function browserDeviceName(id:string):string {
 const ua=navigator.userAgent,platform=/Android/i.test(ua)?"Android":/iPhone|iPad/i.test(ua)?"iOS":/Windows/i.test(ua)?"Windows":/Mac/i.test(ua)?"Mac":"Linux";
 const browser=/Edg\//.test(ua)?"Edge":/Firefox\//.test(ua)?"Firefox":/Chrome\//.test(ua)?"Chrome":"浏览器";
 return `${platform} ${browser} · ${id.slice(-4)}`;
}
export function clientDeviceName(fallback:string){return localStorage.getItem("rmusic.device-name")?.trim()||fallback;}
