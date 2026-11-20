import {useEffect,useRef,useState} from "react";
import type {RMusicClient,LlmConfiguration as Configuration} from "@r-music/sdk";
import {invalidateResources} from "../resourceCache";
import {errorMessage} from "../errorRecovery";

export function LlmSettings({api,admin}:{api:RMusicClient;admin:boolean}) {
  const [advanced,setAdvanced]=useState<boolean>();
  const [draft,setDraft]=useState<Configuration|null>(null),[key,setKey]=useState("");
  const [clearKey,setClearKey]=useState(false),[models,setModels]=useState<string[]>([]);
  const [error,setError]=useState(""),[notice,setNotice]=useState(""),[busy,setBusy]=useState(false);
  const pending=useRef<AbortController|null>(null);
  const begin=()=>{pending.current?.abort();const c=new AbortController();pending.current=c;setBusy(true);setError("");return c;};
  async function read() {
    const c=begin();try{const [value,algorithm]=await Promise.all([api.request<Configuration>("/llm/settings",{signal:c.signal}),api.request<{advanced:boolean}>("/recommendations/settings",{signal:c.signal})]);if(!c.signal.aborted){setAdvanced(algorithm.advanced);setDraft(value);setNotice("已读取最新配置；密钥输入已保留。");}}
    catch(e){if(!c.signal.aborted)setError(errorMessage(e));}finally{if(!c.signal.aborted)setBusy(false);}
  }
  useEffect(()=>{setKey("");setClearKey(false);setModels([]);setDraft(null);setAdvanced(undefined);if(admin)void read();return()=>pending.current?.abort();},[api,admin]);
  if(!admin)return <section className="settings-section"><p>LLM 服务由 Core 管理员配置。关闭时仍可使用本地与平台推荐。</p></section>;
  async function selectAlgorithm(next:boolean) {
    const c=begin();
    try {
      const value=await api.request<{advanced:boolean}>("/recommendations/settings",{method:"PUT",signal:c.signal,body:JSON.stringify({advanced:next})});
      if(!c.signal.aborted){setAdvanced(value.advanced);invalidateResources(api);setNotice("家庭推荐算法已保存。");}
    }catch(e){if(!c.signal.aborted)setError(errorMessage(e));}finally{if(!c.signal.aborted)setBusy(false);}
  }
  return <section className="settings-section llm-settings"><h2>推荐算法</h2>
    <p className="fine-print">统一用于家庭账号的发现推荐和电台。收藏、聆听记录和推荐结果仍按账号分开。</p>
    {advanced!==undefined&&<label>家庭推荐算法<select value={advanced?"advanced":"local"} disabled={busy} onChange={event=>void selectAlgorithm(event.target.value==="advanced")}><option value="local">本地算法</option><option value="advanced">进阶算法</option></select></label>}
    <p className="fine-print">搜歌时，进阶算法向模型发送参考歌曲的歌名、艺人和曲风。重排时会发送候选曲目编号、歌名、艺人、专辑和简短偏好。模型不可用时，Core 仍可从曲库和音源目录选歌。</p>
    {error&&<p role="alert">{error} · 草稿保留。<button disabled={busy} onClick={()=>void read()}>重新读取配置</button></p>}
    {notice&&<p role="status">{notice}</p>}
    {draft&&<form className="extension-form" onSubmit={async e=>{e.preventDefault();const c=begin();try{const value=await api.request<Configuration>("/llm/settings",{method:"PUT",signal:c.signal,body:JSON.stringify({expected_revision:draft.revision,enabled:draft.enabled,base_url:draft.base_url,model:draft.model,...(clearKey?{key:""}:key?{key}:{})})});if(!c.signal.aborted){setDraft(value);setKey("");setClearKey(false);setNotice("LLM 服务配置已保存。");}}catch(e){if(!c.signal.aborted)setError(errorMessage(e));}finally{if(!c.signal.aborted)setBusy(false);}}}>
      <fieldset className="workflow-fields" disabled={busy}><label className="check-line"><input type="checkbox" className="toggle" checked={draft.enabled} onChange={e=>setDraft({...draft,enabled:e.target.checked})}/>允许使用 LLM 推荐</label>
      <label>接口地址<input type="url" placeholder="https://example.com 或 https://example.com/v1" maxLength={2048} value={draft.base_url} onChange={e=>{setDraft({...draft,base_url:e.target.value});setModels([]);}}/></label>
      <label>服务密钥<input type="password" autoComplete="new-password" maxLength={8192} value={key} placeholder={draft.key_configured?"已配置，留空保留":"服务允许匿名访问时可留空"} onChange={e=>{setKey(e.target.value);setClearKey(false);}}/></label>
      {draft.key_configured&&<label className="check-line"><input type="checkbox" checked={clearKey} onChange={e=>setClearKey(e.target.checked)}/>清除已保存密钥</label>}
      <div className="group-actions"><button type="button" disabled={busy||!draft.base_url} onClick={async()=>{const c=begin();try{const result=await api.request<{items:string[]}>("/llm/models",{method:"POST",signal:c.signal,body:JSON.stringify({base_url:draft.base_url,...(clearKey?{key:""}:key?{key}:{})})});if(!c.signal.aborted){setModels(result.items);if(result.items.length===1)setDraft({...draft,model:result.items[0]});setNotice(`获取到 ${result.items.length} 个模型。选择后保存配置。`);}}catch(e){if(!c.signal.aborted)setError(errorMessage(e));}finally{if(!c.signal.aborted)setBusy(false);}}}>获取可用模型</button></div>
      <label>模型<input list="llm-models" maxLength={200} value={draft.model} onChange={e=>setDraft({...draft,model:e.target.value})}/><datalist id="llm-models">{models.map(id=><option key={id} value={id}/>)}</datalist></label>
      <div className="form-actions"><button type="button" disabled={busy} onClick={()=>void read()}>重新读取</button><button className="primary" disabled={busy}>{busy?"处理中…":"保存 LLM 配置"}</button></div>
      </fieldset>
    </form>}
  </section>;
}
