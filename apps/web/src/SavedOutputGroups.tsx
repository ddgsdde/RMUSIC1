import {useEffect,useRef,useState,type FormEvent} from "react";
import {ApiError,type OutputGroup,type RMusicClient,type Zone} from "@r-music/sdk";
import {Modal} from "./components";
import {clientId} from "./clientId";
type Draft={id:string;name:string;zone_ids:string[];revision:number};
export function SavedOutputGroups({api,zones,canChoose,onChoose}:{api:RMusicClient;zones:Zone[];canChoose:boolean;onChoose:(ids:string[])=>void}) {
 const [open,setOpen]=useState(false),[groups,setGroups]=useState<OutputGroup[]>([]),[draft,setDraft]=useState<Draft|null>(null),[busy,setBusy]=useState(false),[loading,setLoading]=useState(false),[notice,setNotice]=useState(""),[reload,setReload]=useState(0);
 const current=useRef(api),pending=useRef<AbortController|null>(null);current.current=api;
 useEffect(()=>{setBusy(false);setDraft(null);setGroups([]);return()=>{pending.current?.abort();pending.current=null;};},[api]);
 useEffect(()=>{
  if(!open)return;let live=true;const controller=new AbortController();const timeout=setTimeout(()=>controller.abort(),10000);setLoading(true);
  void api.listOutputGroups(controller.signal).then(value=>{if(live&&current.current===api){setGroups(value.items);setNotice("");}}).catch(()=>{if(live&&current.current===api)setNotice("无法读取常用组合，请检查连接或权限后刷新。");}).finally(()=>{clearTimeout(timeout);if(live)setLoading(false);});
  return()=>{live=false;controller.abort();clearTimeout(timeout);};
 },[api,open,reload]);
 async function mutate(kind:"save"|"delete",value:Draft) {
  if(pending.current||loading)return;const controller=new AbortController();pending.current=controller;setBusy(true);setNotice("");const timeout=setTimeout(()=>controller.abort(),10000);
  try {
   const saved=kind==="save"?await api.saveOutputGroup(value.id,{name:value.name,zone_ids:value.zone_ids,expected_revision:value.revision},controller.signal):await api.deleteOutputGroup(value.id,value.revision,controller.signal);
   if(current.current!==api||pending.current!==controller||controller.signal.aborted)return;
   if(saved){setDraft(saved);setGroups(old=>[...old.filter(item=>item.id!==saved.id),saved]);setNotice("组合已保存，尚未启动播放。");}
   else{setGroups(old=>old.filter(item=>item.id!==value.id));if(draft?.id===value.id)setDraft(null);setNotice("组合已删除，正在播放的分组不受影响。");}
  }catch(error){if(current.current===api&&pending.current===controller)setNotice(error instanceof ApiError?error.status===409?"组合已被其他控制端修改或删除。请刷新列表后重新编辑，当前输入已保留。":error.message:"尚未收到操作结果；可用相同内容重试，或刷新列表查询。");}
  finally{clearTimeout(timeout);if(pending.current===controller){pending.current=null;setBusy(false);}}
 }
 const valid=!!draft&&draft.name.trim().length>0&&draft.name.trim().length<=80&&draft.zone_ids.length>=2&&draft.zone_ids.length<=4&&draft.zone_ids.every(id=>zones.some(zone=>zone.id===id))&&new Set(draft.zone_ids.map(id=>zones.find(zone=>zone.id===id)?.endpoint_id)).size===draft.zone_ids.length;
 function submit(event:FormEvent){event.preventDefault();if(draft&&valid)void mutate("save",draft);}
 return <>
  <button type="button" onClick={()=>{setOpen(true);setNotice("");}}>常用音区组合</button>
  {open&&<Modal title="常用音区组合" onClose={()=>{if(!busy)setOpen(false);}}>
   <p className="muted">保存在当前 Core，控制账号之间共享。选择组合后仍需选歌并手动起播。</p>
   <div className="group-actions"><button type="button" disabled={busy||loading} onClick={()=>{setDraft({id:clientId(),name:"",zone_ids:[],revision:0});setNotice("");}}>新建组合</button><button type="button" disabled={busy||loading} onClick={()=>setReload(value=>value+1)}>刷新列表</button></div>
   {notice&&<p role="status">{notice}</p>}
   {loading&&<p className="muted">正在读取组合…</p>}
   {!loading&&!groups.length&&<p className="muted">还没有常用组合。</p>}
   <div className="saved-output-groups">{groups.map(group=><article className="saved-output-group" key={group.id}>
    <div><strong>{group.name}</strong><p className="muted">{group.zone_ids.map(id=>zones.find(zone=>zone.id===id)?.name||"已移除音区").join("、")}</p></div>
    <div className="group-actions"><button type="button" disabled={busy||loading||!canChoose||group.zone_ids.some(id=>!zones.some(zone=>zone.id===id))} onClick={()=>{onChoose([...group.zone_ids]);setOpen(false);}}>选择此组合</button><button type="button" disabled={busy||loading} aria-label={`编辑组合 ${group.name}`} onClick={()=>{setDraft({...group,zone_ids:[...group.zone_ids]});setNotice("");}}>编辑</button><button type="button" disabled={busy||loading} aria-label={`删除组合 ${group.name}`} onClick={()=>void mutate("delete",group)}>删除</button></div>
   </article>)}</div>
   {draft&&<form className="saved-output-editor" onSubmit={submit}>
    <h3>{draft.revision?"编辑组合":"新建组合"}</h3>
    <label>组合名称<input aria-label="组合名称" maxLength={80} value={draft.name} disabled={busy} onChange={event=>setDraft({...draft,name:event.target.value})}/></label>
    <p className="muted">选择二至四个使用不同输出的音区。可预先保存离线输出，播放前会重新检查可用状态。</p>
    {draft.zone_ids.filter(id=>!zones.some(zone=>zone.id===id)).map(id=><label className="group-room" key={id}><input type="checkbox" checked disabled={busy} onChange={()=>setDraft({...draft,zone_ids:draft.zone_ids.filter(value=>value!==id)})}/><span>已移除音区</span></label>)}
    {zones.map(zone=><label className="group-room" key={zone.id}><input type="checkbox" checked={draft.zone_ids.includes(zone.id)} disabled={busy||draft.zone_ids.length>=4&&!draft.zone_ids.includes(zone.id)} onChange={event=>setDraft({...draft,zone_ids:event.target.checked?[...draft.zone_ids,zone.id]:draft.zone_ids.filter(id=>id!==zone.id)})}/><span>{zone.name}</span></label>)}
    <p className="fine-print">保存只记录音区选择，不改变曲目、音量或正在播放的分组。</p>
    <div className="group-actions"><button className="primary" disabled={busy||loading||!valid}>保存组合</button><button type="button" disabled={busy} onClick={()=>setDraft(null)}>结束编辑</button></div>
   </form>}
  </Modal>}
 </>;
}
