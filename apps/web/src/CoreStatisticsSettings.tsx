import {useEffect,useState} from "react";
import type {RMusicClient} from "@r-music/sdk";
export function CoreStatisticsSettings({api,onError,notify}:{api:RMusicClient;onError:(error:unknown)=>void;notify:(message:string)=>void}){
 const[value,setValue]=useState<string>(),[saving,setSaving]=useState(false);
 useEffect(()=>{const abort=new AbortController();void api.request<{statistics_timezone?:string}>("/settings",{signal:abort.signal}).then(v=>{if(!abort.signal.aborted)setValue(v.statistics_timezone||"Asia/Shanghai");}).catch(e=>{if(!abort.signal.aborted)onError(e);});return()=>abort.abort();},[api,onError]);
 const zones=Array.from(new Set(["Asia/Shanghai",...(Intl.supportedValuesOf?.("timeZone")||[]),"UTC",value||"Asia/Shanghai"]));
 return <section className="settings-section"><h2>聆听统计</h2><p>统一设置连接此核心的客户端如何划分日期。默认上海，不更改已保存的聆听时长。</p><label>统计时区<select aria-label="核心统计时区" disabled={!value||saving} value={value||"Asia/Shanghai"} onChange={e=>setValue(e.target.value)}>{zones.map(zone=><option key={zone} value={zone}>{zone==="Asia/Shanghai"?"上海 · Asia/Shanghai":zone.replaceAll("_"," ")}</option>)}</select></label><button disabled={!value||saving} onClick={async()=>{setSaving(true);try{await api.patch("/settings",{statistics_timezone:value});notify("核心统计设置已保存");}catch(e){onError(e);}finally{setSaving(false);}}}>{saving?"正在保存…":"保存统计设置"}</button></section>;
}
