import {useCallback,useEffect,useRef,useState} from "react";
import type {ConnectionMetrics,RMusicClient} from "@r-music/sdk";
import {Activity,ArrowDown,Radio} from "lucide-react";
import {isNative} from "./native";
import "./ConnectionQuality.css";

const modes:Record<string,string>={local:"本机直连",lan:"局域网直连",vpn:"VPN／专网",private:"私网直连",https:"HTTPS 直连",reverse_proxy:"HTTPS 反代",direct:"HTTP 直连"};
export function ConnectionQuality({api}:{api:RMusicClient}){
 const [value,setValue]=useState<ConnectionMetrics>(),[busy,setBusy]=useState<"ping"|"speed"|null>(null),[error,setError]=useState("");
 const current=useRef<AbortController|null>(null);
 const measure=useCallback(async(speed:boolean)=>{
  if(current.current)return;const controller=new AbortController();current.current=controller;setBusy(speed?"speed":"ping");setError("");
  const timeout=window.setTimeout(()=>controller.abort(new Error("测试超时")),speed?48000:20000);
  try{
   const next=await api.connectionMetrics(speed,controller.signal);
   if(current.current===controller&&!controller.signal.aborted)setValue(old=>!speed&&old?.mode===next.mode&&old?.address===next.address?{...old,...next}:next);
  }catch(e){if(current.current===controller){setValue(undefined);setError(controller.signal.aborted?"测试已中止或超时":e instanceof Error?e.message:"无法测量当前连接");}}
  finally{window.clearTimeout(timeout);if(current.current===controller){current.current=null;setBusy(null);}}
 },[api]);
 useEffect(()=>{
  setValue(undefined);setError("");setBusy(null);
  const stop=()=>{const c=current.current;current.current=null;c?.abort();};
  const visible=()=>{if(document.hidden){stop();setBusy(null);}else void measure(false);};
  visible();const timer=setInterval(()=>{if(!document.hidden)void measure(false);},60000);
  const networkChanged=()=>{stop();setValue(undefined);visible();};
  document.addEventListener("visibilitychange",visible);
  if(isNative)window.addEventListener("rmusic-network-change",networkChanged);
  return()=>{stop();clearInterval(timer);document.removeEventListener("visibilitychange",visible);window.removeEventListener("rmusic-network-change",networkChanged);};
 },[measure]);
 return <div className="connection-quality">
  <div className="quality-line"><span><Radio size={17}/><strong>{value?(modes[value.mode]||"连接方式待识别"):"当前连接"}</strong></span><span className="quality-latency"><Activity size={15}/>{value?Math.round(value.latency_ms)+" ms":busy?"测量中…":"延迟未知"}</span></div>
  {value&&<div className="quality-address">{value.address}{value.underlay==="vpn"&&value.mode!=="vpn"?" · 经 VPN":""}{value.end_to_end?" · 端到端加密":""}</div>}
  <div className="quality-actions"><button type="button" disabled={busy!==null} onClick={()=>void measure(true)}>{busy==="speed"?"正在测速…":"测速"}</button><small>{value?.down_mbps!=null?<><ArrowDown size={14}/>{value.down_mbps.toFixed(1)} Mbps</>:"下载 1 MiB · 应用往返延迟"}</small></div>
  {error&&<small role="status">{error}</small>}
 </div>;
}
