import {outputLabel,zoneDisplayName} from "./outputLabels";
import {Check,Speaker,Users,Layers3} from "lucide-react";
import type {Endpoint,RMusicClient,Zone} from "@r-music/sdk";
import {Modal} from "./components";
import {presentedZones} from "./deviceGroupModel";

export function ZonePicker({labels={},zones,endpoints,selected,familySync=true,compact=false,onSelect,onManage,close}:{labels?:Record<string,string>;api:RMusicClient;zones:Zone[];endpoints:Endpoint[];selected:string;definitions:boolean;familySync?:boolean;compact?:boolean;onSelect:(id:string)=>void;onGroup:(ids:string[])=>void;onManage:()=>void;close:()=>void}) {
 const targets=presentedZones(zones,endpoints).map(zone=>({zone,endpoint:endpoints.find(item=>item.id===zone.endpoint_id)}));
 const online=targets.filter(target=>target.endpoint?.online===true).sort((a,b)=>Number(b.zone.id===selected)-Number(a.zone.id===selected));
 const offline=targets.filter(target=>target.endpoint?.online!==true);
 const row=({zone,endpoint}:typeof targets[number])=>{
  const connected=endpoint?.online===true,active=zone.id===selected;
  const label=outputLabel(zone,endpoint,labels),name=zoneDisplayName(zone);
  const status=connected?zone.status==="playing"?"正在播放":active&&compact?"当前设备":"在线":"离线";
  return <button key={zone.id} aria-pressed={active} disabled={!connected} onClick={()=>onSelect(zone.id)}>
   {endpoint?.kind==="group"?<Layers3 size={20}/>:<Speaker size={20}/>}
   <span><strong>{label}</strong><small>{compact?status:`${name} · ${status}`}</small></span>
   {active&&<Check size={18}/>}
  </button>;
 };
 return <Modal title="选择播放设备" onClose={close}>
  <div className={`zone-picker-list${compact?" zone-picker-compact":""}`}>
   {(compact?online:targets).map(row)}
   {compact&&!online.length&&<p className="muted">暂无在线播放设备</p>}
  </div>
  {compact&&offline.length>0&&<details className="zone-picker-offline">
   <summary>离线设备 · {offline.length}</summary><div className="zone-picker-list zone-picker-compact">{offline.map(row)}</div>
  </details>}
  {familySync&&<div className="zone-picker-groups"><button onClick={onManage}><Users size={18}/>管理设备组合</button></div>}
 </Modal>;
}
