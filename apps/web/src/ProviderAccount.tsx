import "./ProviderAccount.css";
import {useEffect,useState} from "react";
import type {ProviderManifest,RMusicClient} from "@r-music/sdk";
import {errorMessage} from "./errorRecovery";

type Health={authenticated:boolean;login_pending?:boolean;auth_status?:string};
export function ProviderAccount({api,plugin,notify}:{api:RMusicClient;plugin:ProviderManifest;notify:(message:string)=>void}){
 const [health,setHealth]=useState<Health|null>(null),[error,setError]=useState(""),[checking,setChecking]=useState(true),[working,setWorking]=useState(false),[revision,setRevision]=useState(0);
 useEffect(()=>{
  if(!plugin.enabled)return;
  const controller=new AbortController();setChecking(true);setError("");
  api.request<Health>(`/plugins/${encodeURIComponent(plugin.id)}/health`,{signal:controller.signal}).then(value=>{if(!controller.signal.aborted)setHealth(value);}).catch(error=>{if(!controller.signal.aborted)setError(errorMessage(error));}).finally(()=>{if(!controller.signal.aborted)setChecking(false);});
  return()=>controller.abort();
 },[api,plugin.id,plugin.enabled,plugin.account_context,revision]);
 if(!plugin.enabled||!plugin.capabilities.some(cap=>["login","qr_login","auth"].includes(cap)))return null;
 const label=checking?"正在检查登录状态…":error?"登录状态检查失败":health?.login_pending?"等待扫码确认":health?.auth_status==="expired"?"登录已过期，请重新登录":health?.authenticated?"已登录":"未登录";
 return <div className="provider-account"><strong role="status" data-authenticated={!!health?.authenticated}>{label}</strong><button disabled={checking||working} onClick={()=>setRevision(value=>value+1)}>刷新状态</button>{plugin.capabilities.includes("logout")&&(health?.authenticated||health?.login_pending)&&<button disabled={working} onClick={async()=>{setWorking(true);setError("");try{setHealth(await api.post<Health>(`/providers/${encodeURIComponent(plugin.id)}/logout`));notify(`${plugin.name} 已退出登录，本地音乐与歌单保留`);}catch(error){setError(errorMessage(error));}finally{setWorking(false);}}}>退出音源账号</button>}{error&&<small role="alert">{error}</small>}</div>;
}
