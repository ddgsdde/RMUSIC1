import {PlaybackPolicyEditor} from "./PlaybackPolicy";
import {VersionEvidence} from "./VersionEvidence";
import {CatalogLink} from "../CatalogLinks";
import {useEffect, useRef, useState} from "react";
import type {RMusicClient, Track,MusicEdition as Edition,MusicVersionsPage as Versions} from "@r-music/sdk";
import {errorMessage} from "../errorRecovery";
import {quality} from "../components";

const labels:Record<string,string> = {unspecified:"未标注", original:"原版", live:"现场", remaster:"重制母带", remix:"混音", cover:"翻唱", instrumental:"器乐／伴奏", radio_edit:"电台剪辑", other:"其他"};

export function MusicVersions({api,track,play,playExact,enqueue,initialOpen=false}:{api:RMusicClient;track:Track;play:(t:Track)=>Promise<void>;playExact?:(t:Track)=>Promise<void>;enqueue:(t:Track)=>Promise<void>;initialOpen?:boolean}) {
  const [open,setOpen]=useState(initialOpen), [offset,setOffset]=useState(0), [revision,refresh]=useState(0);
  const [playbackPreferred,setPlaybackPreferred]=useState<Track>();
  const [otherOffset,setOtherOffset]=useState(0),[query,setQuery]=useState(""),[input,setInput]=useState(""),[sort,setSort]=useState("title"),[available,setAvailable]=useState(false);
  const [result,setResult]=useState<Versions|null>(null), [draft,setDraft]=useState<Edition|null>(null);
  const [error,setError]=useState(""), [notice,setNotice]=useState(""), [busy,setBusy]=useState(false);
  const pending=useRef<AbortController|null>(null);
  useEffect(()=>{setResult(null);setDraft(null);setPlaybackPreferred(undefined);setOffset(0);setOtherOffset(0);setQuery("");setInput("");setError("");setNotice("");return()=>pending.current?.abort();},[api,track.id]);
  useEffect(()=>{
    if(!open)return;
    const controller=new AbortController();pending.current=controller;setBusy(true);
    api.request<Versions>(`/library/tracks/${encodeURIComponent(track.id)}/versions?${new URLSearchParams({offset:String(offset),other_offset:String(otherOffset),q:query,sort,...(available?{available:"true"}:{})})}`,{signal:controller.signal})
      .then(value=>{if(!controller.signal.aborted){setResult(value);setDraft(current=>current||value.edition);setError("");}})
      .catch(e=>{if(!controller.signal.aborted)setError(errorMessage(e));})
      .finally(()=>{if(!controller.signal.aborted)setBusy(false);});
    return()=>controller.abort();
  },[api,track.id,open,offset,otherOffset,query,sort,available,revision]);
  async function action(work:(signal:AbortSignal)=>Promise<unknown>) {
    pending.current?.abort();const controller=new AbortController();pending.current=controller;setBusy(true);setError("");
    try {await work(controller.signal);if(!controller.signal.aborted){setNotice("已保存");refresh(v=>v+1);}}
    catch(e){if(!controller.signal.aborted)setError(errorMessage(e));}
    finally{if(!controller.signal.aborted)setBusy(false);}
  }
  return <details className="music-versions" open={initialOpen?open:undefined} onToggle={e=>setOpen(e.currentTarget.open)}>
    <summary>音乐版本与录音</summary>
    {open&&<>
      {error&&<p role="alert">{error}<button onClick={()=>refresh(v=>v+1)}>重新读取</button></p>}
      {notice&&<p role="status">{notice}</p>}
      {!result&&!error&&<p>正在读取版本…</p>}
      {result&&<>
        <p className="fine-print">{result.identity_resolved?"以下文件具有相同且唯一的录音身份。版本标签不会改变身份，推荐偏好与播放策略分别设置。":"录音身份尚未唯一确认。请先编辑确认关系，同名歌曲不会自动合并。"}</p>
        {result.candidate_scan_limited&&<p className="fine-print">关联记录超过单次检查范围，本页仅显示已检查的版本。</p>}
        <form className="version-filters group-actions" onSubmit={e=>{e.preventDefault();setOffset(0);setOtherOffset(0);setQuery(input.trim());}}><label>筛选版本<input aria-label="筛选版本" value={input} maxLength={200} onChange={e=>setInput(e.target.value)} placeholder="歌曲、艺术家或专辑"/></label><button disabled={busy}>筛选</button><label>排序<select aria-label="版本排序" value={sort} onChange={e=>{setSort(e.target.value);setOffset(0);setOtherOffset(0);}}><option value="title">名称</option><option value="source">来源</option><option value="added">最近加入</option></select></label><label><input type="checkbox" checked={available} onChange={e=>{setAvailable(e.target.checked);setOffset(0);setOtherOffset(0);}}/>仅可用</label></form>
        {!result.items.length&&<p className="muted">没有符合筛选的版本</p>}
        <div className="version-list">{result.items.map(({track:item,edition,quality_evidence,identity})=><article className="version-row" key={item.id}>
          <div><strong>{edition.label||labels[edition.kind]} · {item.title}</strong><small><CatalogLink track={item}/> · <CatalogLink track={item} album/> · {item.source==="local"?"本地":item.source}</small><small>{quality(item)}{item.id===track.id?" · 当前歌曲":""}{!item.available?" · 不可用":""}</small><VersionEvidence evidence={quality_evidence} identity={identity}/></div>
          <div className="group-actions"><button disabled={busy||!item.available} onClick={()=>void (playExact||play)(item).catch(e=>setError(errorMessage(e)))}>播放此版本</button><button disabled={busy||!item.available} onClick={()=>void enqueue(item).catch(e=>setError(errorMessage(e)))}>入队</button>
            <button disabled={busy||!result.identity_resolved||!item.available} onClick={()=>setPlaybackPreferred(item)}>选为播放首选</button><button disabled={busy||!result.identity_resolved||!item.available} aria-pressed={result.preferred.track_id===item.id} onClick={()=>void action(signal=>api.request(`/library/tracks/${encodeURIComponent(track.id)}/versions`,{method:"PUT",signal,body:JSON.stringify({expected_revision:result.preferred.revision,track_id:result.preferred.track_id===item.id?null:item.id})}))}>{result.preferred.track_id===item.id?"取消推荐偏好":"设为推荐偏好"}</button></div>
        </article>)}</div>
        {result.total>20&&<div className="pagination-controls"><button disabled={busy||!offset} onClick={()=>setOffset(v=>Math.max(0,v-20))}>上一页</button><span>{offset+1}–{Math.min(offset+20,result.total)} / {result.total}</span><button disabled={busy||offset+20>=result.total} onClick={()=>setOffset(v=>v+20)}>下一页</button></div>}
        {(!!result.other_recordings.length||!!otherOffset)&&<details><summary>同一作品的其他录音{result.other_page?` · ${result.other_page.total}`:""}</summary><p className="fine-print">演唱、演奏或编配可能不同，保持独立录音。</p>{result.other_recordings.map(item=><div className="version-row" key={item.id}><span>{item.title}<small><CatalogLink track={item}/> · <CatalogLink track={item} album/></small></span><button disabled={!item.available} onClick={()=>void (playExact||play)(item).catch(e=>setError(errorMessage(e)))}>播放此录音</button></div>)}{result.other_page&&(otherOffset>0||result.other_page.total>20)&&<div className="pagination-controls"><button disabled={busy||!otherOffset} onClick={()=>setOtherOffset(v=>Math.max(0,v-20))}>上一页录音</button><span>{Math.min(otherOffset+1,result.other_page.total)}–{Math.min(otherOffset+20,result.other_page.total)} / {result.other_page.total}</span><button disabled={busy||result.other_page.next_offset==null} onClick={()=>setOtherOffset(result.other_page!.next_offset!)}>下一页录音</button></div>}</details>}
        <PlaybackPolicyEditor api={api} id={track.id} preferred={playbackPreferred}/>
        {draft&&<details><summary>编辑当前文件的版本资料</summary><form className="extension-form" onSubmit={e=>{e.preventDefault();void action(async signal=>{const next=await api.request<Edition>(`/library/tracks/${encodeURIComponent(track.id)}/edition`,{method:"PUT",signal,body:JSON.stringify({expected_revision:draft.revision,kind:draft.kind,label:draft.label,notes:draft.notes})});if(!signal.aborted)setDraft(next);});}}>
          <label>版本类型<select value={draft.kind} onChange={e=>setDraft({...draft,kind:e.target.value})}>{Object.entries(labels).map(([key,label])=><option key={key} value={key}>{label}</option>)}</select></label>
          <label>版本名称<input maxLength={200} value={draft.label} onChange={e=>setDraft({...draft,label:e.target.value})}/></label>
          <label>备注<textarea maxLength={2000} value={draft.notes} onChange={e=>setDraft({...draft,notes:e.target.value})}/></label>
          <div className="group-actions"><button type="button" disabled={busy} onClick={()=>setDraft(result.edition)}>载入最新资料</button><button disabled={busy}>保存版本资料</button></div>
        </form></details>}
      </>}
    </>}
  </details>;
}
