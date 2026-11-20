import type {GroupClockDiagnostics as Diagnostics,Zone} from "@r-music/sdk";
import {useEffect,useState} from "react";

export function GroupClockDiagnostics({value,deadline,zones}:{value?:Diagnostics;deadline?:number;zones:Zone[]}) {
 const [,render]=useState(0);
 useEffect(()=>{if(deadline===undefined)return;const timeout=setTimeout(()=>render(n=>n+1),Math.max(0,deadline-performance.now()));return()=>clearTimeout(timeout);},[deadline]);
 const expired=!!value&&["available","partial"].includes(value.state)&&(deadline===undefined||performance.now()>=deadline);
 const shown=expired?undefined:value;
 const name=(id:string)=>zones.find(zone=>zone.id===id)?.name||"音区";
 return <details className="fine-print"><summary>同步诊断（开发）</summary>
  {expired?<p>反馈已过期，等待更新。</p>:!shown||shown.state==="unavailable"?<p>等待可比较的设备反馈。</p>:shown.state==="inactive"?<p>播放后可比较输出。</p>:shown.state==="single_output"?<p>当前只有一个输出，暂无相对偏差。</p>:<p>参考输出：{shown.reference_zone_id?name(shown.reference_zone_id):"等待反馈"}</p>}
  {shown?.members.map(member=><p key={member.zone_id}>{name(member.zone_id)} · {!member.available?member.reason||"反馈暂不可用":member.zone_id===shown.reference_zone_id?"参考输出":typeof member.estimated_offset_ns==="number"?`估计${member.estimated_offset_ns>=0?"领先":"落后"} ${(Math.abs(member.estimated_offset_ns)/1e6).toLocaleString("zh-CN",{maximumFractionDigits:3})} 毫秒`:"等待比较"}{member.available&&member.zone_id!==shown.reference_zone_id&&(typeof member.relative_drift_ppm==="number"?` · 相对速率 ${member.relative_drift_ppm>=0?"+":""}${member.relative_drift_ppm.toLocaleString("zh-CN",{maximumFractionDigits:1})} ppm`:" · 速率待观察")}</p>)}
  {shown?.members.some(member=>member.projection==="nominal")&&<p>部分输出尚在观察速率，当前采用标称速率估计。</p>}
  <p>这是设备反馈估计，驱动与声学精度尚未验证；{value?.correction==="experimental"?"已选择实验自动校正，最近状态见分组面板；严格同步尚未验证。":"当前未启用补偿。"}</p>
 </details>;
}
