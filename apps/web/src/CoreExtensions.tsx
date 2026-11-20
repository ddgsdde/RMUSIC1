import {errorMessage} from "./errorRecovery";
import {useEffect,useRef,useState} from "react";
import type {ExtensionMethod,RMusicClient} from "@r-music/sdk";
import {Modal} from "./components";

import {type Schema,type Values,object,protectedValue,mergeValues,initial,validate,configurationPatch} from "./pluginSchema";
function ArrayField({schema,value,change,label}:{schema:Schema;value:unknown;change:(value:unknown)=>void;label:string}){
 const items=Array.isArray(value)?value:[],[page,setPage]=useState(0);const offset=Math.min(page,Math.max(0,Math.ceil(items.length/20)-1))*20;
 return <fieldset><legend>{label} · {items.length} 项</legend>{items.slice(offset,offset+20).map((item,local)=>{const index=offset+local;return <div className="extension-array-item" key={index}><Fields schema={{type:'object',properties:{value:schema.items!},required:['value']}} values={{value:item}} prefix={`${label}.${index}.`} change={(_,next)=>change(items.map((v,i)=>i===index?next:v))}/><button type="button" onClick={()=>change(items.filter((_,i)=>i!==index))}>移除第 {index+1} 项</button></div>;})}<div className="group-actions"><button type="button" disabled={items.length>=(schema.maxItems??1000)} onClick={()=>{change([...items,initial(schema.items!)]);setPage(Math.floor(items.length/20));}}>添加一项</button><button type="button" disabled={offset===0} onClick={()=>setPage(offset/20-1)}>上一页</button><span>{offset+1}–{Math.min(items.length,offset+20)} / {items.length}</span><button type="button" disabled={offset+20>=items.length} onClick={()=>setPage(offset/20+1)}>下一页</button></div></fieldset>;
}
function SecretField({schema,value,change,label,id}:{schema:Schema;value:unknown;change:(value:unknown)=>void;label:string;id:string}){
 const [editing,setEditing]=useState(false),[advanced,setAdvanced]=useState(false);
 if(!editing)return <div className="setting-row"><span>{label} · {protectedValue(value)?'已保护':'尚未设置'}</span><button type="button" onClick={()=>setEditing(true)}>更改{label}</button></div>;
 return <fieldset><legend>{label}</legend><Fields schema={{type:'object',properties:{value:{...schema,secret:false}},required:['value']}} prefix={`${id}.`} values={{value:protectedValue(value)?initial(schema):value}} change={(_,next)=>change(next)}/><details onToggle={e=>setAdvanced(e.currentTarget.open)}><summary>高级输入</summary>{advanced&&<label>结构化值<textarea aria-label={id} placeholder="输入 JSON 值" maxLength={65536} onChange={event=>{try{const parsed=JSON.parse(event.target.value);event.target.setCustomValidity('');change(parsed);}catch{event.target.setCustomValidity('请输入有效 JSON 值');}}}/></label>}</details></fieldset>;
}
function Fields({schema,values,change,prefix=""}:{schema:Schema;values:Values;change:(name:string,value:unknown)=>void;prefix?:string}) {
  return <>{Object.entries(schema.properties||{}).map(([name,field])=>{
    const value=values[name],label=field.description||name.replaceAll('_',' '),id=prefix+name,required=schema.required?.includes(name);
    if(field.secret&&field.type!=="string")return <SecretField key={name} schema={field} value={value} change={v=>change(name,v)} label={label} id={id}/>;
    if(field.type==="object")return <fieldset key={name}><legend>{label}</legend><Fields schema={field} prefix={id+"."} values={object(value)?value:{}} change={(key,v)=>change(name,mergeValues(object(value)?value:{},{[key]:v},field))}/></fieldset>;
    if(field.type==="array")return <ArrayField key={name} schema={field} value={value} label={label} change={v=>change(name,v)}/>;
    if(field.enum)return <label key={name}>{label}<select required={required} aria-label={id} value={JSON.stringify(value)??""} onChange={event=>change(name,JSON.parse(event.target.value))}><option value="" disabled>请选择</option>{field.enum.map(item=><option key={JSON.stringify(item)} value={JSON.stringify(item)}>{String(item)}</option>)}</select></label>;
    if(field.type==="boolean")return <label key={name} className="setting-row"><span>{label}</span><input className="toggle" aria-label={id} type="checkbox" checked={value===true} onChange={event=>change(name,event.target.checked)}/></label>;
    const configured=field.secret&&protectedValue(value)===true;
    return <label key={name}>{label}{required?' *':''}<input required={required&&!configured} aria-label={id} autoComplete="off" type={field.secret?"password":field.type==="string"?"text":"number"} min={field.minimum} max={field.maximum} step={field.type==="integer"?1:"any"} maxLength={field.maxLength} value={configured?"":typeof value==="string"||typeof value==="number"?value:""} placeholder={configured?"已设置 · 留空保留":""} onChange={event=>{if(configured&&event.target.value==='')return;change(name,field.type==="string"?event.target.value:event.target.value===""?null:Number(event.target.value));}}/></label>;
  })}</>;
}
export function ResultView({value,schema}:{value:unknown;schema?:Schema}){
 const [shown,setShown]=useState(20);
 if(Array.isArray(value))return <div className="extension-output-list">{value.slice(0,shown).map((item,i)=><article key={i}><ResultView value={item} schema={schema?.items}/></article>)}{!value.length&&<p className="muted">没有结果</p>}{value.length>shown&&<button type="button" onClick={()=>setShown(v=>v+20)}>显示更多（还剩 {value.length-shown} 项）</button>}</div>;
 if(object(value))return <dl className="detail-facts extension-output">{Object.entries(value).map(([key,v])=><div key={key}><dt>{schema?.properties?.[key]?.description||key.replaceAll('_',' ')}</dt><dd><ResultView value={v} schema={schema?.properties?.[key]}/></dd></div>)}</dl>;
 return <span>{value==null?'—':typeof value==='boolean'?value?'是':'否':typeof value==='number'?value.toLocaleString():String(value)}</span>;
}
export function PluginConfiguration({api,id,close}:{api:RMusicClient;id:string;close:()=>void}) {
  const [config,setConfig]=useState<{schema:Schema;values:Values;revision:string}|null>(null),[patch,setPatch]=useState<Values>({});
  const [busy,setBusy]=useState(false),[error,setError]=useState(""),[version,setVersion]=useState(0),[conflict,setConflict]=useState(false);
  const pending=useRef<AbortController|null>(null);useEffect(()=>()=>pending.current?.abort(),[api,id]);
  useEffect(()=>{const controller=new AbortController();setError('');api.pluginConfiguration(id,controller.signal).then(value=>{if(!controller.signal.aborted){setConfig(value as typeof config);setConflict(false);}}).catch(cause=>{if(!controller.signal.aborted)setError(errorMessage(cause));});return()=>controller.abort();},[api,id,version]);
  return <Modal title="插件配置" onClose={close}><form onSubmit={async event=>{event.preventDefault();if(!config||busy)return;const controller=new AbortController();pending.current=controller;setBusy(true);setError("");try{await api.configurePlugin(id,{expected_revision:config.revision,values:configurationPatch(config.values,mergeValues(config.values,patch,config.schema),config.schema)},controller.signal);if(!controller.signal.aborted)close();}catch(cause){if(!controller.signal.aborted){setError(errorMessage(cause));setConflict((cause as {status?:number}).status===409);}}finally{if(!controller.signal.aborted)setBusy(false);}}} className="extension-form">
    {error&&<div role="alert"><p>{error}</p><button type="button" disabled={busy} onClick={()=>setVersion(v=>v+1)}>{conflict?'读取最新配置（保留填写内容）':'重新读取'}</button></div>}{config?<><Fields schema={config.schema} values={mergeValues(config.values,patch,config.schema)} change={(name,value)=>setPatch(old=>mergeValues(old,{[name]:value},config.schema))}/><div className="form-actions"><button type="button" onClick={close}>取消</button><button className="primary" disabled={busy||conflict}>{busy?'保存中…':'保存配置'}</button></div></>:!error&&<p>正在读取配置…</p>}
  </form></Modal>;
}
const category={metadata:'元数据',statistics:'统计',automation:'自动化'};
const scope={read:'读取',library:'修改曲库',control:'播放控制',admin:'管理员'};
export function CoreExtensions({api}:{api:RMusicClient}) {
  const [items,setItems]=useState<{id:string;name:string;enabled:boolean;extensions:ExtensionMethod[]}[]>([]),[error,setError]=useState(""),[version,setVersion]=useState(0),[loading,setLoading]=useState(true);
  const [selected,setSelected]=useState<{id:string;name:string;method:ExtensionMethod}|null>(null),[input,setInput]=useState<Values>({});
  const [result,setResult]=useState<unknown>(),[busy,setBusy]=useState(false),[runError,setRunError]=useState('');
  const pending=useRef<AbortController|null>(null);useEffect(()=>()=>pending.current?.abort(),[api]);
  useEffect(()=>{const controller=new AbortController();setLoading(true);setError('');api.extensions(controller.signal).then(value=>{if(!controller.signal.aborted)setItems(value.items);}).catch(cause=>{if(!controller.signal.aborted)setError(errorMessage(cause));}).finally(()=>{if(!controller.signal.aborted)setLoading(false);});return()=>controller.abort();},[api,version]);
  const title=(method:ExtensionMethod)=>method.title||`${method.method.replaceAll('_',' ')} · ${category[method.kind]}`;
  return <section className="settings-section"><div className="section-heading"><h2>插件扩展</h2><button disabled={loading} onClick={()=>setVersion(v=>v+1)}>刷新插件</button></div>{error&&<p role="alert">{error}<button onClick={()=>setVersion(v=>v+1)}>重试</button></p>}{loading&&<p role="status">读取插件…</p>}{!loading&&!items.length&&!error&&<p className="muted">尚未安装扩展插件。在音乐平台与插件中安装后，会在这里显示可用功能。</p>}
    <div className="extension-cards">{items.map(item=><article className="extension-card" key={item.id}><header><h3>{item.name}</h3><span>{item.enabled?'已启用':'已停用'}</span></header>{item.extensions.map(method=><div className="extension-action" key={method.method}><div><strong>{title(method)}</strong>{method.description&&<p>{method.description}</p>}<small>{scope[method.scope]} · {method.effect==='proposal'?'生成建议，需另行确认':'仅返回结果'}</small></div><button disabled={!item.enabled} onClick={()=>{setSelected({id:item.id,name:item.name,method});setInput(initial(method.input as Schema) as Values);setResult(undefined);setRunError('');}}>打开{method.title||''}</button></div>)}</article>)}</div>
    {selected&&<Modal title={title(selected.method)} onClose={()=>{pending.current?.abort();setBusy(false);setSelected(null);}}><form className="extension-form" onSubmit={async event=>{event.preventDefault();if(busy)return;const controller=new AbortController();pending.current=controller;setBusy(true);setRunError('');try{validate(input,selected.method.input as Schema);if(new TextEncoder().encode(JSON.stringify(input)).length>65536)throw new Error("输入超过 64 KiB，尚未发送");const value=await api.invokeExtension(selected.id,selected.method.method,input,controller.signal);if(!controller.signal.aborted)setResult(value.output);}catch(cause){if(!controller.signal.aborted)setRunError(errorMessage(cause));}finally{if(!controller.signal.aborted)setBusy(false);}}}>
      <p className="muted">{selected.name} · {scope[selected.method.scope]}</p><Fields key={selected.id+selected.method.method} schema={selected.method.input as Schema} values={input} change={(name,value)=>setInput(old=>mergeValues(old,{[name]:value}))}/><button disabled={busy} className="primary">{busy?'正在运行…':runError?'重试扩展':'运行扩展'}</button>
      {runError&&<p role="alert">{runError} · 检查插件状态或输入后重试，填写内容已保留。</p>}{result!==undefined&&<section aria-label="扩展结果"><h3>运行结果</h3><ResultView value={result} schema={selected.method.output as Schema}/><details><summary>原始数据</summary><pre className="extension-result">{JSON.stringify(result,null,2)}</pre></details></section>}{selected.method.effect==="proposal"&&<p className="fine-print">结果为建议，尚未应用到 Core。</p>}
    </form></Modal>}
  </section>;
}
