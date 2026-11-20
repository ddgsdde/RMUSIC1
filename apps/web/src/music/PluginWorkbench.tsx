import {useEffect,useRef,useState} from "react";
import type {ProviderManifest,RMusicClient,PluginDevelopmentCatalog as Tools} from "@r-music/sdk";
import {errorMessage} from "../errorRecovery";
import {saveApiFile} from "../fileDownload";
export function PluginWorkbench({api,plugins}:{api:RMusicClient;plugins:ProviderManifest[]}) {
  const [tools,setTools]=useState<Tools|null>(null),[kind,setKind]=useState("extension"),[id,setId]=useState("my-plugin");
  const [target,setTarget]=useState(""),[error,setError]=useState(""),[notice,setNotice]=useState(""),[busy,setBusy]=useState(false);
  const pending=useRef<AbortController|null>(null);
  useEffect(()=>{const c=new AbortController();pending.current=c;api.request<Tools>("/plugins/development",{signal:c.signal}).then(v=>{if(!c.signal.aborted)setTools(v);}).catch(e=>{if(!c.signal.aborted)setError(errorMessage(e));});return()=>pending.current?.abort();},[api]);
  return <section className="settings-section plugin-workbench"><h2>插件开发工具</h2>
    {error&&<p role="alert">{error}</p>}{notice&&<p role="status">{notice}</p>}
    {tools&&<>
      <p className="fine-print">SDK {tools.sdk_version} · Core 插件 API {tools.api_version}。{tools.trust}</p>
      <div className="identity-editor-grid"><label>模板类型<select value={kind} onChange={e=>setKind(e.target.value)}><option value="extension">只读曲库扩展</option><option value="source">音源骨架</option></select></label><label>插件 ID<input maxLength={64} pattern="[a-z][a-z0-9_-]*" value={id} onChange={e=>setId(e.target.value)}/></label></div>
      <button disabled={busy||!/^[a-z][a-z0-9_-]{0,63}$/.test(id)} onClick={async()=>{setBusy(true);setError("");try{await saveApiFile(api,`/plugins/development/template?kind=${kind}&id=${encodeURIComponent(id)}`,`${id}.zip`,{preferRequestedFilename:true});setNotice("模板已下载，包含 Python SDK、清单和入口文件。");}catch(e){setError(errorMessage(e));}finally{setBusy(false);}}}>下载开发模板</button>
      <details><summary>创建、检查与打包命令</summary><div className="plugin-commands">{tools.commands.map(command=><pre key={command}><code>{command}</code></pre>)}</div><p className="fine-print">本地工具使用临时空数据目录，检查清单、RPC 一致性和健康状态。安装前仍由 Core 执行最终兼容性与权限合同检查。</p></details>
      <form className="llm-request" onSubmit={async e=>{e.preventDefault();const c=new AbortController();pending.current=c;setBusy(true);setError("");try{const result=await api.request<{passed:boolean;checks:string[]}>(`/plugins/${encodeURIComponent(target)}/contract-check`,{method:"POST",body:"{}",signal:c.signal});if(!c.signal.aborted)setNotice(result.passed?"检查通过：包清单、配置与扩展声明、RPC 清单一致性、健康状态。":"合同检查未通过");}catch(e){if(!c.signal.aborted)setError(errorMessage(e));}finally{if(!c.signal.aborted)setBusy(false);}}}>
        <label>已安装插件<select disabled={busy} required value={target} onChange={e=>setTarget(e.target.value)}><option value="">选择要检查的插件</option>{plugins.map(p=><option key={p.id} value={p.id}>{p.name} · {p.version}</option>)}</select></label><button disabled={busy||!target}>运行合同检查</button>
      </form>
    </>}
  </section>;
}
