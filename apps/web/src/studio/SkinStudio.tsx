import {androidInterfaces,mobileInterfaces,mobileUI,switchMobileUI} from "../mobileAppearance";
import {useState,type CSSProperties} from "react";
import {Check,Download,Palette,SlidersHorizontal,Trash2,Upload} from "lucide-react";
import type {Preferences} from "../App";
import {Modal} from "../components";
import {isAndroidClient,invokeNative} from "../native";
import {activeSkin,androidBuiltins,androidDefaultTabs,builtins,colors,defaultTabs,tabNames,type Tab,importedColorSkins,importSkin,storedSkins,validateSkin,type Skin} from "./skins";

const names=["背景","面板","次级面板","正文","辅助文字","分隔线","强调色","强调色文字"];
const uid=()=>"custom-"+Array.from(crypto.getRandomValues(new Uint8Array(8)),b=>b.toString(16).padStart(2,"0")).join("");
function Sample({skin,mode}:{skin:Skin;mode:"dark"|"light"}){
 const c=skin[mode];
 return <div className="skin-sample" data-layout={skin.layout} aria-hidden="true" style={{"--sample-gap":skin.spacing==="dense"?"5px":"10px","--sample-ink":c.text,"--sample-accent":c.accent,"--sample-edge":c.border,background:c.bg,color:c.text,borderRadius:skin.radius,fontFamily:skin.font==="serif"?"Georgia,serif":skin.font==="mono"?"monospace":"sans-serif",fontSize:17*skin.scale} as CSSProperties}><div><i/><i/><i/></div><section><b>夜航<br/>手记</b><em/></section><footer><i/><i/><i/></footer></div>
}
export function SkinStudio({preferences,save,notify}:{preferences:Preferences;save:(v:Partial<Preferences>)=>Promise<void>;notify:(s:string)=>void}){
 const android=isAndroidClient,interfaces=android?androidInterfaces:mobileInterfaces,presets=android?androidBuiltins:builtins;
 const active=activeSkin(preferences,android),custom=storedSkins(preferences),[draft,setDraft]=useState<Skin|null>(null),[error,setError]=useState("");
 const [mode,setMode]=useState<"dark"|"light">(document.documentElement.dataset.mode==="light"?"light":"dark");
 const [deleteId,setDeleteId]=useState("");
 const gallery=[...presets,...custom,...importedColorSkins(preferences)];
 const choose=async(id:string)=>{const skin=gallery.find(s=>s.id===id);await save({studio_skin:id,...(skin?.mobile_ui?{mobile_ui:skin.mobile_ui}:{})});};
 const keep=async(value:Skin)=>{const skin=validateSkin(value);const next=custom.filter(s=>s.id!==skin.id);if(next.length>=20)throw Error("最多保存 20 套自定义皮肤");await save({studio_skin:skin.id,...(skin.mobile_ui?{mobile_ui:skin.mobile_ui}:{}),studio_skins:[...next,skin]});setDraft(null);notify("已保存并应用皮肤");};
 const edit=()=>{setError("");setDraft({...active,...(custom.some(s=>s.id===active.id)?{}:{id:uid(),name:active.name+" · 自选"}),mobile_ui:mobileUI(preferences.mobile_ui,android)});};
 const exportFile=async()=>{const content=JSON.stringify({...active,mobile_ui:mobileUI(preferences.mobile_ui,android)},null,2);if(isAndroidClient){await invokeNative("native_export_skin",{content,filename:active.id+".rmusic-skin.json"});notify("已导出到下载目录的 R-Music 文件夹");return;}const href=URL.createObjectURL(new Blob([content],{type:"application/json"}));const a=document.createElement("a");a.href=href;a.download=active.id+".rmusic-skin.json";a.click();setTimeout(()=>URL.revokeObjectURL(href),1000);};
 const draftTabs=draft?.tabs??(android?androidDefaultTabs:defaultTabs);
 const patch=<K extends keyof Skin>(key:K,value:Skin[K])=>setDraft(old=>old?{...old,[key]:value}:null);
 return <section className="settings-section studio-skins"><div className="section-heading"><h2><Palette size={20}/>皮肤工作室</h2></div>
 <fieldset className="mobile-interface-settings"><legend>手机界面布局</legend><div className="mobile-interface-grid">{interfaces.map(item=><button key={item.id} className="mobile-interface-option" aria-pressed={mobileUI(preferences.mobile_ui,android)===item.id} onClick={()=>void save(switchMobileUI(preferences,item.id,android))}><span className={`interface-preview interface-${item.id}`} aria-hidden="true"><i/><span><b/><b/></span><em/><small><b/><b/><b/><b/></small></span><strong>{item.name}</strong><small>{item.description}</small></button>)}</div></fieldset>
 <h3>配色与细节</h3><div className="skin-gallery">{[...gallery,...(!gallery.some(s=>s.id===active.id)?[active]:[])].map(s=><button key={s.id} className="skin-choice" aria-pressed={s.id===active.id} onClick={()=>{if(s.id!==active.id)void choose(s.id)}}><Sample skin={s} mode={mode}/><strong>{s.name}{s.id===active.id&&<Check size={15}/>}</strong></button>)}</div>
 <div className="skin-toolbar"><button onClick={edit}><SlidersHorizontal/>编辑皮肤</button><label className="skin-file"><Upload/>导入<input type="file" aria-label="导入皮肤 JSON" accept=".json,application/json" onChange={async e=>{const file=e.target.files?.[0];e.target.value="";if(!file)return;try{if(file.size>32768)throw Error("皮肤文件最多 32 KiB");const incoming=importSkin(await file.text(),active);setError("");setDraft({...incoming,id:uid()});}catch(err){notify(String(err));}}}/></label><button onClick={()=>void exportFile().catch(e=>notify(String(e)))}><Download/>导出</button>{custom.some(s=>s.id===active.id)&&<button aria-label="删除当前自定义皮肤" onClick={()=>setDeleteId(active.id)}><Trash2/></button>}</div>
 {draft&&<Modal portal title="编辑皮肤" onClose={()=>setDraft(null)} wide><div className="skin-editor"><div className="skin-proof"><Sample skin={draft} mode={mode}/><div className="studio-segments">{(["light","dark"] as const).map(v=><button key={v} aria-pressed={mode===v} onClick={()=>setMode(v)}>{v==="light"?"浅色":"深色"}</button>)}</div></div>
 <div className="skin-fields"><label>皮肤名称<input value={draft.name} maxLength={40} onChange={e=>patch("name",e.target.value)}/></label>
 <label>手机界面<select value={mobileUI(draft.mobile_ui||preferences.mobile_ui,android)} onChange={e=>patch("mobile_ui",mobileUI(e.target.value,android))}>{interfaces.map(item=><option key={item.id} value={item.id}>{item.name}</option>)}</select></label>
 {([{key:"layout",label:"首页构图",items:[["sleeve","唱片封套"],["gallery","封面画廊"],["type","文字海报"]]},{key:"font",label:"标题字体",items:[["serif","衬线"],["sans","无衬线"],["mono","等宽"]]},{key:"spacing",label:"信息密度",items:[["open","宽松"],["dense","紧凑"]]},{key:"nav",label:"手机导航",items:[["labels","图标与文字"],["icons","仅图标"]]},{key:"cover",label:"播放器封面",items:[["square","方形"],["soft","圆角"],["disc","唱片"]]},{key:"motion",label:"动效",items:[["system","跟随系统"],["quiet","减少动效"]]}] as const).map(row=><label key={row.key}>{row.label}<select value={draft[row.key]??"square"} onChange={e=>patch(row.key,e.target.value as never)}>{row.items.map(([v,n])=><option key={v} value={v}>{n}</option>)}</select></label>)}
 <label>圆角 · {draft.radius} px<input type="range" min="0" max="32" value={draft.radius} onChange={e=>patch("radius",Number(e.target.value))}/></label>
 <label>字号 · {Math.round(draft.scale*100)}%<input type="range" min="85" max="125" step="5" value={Math.round(draft.scale*100)} onChange={e=>patch("scale",Number(e.target.value)/100)}/></label>
 <fieldset className="skin-tab-order"><legend>手机底栏顺序</legend>{draftTabs.map((tab,i)=><label key={i}>第 {i+1} 项<select value={tab} onChange={e=>{const next=[...draftTabs],v=e.target.value as Tab,old=next.indexOf(v);if(old>=0)next[old]=next[i];next[i]=v;patch("tabs",next);}}>{Object.entries(tabNames).map(([k,v])=><option key={k} value={k}>{v}</option>)}</select></label>)}
 <button type="button" onClick={()=>patch("tabs",draftTabs.length===4?draftTabs.slice(0,3):[...draftTabs,(Object.keys(tabNames) as Tab[]).find(tab=>!draftTabs.includes(tab)&&tab!=="settings")!])}>{draftTabs.length===4?"移除第四项":"添加第四项"}</button>
 </fieldset>
 <div className="skin-colors">{colors.map((k,i)=><label key={k}><input type="color" aria-label={names[i]} value={draft[mode][k]} onChange={e=>patch(mode,{...draft[mode],[k]:e.target.value})}/>{names[i]}</label>)}</div></div></div>
 {error&&<p role="alert">{error}</p>}<div className="skin-toolbar skin-editor-actions"><button onClick={()=>setDraft(null)}>取消</button><button className="primary" onClick={()=>void keep(draft).catch(e=>setError(String(e)))}><Check/>保存并应用</button></div></Modal>}
 {deleteId&&<Modal portal title="删除这套皮肤？" onClose={()=>setDeleteId("")}><p>{active.name}</p><button onClick={()=>{void save({studio_skin:presets[0].id,studio_skins:custom.filter(s=>s.id!==deleteId)});setDeleteId("");}}>删除并使用{presets[0].name}</button></Modal>}
 </section>
}

