import {CatalogLink} from "../CatalogLinks";
import {useEffect,useRef,useState} from "react";
import type {Playlist,ProviderManifest,RMusicClient,SmartPlaylistQuery,SmartPlaylistPreview as Preview} from "@r-music/sdk";
import {Modal} from "../components";
import {errorMessage} from "../errorRecovery";

const presets:Record<string,SmartPlaylistQuery>={custom:{},unheard:{max_plays:0,sort:"recent_added",max_results:100},rediscover:{not_played_days:30,min_plays:1,sort:"least_played",max_results:100},hires:{min_sample_rate:88200,min_bit_depth:24,max_results:100},favorites:{favorites_only:true,sort:"recent",max_results:100}};
const titles={custom:"自定义规则",unheard:"还没认真听过",rediscover:"久违的音乐",hires:"高解析曲库",favorites:"我的收藏"};
export function SmartPlaylistEditor({api,playlist,providers,close,saved,initialRule,initialName}:{api:RMusicClient;playlist?:Playlist;providers:ProviderManifest[];close:()=>void;saved:()=>void;initialRule?:SmartPlaylistQuery;initialName?:string}) {
  const [name,setName]=useState(playlist?.name||initialName||""),[rule,setRule]=useState<SmartPlaylistQuery>(playlist?.smart_query||initialRule||{});
  const [preview,setPreview]=useState<Preview|null>(null),[error,setError]=useState(""),[busy,setBusy]=useState(false);
  const pending=useRef<AbortController|null>(null);
  useEffect(()=>()=>pending.current?.abort(),[api]);
  function change<K extends keyof SmartPlaylistQuery>(field:K,value:SmartPlaylistQuery[K]){setRule(old=>{const next={...old};if(value===undefined||value==="")delete next[field];else next[field]=value;return next;});setPreview(null);}
  async function inspect(offset=0) {
    pending.current?.abort();const c=new AbortController();pending.current=c;setBusy(true);setError("");
    try{const result=await api.request<Preview>("/smart-playlists/preview",{method:"POST",signal:c.signal,body:JSON.stringify({rule,offset})});if(!c.signal.aborted)setPreview(result);}
    catch(e){if(!c.signal.aborted)setError(errorMessage(e));}finally{if(!c.signal.aborted)setBusy(false);}
  }
  const numbers:[keyof SmartPlaylistQuery,string,number][]=[["min_plays","最少聆听次数",1000000],["max_plays","最多聆听次数",1000000],["min_seconds","最短时长（秒）",86400],["max_seconds","最长时长（秒）",86400],["not_played_days","至少多少天没听",36500],["min_sample_rate","最低采样率（Hz）",768000],["min_bit_depth","最低位深",64]];
  return <Modal title={playlist?"编辑智能歌单":"创建智能歌单"} onClose={close} wide>
    <form className="smart-rule-editor extension-form" onSubmit={async e=>{e.preventDefault();const c=new AbortController();pending.current?.abort();pending.current=c;setBusy(true);setError("");try{await api.request(playlist?`/playlists/${encodeURIComponent(playlist.id)}`:"/playlists",{method:playlist?"PUT":"POST",signal:c.signal,body:JSON.stringify({name,smart_query:rule})});if(!c.signal.aborted)saved();}catch(e){if(!c.signal.aborted)setError(errorMessage(e));}finally{if(!c.signal.aborted)setBusy(false);}}}>
      <fieldset className="workflow-fields" disabled={busy}><label>名称<input required maxLength={100} value={name} onChange={e=>setName(e.target.value)}/></label>
      <label>从模板开始<select defaultValue="custom" onChange={e=>{setRule({...presets[e.target.value]});setPreview(null);}}>{Object.entries(titles).map(([id,title])=><option key={id} value={id}>{title}</option>)}</select></label>
      <div className="identity-editor-grid">
        <label>搜索词<input value={rule.q||""} maxLength={500} onChange={e=>change("q",e.target.value)}/></label>
        <label>音源<select value={rule.source||""} onChange={e=>change("source",e.target.value)}><option value="">所有可用音源</option><option value="local">本地</option>{providers.filter(p=>p.kind!=="extension").map(p=><option key={p.id} value={p.id}>{p.name}</option>)}</select></label>
        <label>文件类型<select value={rule.storage||""} onChange={e=>change("storage",e.target.value)}><option value="">全部</option><option value="local">本地原文件</option><option value="cache">完整缓存</option></select></label>
        <label>音频格式<input placeholder="例如 flac" maxLength={30} value={rule.format||""} onChange={e=>change("format",e.target.value)}/></label>
        <label>排序<select value={rule.sort||"title"} onChange={e=>change("sort",e.target.value as SmartPlaylistQuery["sort"])}>{Object.entries({title:"歌曲名称",recent:"最近聆听",least_played:"聆听次数少到多",most_played:"聆听次数多到少",recent_added:"最近加入曲库"}).map(([id,title])=><option key={id} value={id}>{title}</option>)}</select></label>
        <label>最多选取<input type="number" min={1} max={10000} value={rule.max_results??10000} onChange={e=>change("max_results",Number(e.target.value))}/></label>
      </div>
      <details className="smart-advanced"><summary>聆听与音质条件{numbers.some(([field])=>rule[field]!==undefined)?` · 已设置 ${numbers.filter(([field])=>rule[field]!==undefined).length} 项`:""}</summary><div className="identity-editor-grid">{numbers.map(([field,title,max])=><label key={field}>{title}<input type="number" min={0} max={max} value={typeof rule[field]==="number"?rule[field]:""} onChange={e=>change(field,e.target.value===""?undefined:Number(e.target.value))}/></label>)}</div></details>
      <label className="check-line"><input type="checkbox" className="toggle" checked={rule.favorites_only===true} onChange={e=>change("favorites_only",e.target.checked||undefined)}/>仅我的收藏</label>
      <label className="check-line"><input type="checkbox" className="toggle" checked={rule.exclude_saved===true} onChange={e=>change("exclude_saved",e.target.checked||undefined)}/>排除已加入我的手动歌单的歌曲</label>
      {(rule.album_id||rule.artist_id)&&<p className="fine-print">保留原有专辑／艺术家身份限制。<button type="button" onClick={()=>{change("album_id",undefined);change("artist_id",undefined);}}>清除这两个限制</button></p>}
      <p className="fine-print">播放时按最新曲库重新计算，不会下载歌曲。聆听次数使用共享 Core 的已确认统计，收藏与歌单条件仅属于当前账号。</p>
      {error&&<p role="alert">{error} · 规则已保留。</p>}
      <div className="form-actions"><button type="button" disabled={busy} onClick={()=>void inspect()}>预览匹配</button><button className="primary" disabled={busy||!name.trim()}>保存智能歌单</button></div>
      {preview&&<section aria-label="智能歌单预览"><strong>{preview.total} 首{preview.truncated?" · 已达到选取上限":""}</strong>{preview.items.map(t=><div className="version-row" key={t.id}><span>{t.title}<small><CatalogLink track={t}/> · <CatalogLink track={t} album/></small></span><small>{t.source}</small></div>)}{!preview.total&&<p>暂无匹配，调整条件后可重新预览。</p>}<div className="pagination-controls"><button type="button" disabled={busy||!preview.offset} onClick={()=>void inspect(Math.max(0,preview.offset-30))}>上一页</button><button type="button" disabled={busy||preview.offset+30>=preview.total} onClick={()=>void inspect(preview.offset+30)}>下一页</button></div></section>}
      </fieldset>
    </form>
  </Modal>;
}
