import {useState} from "react";
import type {Endpoint,RMusicClient,Zone} from "@r-music/sdk";

export function PlaybackSettings({api,zone,endpoint,refresh,onError}:{api:RMusicClient;admin?:boolean;zone?:Zone;endpoint?:Endpoint;refresh:()=>Promise<unknown>;notify?:(message:string)=>void;onError:(error:unknown)=>void}){
 const [saving,setSaving]=useState(false),[saved,setSaved]=useState(false);
 async function changeFade(key:"fade_in_ms"|"fade_out_ms",value:number){
  if(!zone||saving)return;setSaving(true);setSaved(false);
  try{await api.patch(`/zones/${zone.id}`,{transitions:{fade_in_ms:zone.transitions?.fade_in_ms||0,fade_out_ms:zone.transitions?.fade_out_ms||0,[key]:value}});await refresh();setSaved(true);}catch(error){onError(error);}finally{setSaving(false);}
 }
 return <>
  {endpoint?.capabilities.transitions!==false&&<section className="settings-section playback-settings"><h3>淡入淡出</h3>
   {([['fade_in_ms','曲首淡入'],['fade_out_ms','曲尾淡出']] as const).map(([key,label])=><label key={key}>{label}<select aria-label={label} value={zone?.transitions?.[key]||0} disabled={!zone||saving} onChange={event=>changeFade(key,Number(event.target.value))}>
    {[0,250,500,1000,2000,3000,5000,10000].map(value=><option key={value} value={value}>{value?`${value/1000} 秒`:"关闭"}</option>)}
   </select></label>)}
  {saved&&<small role="status">已保存到此输出</small>}</section>}
 </>;
}
