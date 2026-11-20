import { useEffect,useState,useRef } from "react";
import { Plus,Radio,RefreshCw,Server,Trash2,ArrowRight,ChevronDown,LoaderCircle } from "lucide-react";
import type { ConnectionProfile,CoreAddress,DiscoveredService,RMusicClient } from "@r-music/sdk";
import { invokeNative,isNative,nativeProfiles } from "./native";

export function CoreConnections({api,profiles,onConnect,variant="settings"}:{api:RMusicClient;profiles:ConnectionProfile[];variant?:"settings"|"onboarding";
  onConnect:(url:string,fingerprint?:string,alternatives?:CoreAddress[])=>void|Promise<void>}) {
  const [address,setAddress]=useState(variant==="onboarding"&&!profiles.length?"":api.baseUrl);
  const [fingerprint,setFingerprint]=useState("");
  const [alternatives,setAlternatives]=useState<CoreAddress[]>([]);
  const [items,setItems]=useState<DiscoveredService[]>([]);
  const [available,setAvailable]=useState(true);
  const [error,setError]=useState("");
  const [active,setActive]=useState("");
  const [searching,setSearching]=useState(false);
  const [working,setWorking]=useState(false),[manual,setManual]=useState(variant==="onboarding"&&!profiles.length);
  const alive=useRef(true);useEffect(()=>{alive.current=true;return()=>{alive.current=false;};},[]);
  async function search(){
    setSearching(true);
    try {
      const result=isNative ? await invokeNative<{available:boolean;items:DiscoveredService[]}>("native_discovery")
        : await api.get<{available:boolean;items:DiscoveredService[]}>("/discovery");
      setAvailable(result.available);setItems(result.items.filter(item=>item.kind==="core"));
    } catch {setAvailable(false);} finally {setSearching(false);}
  }
  useEffect(()=>{
    let live=true;
    if(isNative) void nativeProfiles().then(async saved=>{
      const profile=saved.find(p=>p.address.replace(/\/+$/,"")===api.baseUrl.replace(/\/+$/,""));
      if(profile&&live) {setFingerprint(profile.certificate_sha256||"");setAlternatives(profile.alternatives||[]);
        try {const selected=await invokeNative<{address:string}>("native_connection",{profileId:profile.id});if(live)setActive(selected.address);}catch{}
      }
    });
    if(isNative||api.token) {void search();const timer=setInterval(()=>{if(!document.hidden)void search();},3000);
      return ()=>{live=false;clearInterval(timer);};}
    return ()=>{live=false;};
  },[api]);
  function choose(url:string){setAddress(url);setFingerprint("");setAlternatives([]);setError("");}
  async function connect(url=address,pinValue:string|undefined=fingerprint,fallbacks:CoreAddress[]|undefined=alternatives){
    if(working)return;setWorking(true);setError("");
    try {
      for(const item of [{address:url,certificate_sha256:pinValue},...(fallbacks||[])]){
        const parsed=new URL(item.address.trim());
        if(!["http:","https:"].includes(parsed.protocol)||parsed.username||parsed.password||parsed.search||parsed.hash)throw new Error("请输入完整的 HTTP 或 HTTPS 地址，不含账号或参数");
        const pin=item.certificate_sha256?.replace(/[:\s]/g,"");
        if(pin&&(parsed.protocol!=="https:"||!/^[a-fA-F0-9]{64}$/.test(pin)))throw new Error("证书指纹需为 HTTPS 地址的 64 位 SHA-256");
      }
      await onConnect(url.trim().replace(/\/+$/,""),pinValue,fallbacks);
    }catch(error){if(alive.current)setError(error instanceof Error?error.message:"连接失败，请检查地址和网络。");}
    finally{if(alive.current)setWorking(false);}
  }
  const addresses=<>
    <label>核心地址<input type="url" inputMode="url" autoCapitalize="none" autoCorrect="off" spellCheck={false} disabled={working} value={address} onChange={e=>setAddress(e.target.value)} placeholder="http://192.168.1.10:18854"/></label>
    {isNative&&<details className="connection-advanced"><summary>高级连接设置<ChevronDown size={14}/></summary>
      <label>证书 SHA-256 <span className="muted">（可选）</span><input disabled={working} value={fingerprint} onChange={e=>setFingerprint(e.target.value)} placeholder="仅为此地址信任指定证书"/></label>
      {alternatives.map((item,index)=><div className="fallback-address" key={index}>
        <div className="lan-heading"><strong>备用地址 {index+1}</strong><button type="button" disabled={working} className="text-button" aria-label={`删除备用地址 ${index+1}`} onClick={()=>setAlternatives(current=>current.filter((_,i)=>i!==index))}><Trash2 size={16}/></button></div>
        <input disabled={working} type="url" inputMode="url" autoCapitalize="none" spellCheck={false} aria-label={`备用地址 ${index+1}`} value={item.address} placeholder="https://music.example.com" onChange={e=>setAlternatives(current=>current.map((value,i)=>i===index?{...value,address:e.target.value}:value))}/>
        <input disabled={working} aria-label={`备用地址 ${index+1} 证书指纹`} value={item.certificate_sha256||""} placeholder="此地址的证书指纹（可选）" onChange={e=>setAlternatives(current=>current.map((value,i)=>i===index?{...value,certificate_sha256:e.target.value.replace(/[:\s]/g,"")||undefined}:value))}/>
      </div>)}
      <button type="button" className="text-button" disabled={working||alternatives.length>=8} onClick={()=>setAlternatives(current=>[...current,{address:""}])}><Plus size={15}/>添加备用地址</button>
      {active&&<small className="muted">当前连接 · {active}</small>}
    </details>}
    <button type="button" className="primary connection-submit" disabled={working||!address.trim()} onClick={()=>void connect()}>{working?<><LoaderCircle size={18} className="spin"/>正在连接…</>:<>{variant==="onboarding"?"连接这个核心":"保存并连接"}<ArrowRight size={18}/></>}</button>
  </>;
  const recentAddresses = new Set(profiles.map(item=>item.url.replace(/\/+$/,"")));
  const nearby = items.filter(item=>!recentAddresses.has(item.addresses[0]?.replace(/\/+$/,"")));
  if(variant==="onboarding")return <div className="core-connections core-guide" aria-busy={working}>
    {(profiles.length>0)&&<section className="core-guide-section"><h2>最近连接</h2>{profiles.map(profile=><button type="button" className="core-guide-item" key={profile.id} disabled={working} onClick={()=>void connect(profile.url,undefined,profile.alternatives)}><span className="core-guide-symbol"><Server size={21}/></span><span><strong>{profile.name}</strong><small>{profile.url}</small></span><ArrowRight size={17}/></button>)}</section>}
    <section className="core-guide-section"><div className="core-guide-section-heading"><h2>附近的核心</h2><button type="button" className="core-guide-refresh" aria-label="重新搜索核心" disabled={searching||working} onClick={()=>void search()}><RefreshCw size={16} className={searching?"spin":""}/></button></div>
      {nearby.map(item=><button type="button" className="core-guide-item" key={item.id} disabled={working} onClick={()=>void connect(item.addresses[0],"",[])}><span className="core-guide-symbol"><Radio size={21}/></span><span><strong>{item.name}</strong><small>{item.addresses[0]}</small></span><ArrowRight size={17}/></button>)}
      {!nearby.length&&!profiles.length&&<div className="core-guide-empty"><Radio size={24}/><span>{searching?"正在查找附近的核心…":available?"暂未发现附近的核心":"暂时无法自动发现核心"}<small>确认手机与 Core 连接同一网络，也可以手动填写地址。</small></span></div>}
    </section>
    <details className="core-guide-manual" open={manual} onToggle={e=>setManual(e.currentTarget.open)}><summary><Plus size={18}/>手动填写地址<ChevronDown size={16}/></summary><div className="core-guide-addresses">{addresses}</div></details>
    {working&&<p className="core-guide-progress" role="status"><LoaderCircle size={16} className="spin"/>正在确认核心，请稍候…</p>}
    {error&&<div className="form-error" role="alert">{error}</div>}
  </div>;
  return <div className="core-connections">
    {(isNative||api.token)&&<div className="lan-cores"><div className="lan-heading"><strong><Radio size={16}/>局域网核心</strong>
      <button className="text-button" onClick={()=>void search()} aria-label="重新搜索核心" disabled={searching}><RefreshCw size={16}/></button></div>
      {items.map(item=><button className="discovered-core" key={item.id} onClick={()=>choose(item.addresses[0])}>
        <Server size={20}/><span><strong>{item.name}</strong><small>{item.addresses[0]}</small></span></button>)}
      {!items.length&&<span className="muted">{available?"正在查找附近的核心…":"发现暂不可用，请填写地址"}</span>}
    </div>}
    {addresses}
    {error&&<div className="form-error" role="alert">{error}</div>}
    {profiles.length>0&&<div className="saved-cores">{profiles.map(profile=><button className="text-button" key={profile.id} onClick={()=>void connect(profile.url,undefined,profile.alternatives)}>{profile.name}<small>{profile.url}</small></button>)}</div>}
  </div>;
}
