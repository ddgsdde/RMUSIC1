import {useEffect,useState} from "react";
import type {Endpoint,RMusicClient,Zone} from "@r-music/sdk";
import type {AudioDevice,DeviceOutput} from "./native";

export function outputName(output:Pick<DeviceOutput,"name"|"kind">):string {
 return output.kind==="browser"&&["系统音频输出","本机",""].includes(output.name)?"浏览器输出":output.name;
}
export function deviceOutputLabels(devices:AudioDevice[]):Record<string,string> {
 return Object.fromEntries(devices.flatMap(device=>device.outputs.map(output=>[output.id,[device.name,outputName(output)].filter(Boolean).join(" · ")])));
}
export function zoneDisplayName(zone:Zone):string {
 // Only migrate the built-in placeholder; user-created/custom zone names are preserved.
 return zone.id==="zone-local"&&zone.name==="本机"&&zone.endpoint_id.startsWith("browser-")?"浏览器输出":zone.name;
}
export function outputLabel(zone:Zone|undefined,endpoint:Endpoint|undefined,labels:Record<string,string>):string {
 if(!zone)return "选择音区";
 if(labels[zone.endpoint_id])return labels[zone.endpoint_id];
 if(endpoint?.kind==="browser"&&endpoint.capabilities?.platform!=="android")return [endpoint.name,"浏览器输出"].filter(Boolean).join(" · ");
 return endpoint?.name||zoneDisplayName(zone);
}

// No playback-rate polling: reload only for endpoint inventory changes or opening the picker.
export function useOutputLabels(api:RMusicClient,enabled:boolean,revision:string) {
 const [snapshot,setSnapshot]=useState<{api:RMusicClient;labels:Record<string,string>}>();
 useEffect(()=>{
  if(!enabled){setSnapshot(undefined);return;}
  const controller=new AbortController();
  void api.request<{items:AudioDevice[]}>("/devices",{signal:controller.signal}).then(value=>{
   if(!controller.signal.aborted)setSnapshot({api,labels:deviceOutputLabels(value.items)});
  }).catch(()=>{});
  return()=>controller.abort();
 },[api,enabled,revision]);
 return enabled&&snapshot?.api===api?snapshot.labels:{};
}
