import {useEffect,useState} from "react";
import {Wifi,Signal} from "lucide-react";
import {invokeNative} from "./native";
const choices=[['original','原始音质','保留音源规格，超过输出上限时兼容转换'],['lossless','标准无损','最高 48 kHz / 16 bit，保持原采样率家族'],['hires','高解析无损','最高 192 kHz / 24 bit，不升频']] as const;
type Profiles={wifi:string;mobile:string;network:string};
export function AndroidQuality({onError,notify}:{onError:(e:unknown)=>void;notify:(message:string)=>void}){
 const[profiles,setProfiles]=useState<Profiles|null>(null),[busy,setBusy]=useState(false);
 useEffect(()=>{let active=true;void invokeNative<Profiles>('native_android_quality').then(v=>{if(active)setProfiles({...v,wifi:choices.some(([id])=>id===v.wifi)?v.wifi:'original',mobile:choices.some(([id])=>id===v.mobile)?v.mobile:'original'});}).catch(onError);return()=>{active=false;};},[]);
 const save=async()=>{if(!profiles)return;setBusy(true);try{const saved=await invokeNative<Profiles>('native_android_quality',{values:{wifi:profiles.wifi,mobile:profiles.mobile}});setProfiles(saved);notify('Wi-Fi 与移动流量音质已保存，下次点播生效');}catch(e){onError(e);}finally{setBusy(false);}};
 return <section className="settings-section network-quality"><h2>在线播放音质</h2><p>Wi-Fi 和移动流量分别选择，按当前网络应用。播放中切网保留当前音频流。</p>{(['wifi','mobile'] as const).map(key=><label key={key}><span>{key==='wifi'?<Wifi size={20}/>:<Signal size={20}/>} {key==='wifi'?'Wi-Fi':'移动流量'}</span><select aria-label={`${key==='wifi'?'Wi-Fi':'移动流量'}音质`} disabled={!profiles||busy} value={profiles?.[key]||'original'} onChange={e=>setProfiles(v=>v?{...v,[key]:e.target.value}:v)}>{choices.map(([id,name])=><option key={id} value={id}>{name}</option>)}</select><small>{choices.find(v=>v[0]===profiles?.[key])?.[2]}</small></label>)}<small>外网与 VPN 使用有界预取：当前曲目最多 8 MiB，计费网络下一首最多 2 MiB。原始文件按实际收到的字节缓存；转换流不混入原文件缓存。</small><small>外出使用无损压缩，最高 768 kHz / 32 bit，遵循设备输出上限。DSD 在 Core 解码为 PCM。</small><button className="primary" disabled={!profiles||busy} onClick={()=>void save()}>{busy?'正在保存…':'保存音质选择'}</button></section>;
}
