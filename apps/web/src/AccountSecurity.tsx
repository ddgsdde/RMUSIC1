import {useEffect, useState, type FormEvent} from "react";
import type {RMusicClient, User} from "@r-music/sdk";
import {Modal} from "./components";

export function AccountSecurity({api,user,onUserChange,notify,onError}:{api:RMusicClient;user:User;onUserChange?:(user:User)=>void;notify:(message:string)=>void;onError:(error:unknown)=>void}) {
  const [account,setAccount]=useState(user);
  const [name,setName]=useState(user.username);
  const [current,setCurrent]=useState("");
  const [password,setPassword]=useState("");
  const [confirm,setConfirm]=useState("");
  const [code,setCode]=useState("");
  const [securityPassword,setSecurityPassword]=useState("");
  const [securityCode,setSecurityCode]=useState("");
  const [pending,setPending]=useState<{secret:string;uri:string}|null>(null);
  const [enrollmentCode,setEnrollmentCode]=useState("");
  const [recovery,setRecovery]=useState<string[]>([]);
  const [busy,setBusy]=useState(false);
  useEffect(()=>{setAccount(user);setName(user.username);},[user]);
  // Login results from older cores omit totp_enabled; refresh from the authenticated account.
  useEffect(()=>{let active=true;api.get<User>("/auth/me").then(value=>{if(active){setAccount(value);setName(value.username);}}).catch(onError);return()=>{active=false;};},[api,user.id]);
  const update=(next:User)=>{setAccount(next);onUserChange?.(next);};
  const copy=async(value:string,message:string)=>{try{
    if(navigator.clipboard?.writeText)await navigator.clipboard.writeText(value);
    else{
      const active=document.activeElement,field=document.createElement("textarea");field.value=value;field.style.cssText="position:fixed;opacity:0;pointer-events:none";document.body.append(field);
      try{field.select();if(!document.execCommand("copy"))throw new Error("请选中文字并手动复制");}finally{field.remove();if(active instanceof HTMLElement)active.focus();}
    }
    notify(message);
  }catch(error){onError(error);}};
  const run=async(work:()=>Promise<void>)=>{if(busy)return;setBusy(true);try{await work();}catch(error){onError(error);}finally{setBusy(false);}};
  const save=(event:FormEvent)=>{event.preventDefault();if(password!==confirm){notify("两次输入的新密码不一致");return;}void run(async()=>{
    const result=await api.patch<{user:User}>("/auth/account",{current_password:current,totp:code||undefined,...(account.role==="admin"&&name.trim()!==account.username?{username:name.trim()}:{}),...(password?{new_password:password}:{})});
    update(result.user);setCurrent("");setPassword("");setConfirm("");setCode("");setPending(null);notify("账号信息已更新，其他登录会话和访问凭证已撤销");
  });};
  return <>
    <section className="settings-section">
      <h2>账号与密码</h2>
      <p className="muted">{account.username} · {account.role==="admin"?"管理员":account.role==="external"?"外部账号":"家庭成员"}。修改后保留当前登录，其他设备需重新登录。</p>
      <form className="account-security-form" onSubmit={save}>
        {account.role==="admin"&&<label>登录账号<input required autoComplete="username" maxLength={80} value={name} onChange={event=>setName(event.target.value)}/></label>}
        <label>当前密码<input required type="password" autoComplete="current-password" maxLength={1024} value={current} onChange={event=>setCurrent(event.target.value)}/></label>
        <label>新密码{account.role==="admin"?"（只修改账号时可留空）":""}<input required={account.role!=="admin"} type="password" autoComplete="new-password" minLength={10} maxLength={1024} placeholder="至少 10 个字符" value={password} onChange={event=>setPassword(event.target.value)}/></label>
        <label>确认新密码<input required={!!password} type="password" autoComplete="new-password" maxLength={1024} value={confirm} onChange={event=>setConfirm(event.target.value)}/></label>
        {account.totp_enabled&&<label>两步验证码或恢复码<input required autoComplete="one-time-code" maxLength={128} value={code} onChange={event=>setCode(event.target.value)}/></label>}
        <button className="primary" disabled={busy||(!password&&name.trim()===account.username)}>保存账号与密码</button>
      </form>
    </section>
    <section className="settings-section">
      <h2>两步验证（2FA）</h2>
      <p className="muted">{account.totp_enabled?"已启用。登录时需要验证器动态码，也可使用一次性恢复码。":"可选。启用后，登录时还需要验证器生成的六位动态码。"}</p>
      <form className="account-security-form" onSubmit={event=>{event.preventDefault();void run(async()=>{
        if(account.totp_enabled){
          await api.post("/auth/totp/disable",{current_password:securityPassword,totp:securityCode});
          update({...account,totp_enabled:false});setRecovery([]);notify("两步验证已关闭，其他登录会话和访问凭证已撤销");
        }else{setPending(await api.post("/auth/totp/start",{current_password:securityPassword}));setEnrollmentCode("");}
        setSecurityPassword("");setSecurityCode("");
      });}}>
        <label>验证当前密码<input required type="password" autoComplete="current-password" maxLength={1024} value={securityPassword} onChange={event=>setSecurityPassword(event.target.value)}/></label>
        {account.totp_enabled&&<label>当前动态码或恢复码<input required autoComplete="one-time-code" maxLength={128} value={securityCode} onChange={event=>setSecurityCode(event.target.value)}/></label>}
        <button disabled={busy}>{account.totp_enabled?"关闭两步验证":"设置两步验证"}</button>
      </form>
    </section>
    {pending&&<Modal title="启用两步验证" onClose={()=>{setPending(null);setEnrollmentCode("");}}>
      <p>在验证器中手动添加 R-Music，填写下面的密钥。设置须在 10 分钟内完成。</p>
      <code className="secret">{pending.secret}</code>
      <button onClick={()=>void copy(pending.uri,"已复制验证器链接")}>复制验证器链接</button>
      <form onSubmit={event=>{event.preventDefault();void run(async()=>{
        const result=await api.post<{recovery_codes:string[]}>("/auth/totp/confirm",{code:enrollmentCode});
        setPending(null);setEnrollmentCode("");setRecovery(result.recovery_codes);update({...account,totp_enabled:true});notify("两步验证已启用，其他设备需重新登录");
      });}}>
        <label>动态验证码<input required inputMode="numeric" autoComplete="one-time-code" pattern="[0-9]{6}" maxLength={6} value={enrollmentCode} onChange={event=>setEnrollmentCode(event.target.value)}/></label>
        <button className="primary" disabled={busy}>验证并启用</button>
      </form>
    </Modal>}
    {recovery.length>0&&<Modal title="保存恢复码" onClose={()=>setRecovery([])}>
      <p>恢复码只显示一次，每个可使用一次。请保存在安全的位置。</p>
      <div className="recovery-codes">{recovery.map(value=><code key={value}>{value}</code>)}</div>
      <button onClick={()=>void copy(recovery.join("\n"),"恢复码已复制")}>复制恢复码</button>
      <button className="primary" onClick={()=>setRecovery([])}>已保存恢复码</button>
    </Modal>}
  </>;
}
