import {Mark} from "./studio/StudioShell";
import {useEffect,useRef,useState,type FormEvent} from "react";
import {ArrowLeft,ArrowRight,Check,ChevronDown,Eye,EyeOff,Server,ShieldCheck} from "lucide-react";
import {RMusicClient,type AuthResult,type ConnectionProfile,type CoreAddress} from "@r-music/sdk";
import {isNative,NativeClient,saveNativeProfile} from "./native";
import {CoreConnections} from "./CoreConnections";
import "./Auth.css";

type LoginClient={id:string;name:string;type:string};
type Props={loginClient?:LoginClient;api:RMusicClient;initialized:boolean;onAuth:(result:AuthResult)=>void;profiles:ConnectionProfile[];onConnect:(url:string,fingerprint?:string,alternatives?:CoreAddress[])=>void|Promise<void>;onOffline:()=>void};
type Confirmed={address:string;name:string;core_id:string;initialized:boolean};
export function Auth({api,initialized,onAuth,profiles,onConnect,onOffline,loginClient}:Props){
 const[username,setUsername]=useState(""),[password,setPassword]=useState(""),[bootstrap,setBootstrap]=useState(""),[totp,setTotp]=useState("");
 const[error,setError]=useState(""),[busy,setBusy]=useState(false),[visible,setVisible]=useState(false),[twoFactor,setTwoFactor]=useState(false);
 const[step,setStep]=useState<"core"|"account">("core"),[confirmed,setConfirmed]=useState<Confirmed>();
 const pending=useRef<AbortController|null>(null);
 useEffect(()=>()=>pending.current?.abort(),[]);
 const accountInitialized=isNative&&confirmed?confirmed.initialized:initialized;
 const canSubmit=!isNative||confirmed?.address.replace(/\/+$/,"")===api.baseUrl.replace(/\/+$/,"");
 function back(){setStep("core");setPassword("");setTotp("");setBootstrap("");setError("");setVisible(false);}
 useEffect(()=>{if(!isNative||step!=="account")return;const goBack=(event:Event)=>{if(document.querySelector('[role="dialog"][aria-modal="true"]'))return;event.preventDefault();back();};window.addEventListener("rmusic-system-back",goBack);return()=>window.removeEventListener("rmusic-system-back",goBack);},[step]);
 async function chooseCore(address:string,fingerprint?:string,alternatives?:CoreAddress[]){
  pending.current?.abort();const controller=new AbortController();pending.current=controller;
  const timeout=setTimeout(()=>controller.abort(new Error("连接超时，请检查地址和网络后重试。")),10000);
  try{
   await saveNativeProfile(address,fingerprint,alternatives);controller.signal.throwIfAborted();
   const value=await new NativeClient(address).request<{core_id?:string;name?:string;initialized?:boolean}>("/status",{signal:controller.signal});
   controller.signal.throwIfAborted();
   if(!value.core_id||typeof value.initialized!=="boolean")throw new Error("这个地址没有返回有效的 R-Music 核心，请检查地址与端口。");
   await onConnect(address,fingerprint,alternatives);controller.signal.throwIfAborted();
   setConfirmed({address,name:value.name||"R-Music Core",core_id:value.core_id,initialized:value.initialized});
   setUsername("");setPassword("");setTotp("");setBootstrap("");setError("");setVisible(false);setTwoFactor(false);setStep("account");
  }finally{clearTimeout(timeout);if(pending.current===controller)pending.current=null;}
 }
 async function submit(event:FormEvent){
  event.preventDefault();if(busy||!canSubmit)return;setBusy(true);setError("");
  try{onAuth(await api.post<AuthResult>(accountInitialized?"/auth/login":"/setup",accountInitialized?{username,password,totp:totp||undefined,client:loginClient}:{username,password,bootstrap_token:bootstrap}));}
  catch(cause){const message=cause instanceof Error?cause.message:"登录失败，请稍后重试。";setError(message);if(/两步|验证码|TOTP|2FA/i.test(message))setTwoFactor(true);}
  finally{setBusy(false);}
 }
 const form=<form onSubmit={submit} className="auth-account-form">
  {!accountInitialized&&<label>初始化凭证<input required value={bootstrap} onChange={e=>setBootstrap(e.target.value)} autoComplete="off" placeholder="核心 bootstrap.token 中的凭证"/></label>}
  <label>用户名<input required value={username} onChange={e=>setUsername(e.target.value)} autoComplete="username" autoCapitalize="none" autoCorrect="off" spellCheck={false} placeholder="输入用户名"/></label>
  <label>密码<div className="auth-password"><input required minLength={accountInitialized?1:12} type={visible?"text":"password"} value={password} onChange={e=>setPassword(e.target.value)} autoComplete={accountInitialized?"current-password":"new-password"} placeholder={accountInitialized?"输入密码":"至少 12 位密码"}/>{isNative&&<button type="button" aria-label={visible?"隐藏密码":"显示密码"} aria-pressed={visible} onClick={()=>setVisible(v=>!v)}>{visible?<EyeOff size={19}/>:<Eye size={19}/>}</button>}</div></label>
  {accountInitialized&&(isNative?<details className="auth-two-factor" open={twoFactor} onToggle={event=>setTwoFactor(event.currentTarget.open)}><summary><ShieldCheck size={16}/>使用两步验证<ChevronDown size={15}/></summary><label>验证码或恢复码<input value={totp} onChange={e=>setTotp(e.target.value)} autoComplete="one-time-code" autoCapitalize="none" autoCorrect="off" placeholder="输入验证码或恢复码"/></label></details>:<label>两步验证码 <span className="muted">（如已启用）</span><input value={totp} onChange={e=>setTotp(e.target.value)} autoComplete="one-time-code" placeholder="验证码或恢复码"/></label>)}
  {error&&<div className="form-error" role="alert">{error}</div>}
  <button className="primary full" disabled={busy||!canSubmit}>{busy?"正在登录…":accountInitialized?"登录":"创建管理员"}<ArrowRight size={18}/></button>
 </form>;
 if(!isNative)return <main className="studio-gate"><section className="studio-gate-art" aria-hidden="true"><div className="studio-gate-disc"/><div className="studio-poster-type"><Mark/><span>R–Music</span></div><div className="studio-poster-stripes"/></section><section className="studio-gate-form"><div className="studio-brand"><Mark/><b>R–Music</b></div><h1>{accountInitialized?"欢迎回来":"创建管理员账号"}</h1>{!accountInitialized&&<p>创建第一个账号，管理你的 Core。</p>}{form}<button className="studio-offline" onClick={onOffline}>播放本机离线音乐</button></section></main>;
 return <div className="auth-screen native-auth" data-step={step}>
  <header className="native-auth-header"><div className="native-auth-brand"><Mark/><strong>R-Music</strong></div><nav aria-label="连接步骤"><span aria-current={step==="core"?"step":undefined}>{step==="account"?<Check size={13}/>:"1"}</span><i/><span aria-current={step==="account"?"step":undefined}>2</span></nav></header>
  <main className="native-auth-main" key={step}>
   {step==="core"?<>
    <div className="native-auth-heading"><h1>选择音乐核心</h1><p>选择已发现的 Core，或手动填写连接地址。</p></div>
    <CoreConnections variant="onboarding" api={api} profiles={profiles} onConnect={chooseCore}/>
   </>:<>
    <button className="native-auth-back" type="button" onClick={back} disabled={busy}><ArrowLeft size={18}/>更换核心</button>
    <div className="native-auth-heading"><h1>{accountInitialized?"欢迎回来":"创建你的账号"}</h1><p>{accountInitialized?"登录到已选的 Core。":"为这台 Core 创建管理员账号。"}</p></div>
    <section className="native-auth-core" aria-label="当前音乐核心"><span><Server size={20}/></span><div><strong>{confirmed?.name}</strong><small>{confirmed?.address}</small></div><Check size={16}/></section>
    {form}
   </>}
  </main>
  <footer className="native-auth-footer"><button type="button" onClick={onOffline}>播放本机离线音乐<ArrowRight size={15}/></button></footer>
 </div>;
}
