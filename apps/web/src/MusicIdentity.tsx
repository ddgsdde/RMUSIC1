import {CatalogLink} from "./CatalogLinks";
import {MusicVersions} from "./music/Versions";
import {errorMessage} from "./errorRecovery";
import {useEffect, useRef, useState} from "react";
import type {FingerprintEvidence, MusicIdentity as Identity, MusicIdentityAssertion, RMusicClient, Track, MusicIdentityPreview} from "@r-music/sdk";

import {IdentityEditor,IdentityDiff,manualAssertions} from "./IdentityEditor";

const kinds = {work: "作品", recording: "录音", release: "发行", artist: "参与者"};
const roles: Record<string, string> = {work:"作品",recording:"录音",release:"发行",performer:"演唱／演奏",album_artist:"专辑艺术家",composer:"作曲",conductor:"指挥",lyricist:"作词",producer:"制作",engineer:"录音工程"};
const namespaces:Record<string,string>={"rmusic-manual":"本地身份",rmusic:"曲库身份",local:"本地文件",musicbrainz:"MusicBrainz"};
const bases: Record<string, string> = {manual:"人工确认",source_identity:"来源身份",provider_id:"平台 ID",unresolved_credit:"文件署名",file_tags:"文件标签"};
export function MusicIdentityPanel({api,track,play,playExact,enqueue}:{api:RMusicClient;track:Track;play:(track:Track)=>Promise<void>;playExact?:(track:Track)=>Promise<void>;enqueue:(track:Track)=>Promise<void>}) {
  const [identity,setIdentity] = useState<Identity|null>(null);
  const [fingerprint,setFingerprint] = useState<FingerprintEvidence|null>(null);
  const [error,setError] = useState("");
  const [notice,setNotice] = useState("");
  const [busy,setBusy] = useState(false);
  const [version,setVersion] = useState(0);
  const [editing,setEditing] = useState(false);
  const [draft,setDraft] = useState<MusicIdentityAssertion[]>([]);
  const [preview,setPreview]=useState<{data:MusicIdentityPreview;draft:string}|null>(null);
  const [restore,setRestore]=useState<{revision:number;draft:string}|null>(null);
  const [history,setHistory] = useState<{items:{revision:number;created_at:number;before:Identity;after:Identity}[];total:number;offset:number}|null>(null);
  const [related,setRelated] = useState<{title:string;items:Track[];offset:number;total:number;id:string}|null>(null);
  const draftStamp=JSON.stringify([identity?.revision,draft]);
  const lifetime = useRef(new AbortController());
  useEffect(()=>{const controller=new AbortController();lifetime.current=controller;setIdentity(null);setFingerprint(null);setPreview(null);setRestore(null);setEditing(false);setDraft([]);setHistory(null);setRelated(null);setNotice("");setBusy(false);return()=>controller.abort();},[api,track.id]);
  useEffect(()=>{
    const controller=new AbortController();setError("");
    Promise.all([api.trackIdentity(track.id,controller.signal),api.fingerprint(track.id,controller.signal)])
      .then(([value,evidence])=>{if(!controller.signal.aborted){setIdentity(value);setFingerprint(evidence);}})
      .catch(cause=>{if(!controller.signal.aborted)setError(errorMessage(cause));});
    return()=>controller.abort();
  },[api,track.id,version]);
  async function run(work:()=>Promise<void>) {
    const controller=lifetime.current;setError("");setBusy(true);
    try {await work();}catch(cause){if(!controller.signal.aborted)setError(errorMessage(cause));}
    finally {if(!controller.signal.aborted)setBusy(false);}
  }
  async function browse(id:string,offset=0) {
    const value=await api.musicEntity(id,offset,20,lifetime.current.signal);
    if(!lifetime.current.signal.aborted)setRelated({id,title:value.entity.title,items:value.items,total:value.total,offset});
  }
  return <section className="identity-panel" aria-label="音乐身份与指纹">
    <MusicVersions api={api} track={track} play={play} playExact={playExact} enqueue={enqueue}/>
    <div className="group-actions"><button disabled={busy} onClick={()=>setVersion(v=>v+1)}>刷新身份与指纹</button><button disabled={!identity||busy} onClick={()=>{setDraft(identity!.entities.filter(e=>e.basis==="manual").map(({kind,namespace,external_id,title,role,position})=>({kind,namespace,external_id,title,role,position})));setEditing(true);}}>编辑确认关系</button><button disabled={busy} onClick={()=>void run(async()=>{const value=await api.identityHistory(track.id,0,20,lifetime.current.signal);if(!lifetime.current.signal.aborted)setHistory({...value,offset:0});})}>修订记录</button></div>
    {error&&<div role="alert"><p>{error}</p><button disabled={busy} onClick={()=>setVersion(v=>v+1)}>重新读取（保留草稿）</button></div>}{notice&&<p role="status">{notice}</p>}
    {!identity&&!error&&<p className="muted">正在读取音乐身份…</p>}
    {identity&&<dl className="detail-facts">{identity.entities.map((entity,i)=><div key={entity.id+entity.basis+entity.role+i}><dt>{roles[entity.role]||kinds[entity.kind]}</dt><dd><button className="text-button" disabled={busy} onClick={()=>void run(()=>browse(entity.id))}>{entity.title}</button><small>{bases[entity.basis]||entity.basis} · {namespaces[entity.namespace]||entity.namespace}</small>{entity.basis!=='manual'&&<button className="text-button identity-confirm" disabled={busy||(editing?draft:manualAssertions(identity)).length>=32} onClick={()=>{const current=editing?draft:manualAssertions(identity);if(!current.some(v=>v.kind===entity.kind&&v.namespace===entity.namespace&&v.external_id===entity.external_id&&v.role===entity.role)){const {kind,namespace,external_id,title,role,position}=entity;setDraft([...current,{kind,namespace,external_id,title,role,position}]);}setEditing(true);}}>确认此关系</button>}</dd></div>)}</dl>}
    {editing&&identity&&<IdentityEditor api={api} track={track} draft={draft} change={setDraft} busy={busy} cancel={()=>setEditing(false)} preview={()=>void run(async()=>{const stamp=draftStamp;const data=await api.previewTrackIdentity(track.id,{expected_revision:identity.revision,assertions:draft},lifetime.current.signal);if(!lifetime.current.signal.aborted)setPreview({data,draft:stamp});})} canSave={preview?.draft===draftStamp} save={()=>void run(async()=>{const value=restore?.draft===draftStamp?await api.restoreTrackIdentity(track.id,identity.revision,restore.revision,lifetime.current.signal):await api.editTrackIdentity(track.id,{expected_revision:identity.revision,assertions:draft},lifetime.current.signal);if(!lifetime.current.signal.aborted){setIdentity(value);setEditing(false);setHistory(null);setNotice('人工关系已保存');}})}/>}
    {editing&&preview?.draft===draftStamp&&<section className="identity-preview" aria-label="身份变更预览"><h3>变更预览 · 可能关联 {preview.data.affected_candidates} 条记录</h3><IdentityDiff before={preview.data.before} after={preview.data.after}/>{Object.entries(preview.data.resolution).map(([kind,state])=>state.conflict?<p role="alert" key={kind}>同类身份存在冲突，保存后不会自动合并。</p>:state.selected?<p key={kind}>{roles[kind]} · {state.selected.title} · {state.selected.basis==='manual'?'人工确认':state.selected.namespace}</p>:null)}<p className="fine-print">歌单、音频文件和播放ID保持；恢复操作将新增修订。</p></section>}
    {history&&<section aria-label="身份修订记录"><h3>修订记录 · {history.total} 次</h3>{history.items.map(row=><details key={row.revision}><summary>修订 {row.revision} · {new Date(row.created_at*1000).toLocaleString()}</summary><IdentityDiff before={row.before} after={row.after}/><button disabled={busy} onClick={()=>{const draft=manualAssertions(row.after);setDraft(draft);setRestore({revision:row.revision,draft:JSON.stringify([identity?.revision,draft])});setPreview(null);setEditing(true);setNotice(`已载入修订 ${row.revision}，预览后确认恢复为新修订。`);}}>预览恢复此修订</button></details>)}{!history.total&&<p className="muted">暂无人工修订。</p>}<div className="pagination-controls"><button disabled={busy||!history.offset} onClick={()=>void run(async()=>{const offset=Math.max(0,history.offset-20);const value=await api.identityHistory(track.id,offset,20,lifetime.current.signal);if(!lifetime.current.signal.aborted)setHistory({...value,offset});})}>上一页</button><span>第 {Math.floor(history.offset/20)+1} 页</span><button disabled={busy||history.offset+20>=history.total} onClick={()=>void run(async()=>{const offset=history.offset+20;const value=await api.identityHistory(track.id,offset,20,lifetime.current.signal);if(!lifetime.current.signal.aborted)setHistory({...value,offset});})}>下一页</button><button onClick={()=>setHistory(null)}>收起记录</button></div></section>}
    {related&&<section><h3>{related.title} · {related.total} 首</h3>{related.items.map(item=><div className="identity-match" key={item.id}><span>{item.title}<small><CatalogLink track={item}/> · <CatalogLink track={item} album/></small></span><button disabled={busy} onClick={()=>void run(()=>play(item))}>播放</button><button disabled={busy} onClick={()=>void run(()=>enqueue(item))}>加入队列</button></div>)}<div className="group-actions"><button disabled={busy||related.offset===0} onClick={()=>void run(()=>browse(related.id,related.offset-20))}>上一页</button><button disabled={busy||related.offset+20>=related.total} onClick={()=>void run(()=>browse(related.id,related.offset+20))}>下一页</button><button onClick={()=>setRelated(null)}>收起</button></div></section>}
    <details><summary>曲库身份维护</summary><button disabled={busy} onClick={()=>void run(async()=>{await api.indexMusicIdentity(lifetime.current.signal);if(!lifetime.current.signal.aborted)setNotice('全曲库身份索引已排队，可在设置的后台任务中查看、暂停或取消。');})}>更新全曲库身份索引</button></details>
    <section><h3>音频指纹</h3><p className="fine-print">仅分析本地或已完整缓存的音频。结果为同录音候选，不代表同一母带或发行，不自动合并。</p>
      <div className="group-actions"><button disabled={busy} onClick={()=>void run(async()=>{await api.analyzeFingerprint(track.id);if(!lifetime.current.signal.aborted)setNotice("指纹分析已加入后台任务；完成后刷新查看。");})}>分析指纹</button><span className="muted">{fingerprint?.available?`已分析 ${fingerprint.sampled_seconds} 秒`:fingerprint?.reason}</span></div>
      {fingerprint?.items?.map(item=><div className="identity-match" key={item.track.id}><span>{item.track.title}<small>{item.track.artist} · 相似度 {(item.similarity*100).toFixed(1)}%</small></span><button disabled={busy} onClick={()=>void run(()=>play(item.track))}>播放</button></div>)}
      {fingerprint?.available&&!fingerprint.items?.length&&<p className="muted">本页未找到符合版本与指纹条件的候选。</p>}
      {fingerprint?.next_offset!=null&&<button disabled={busy} onClick={()=>void run(async()=>{const value=await api.fingerprint(track.id,lifetime.current.signal,fingerprint.next_offset!);if(!lifetime.current.signal.aborted)setFingerprint(value);})}>查看下一批候选</button>}
    </section>
  </section>;
}
