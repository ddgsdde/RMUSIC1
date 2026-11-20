import {presentationVisible} from "./presentationVisibility";
import {useEffect,useRef,useState} from "react";
import {Check,Layers3,Plus,RefreshCw,Speaker,Unplug} from "lucide-react";
import {ApiError,type Endpoint,type RMusicClient,type Zone} from "@r-music/sdk";
import {Modal} from "./components";
import {outputLabel,zoneDisplayName} from "./outputLabels";
import {syncEligible,type DeviceGroup} from "./deviceGroupModel";
import {clientId} from "./clientId";
type Draft={create:boolean;request_id:string;name:string;zone_ids:string[];source_zone_id:string};
export function DeviceGroups({api,zones,endpoints,selected,supported,onSelect,refresh,onError}:{api:RMusicClient;zones:Zone[];endpoints:Endpoint[];selected:string;supported:boolean;onSelect:(id:string)=>void;refresh:()=>Promise<unknown>;onError:(error:unknown)=>void}){
 const[editingGroup,setEditingGroup]=useState<DeviceGroup>();
 const[groups,setGroups]=useState<DeviceGroup[]>([]),[editing,setEditing]=useState(false),[dissolve,setDissolve]=useState<DeviceGroup>(),[name,setName]=useState("全屋"),[rooms,setRooms]=useState<string[]>([]),[source,setSource]=useState(""),[busy,setBusy]=useState(false),[notice,setNotice]=useState("");
 const pending=useRef<Draft|null>(null),current=useRef(api),request=useRef<AbortController|null>(null);current.current=api;
 const zoneById=new Map(zones.map(zone=>[zone.id,zone]));
 const endpointById=new Map(endpoints.map(endpoint=>[endpoint.id,endpoint]));
 const heldZones=new Set(groups.flatMap(group=>group.zone_ids));
 const heldOutputs=new Set(endpoints.filter(endpoint=>endpoint.kind==="group").flatMap(endpoint=>endpoint.group?.endpoint_ids||[]));
 for(const id of heldZones){const zone=zoneById.get(id);if(zone)heldOutputs.add(zone.endpoint_id);}
 const available=zones.filter(zone=>!heldOutputs.has(zone.endpoint_id)&&syncEligible(endpointById.get(zone.endpoint_id)));
 const availableCount=new Set(available.map(zone=>zone.endpoint_id)).size;
 const editingIds=new Set(editingGroup?.zone_ids);
 const editingOutputs=new Set(editingGroup?.zone_ids.flatMap(id=>{const zone=zoneById.get(id);return zone?[zone.endpoint_id]:[];}));
 const choices=zones.filter(zone=>editingIds.has(zone.id)||
  (!heldOutputs.has(zone.endpoint_id)||editingOutputs.has(zone.endpoint_id))&&syncEligible(endpointById.get(zone.endpoint_id)));
 const pickedOutputs=new Set(rooms.map(id=>zoneById.get(id)?.endpoint_id));
 const choiceCounts=new Map<string,number>();
 for(const zone of choices)choiceCounts.set(zone.endpoint_id,(choiceCounts.get(zone.endpoint_id)||0)+1);
 const allSelectedOnline=rooms.every(id=>syncEligible(endpointById.get(zoneById.get(id)?.endpoint_id||"")));
 async function complete(group:DeviceGroup,select:boolean){
  pending.current=null;
  setEditing(false);
  setGroups(current=>[...current.filter(item=>item.id!==group.id),group]);
  setNotice(group.error||(select?"已组合为一个播放设备。":"已保存组合成员。"));
  await refresh().catch(onError);
  if(select&&current.current===api&&!group.error)onSelect(group.zone_id);
 }
 async function load(){
  if(!supported||request.current)return;
  const value=await api.request<{items:DeviceGroup[]}>("/device-groups");
  if(current.current!==api||request.current)return;
  setGroups(value.items);
  const draft=pending.current;
  const known=draft?.create&&value.items.find(group=>group.id===draft.request_id);
  if(known&&known.phase!=="joining")await complete(known,true);
 }
 useEffect(()=>{setGroups([]);setEditing(false);setEditingGroup(undefined);setNotice("");pending.current=null;void load().catch(onError);const changed=()=>{if(presentationVisible())void load().catch(()=>{});};window.addEventListener("rmusic-device-groups-changed",changed);const timer=setInterval(changed,60000);return()=>{request.current?.abort();clearInterval(timer);window.removeEventListener("rmusic-device-groups-changed",changed);};},[api,supported]);
 function open(group?:DeviceGroup){
  const initial=available.some(zone=>zone.id===selected)?selected:"";
  setEditingGroup(group);setRooms(group?[...group.zone_ids]:initial?[initial]:[]);
  setSource(group?.source_zone_id||initial);setName(group?.name||"全屋");setNotice("");pending.current=null;setEditing(true);
 }
 function closeEditor(){if(!busy){pending.current=null;setEditing(false);setNotice("");}}
 function choose(id:string){
  const next=rooms.includes(id)?rooms.filter(value=>value!==id):[...rooms,id];
  setRooms(next);if(!next.includes(source))setSource(next[0]||"");
 }
 async function create(){if(busy)return;setBusy(true);setNotice("");const controller=new AbortController();request.current=controller;
  const draft=pending.current||{create:!editingGroup,request_id:editingGroup?.id||clientId(),name:name.trim(),zone_ids:[...rooms].sort(),source_zone_id:source};pending.current=draft;
  try{const group=await api.request<DeviceGroup>(editingGroup?`/device-groups/${editingGroup.id}`:"/device-groups",{method:editingGroup?"PUT":"POST",body:JSON.stringify(editingGroup?{name:draft.name,zone_ids:draft.zone_ids,expected_revision:editingGroup.revision}:{request_id:draft.request_id,name:draft.name,zone_ids:draft.zone_ids,source_zone_id:draft.source_zone_id}),signal:controller.signal});if(current.current!==api||controller.signal.aborted)return;await complete(group,!editingGroup);}
  catch(error){if(!controller.signal.aborted&&current.current===api){if(error instanceof ApiError&&error.status>=400&&error.status<500)pending.current=null;setNotice(error instanceof Error?error.message:"暂时无法确认组合结果，请刷新或重试原请求。");onError(error);}}
  finally{if(current.current===api){setBusy(false);if(request.current===controller)request.current=null;}}
 }
 async function remove(){if(!dissolve||busy)return;setBusy(true);const value=dissolve;try{await api.delete(`/device-groups/${encodeURIComponent(value.id)}`);if(current.current!==api)return;setDissolve(undefined);await load();await refresh();if(selected===value.zone_id)onSelect(value.source_zone_id);setNotice("组合已拆开，各设备原来的队列与播放顺序已恢复。");}catch(error){if(current.current===api)onError(error);}finally{if(current.current===api)setBusy(false);}}
 return <section className="device-groups-page">
  <header className="device-groups-heading"><div><h1>设备组合</h1><p>多个输出共用一条播放队列。</p></div><button className="primary" disabled={!supported||busy||availableCount<2} onClick={()=>open()}><Plus size={18}/>组合设备</button></header>
  {!supported&&<p role="status" className="device-group-notice">当前 Core 尚未提供设备组合，请更新 Core 后使用。</p>}
  {supported&&availableCount<2&&<p role="status" className="device-group-notice">当前有 {availableCount} 个可组合输出。请连接至少两个支持同步的局域网输出。</p>}
  {notice&&!editing&&<p role="status" className="device-group-notice">{notice}</p>}
  <div className="device-group-cards">{groups.map(group=>{const zone=zoneById.get(group.zone_id),online=endpointById.get(group.endpoint_id)?.online;return <article className="device-group-card" key={group.id}>
   <header><span className="device-group-symbol"><Layers3 size={25}/></span><div><h2>{group.name}</h2><p>{group.zone_ids.length} 个输出 · {online?zone?.status==="playing"?"同步播放中":"在线":"部分输出离线"}</p></div>{selected===group.zone_id&&<Check size={19}/>}</header>
   <div className="device-group-members">{group.zone_ids.map(id=>{const z=zoneById.get(id);return <span key={id}><Speaker size={14}/>{z?outputLabel(z,endpointById.get(z.endpoint_id),{}):id}</span>;})}</div>
   {zone?.track&&<p className="device-group-current">{zone.track.title}<small>{zone.queue.length} 首歌曲</small></p>}
   {group.error&&<p role="alert" className="form-error">{group.error}</p>}
   <footer><button className={selected===group.zone_id?"selected":"primary"} disabled={busy||!online} onClick={()=>onSelect(group.zone_id)}>{selected===group.zone_id?"当前播放设备":"在这里听"}</button><button disabled={busy} onClick={()=>open(group)}>调整成员</button><button disabled={busy} onClick={()=>setDissolve(group)}><Unplug size={15}/>拆开</button></footer>
  </article>;})}</div>
  {supported&&!groups.length&&availableCount>=2&&<p className="device-group-empty">还没有组合。可以选择两个到四个输出，共享播放队列。</p>}
  <p className="device-group-footnote">安卓和网页可以控制组合。同步播放需要支持同步的原生输出或 Linux 网桥。</p>
  {editing&&<Modal portal title={editingGroup?"调整组合成员":"组合为一个设备"} onClose={closeEditor}>
   <div className="device-group-editor"><div className="device-group-fields"><label>设备名称<input value={name} maxLength={80} disabled={busy||!!pending.current} placeholder="例如：客厅与书房" onChange={e=>setName(e.target.value)}/></label><p>{editingGroup?"调整后保留组合的队列与进度。移出的设备会恢复原队列。":"选择同一局域网中的二至四个输出。"}</p>
    <div className="device-group-choices">{choices.map(zone=>{
     const endpoint=endpointById.get(zone.endpoint_id),checked=rooms.includes(zone.id);
     const eligible=syncEligible(endpoint),duplicate=!checked&&pickedOutputs.has(zone.endpoint_id);
     const sharedOutput=(choiceCounts.get(zone.endpoint_id)||0)>1;
     const detail=!eligible?"离线或暂不可用 · 可移出":duplicate?"同一物理输出已选":endpoint?.capabilities.platform?String(endpoint.capabilities.platform):endpoint?.kind;
     return <label key={zone.id}><input type="checkbox" disabled={busy||!!pending.current||!checked&&(!eligible||duplicate||rooms.length>=4)} checked={checked} onChange={()=>choose(zone.id)}/><Speaker size={18}/><span><strong>{sharedOutput?zoneDisplayName(zone):outputLabel(zone,endpoint,{})}</strong><small>{detail}</small></span></label>;
    })}</div>
    {!allSelectedOnline&&<p className="muted">请移出离线成员，或等它们重新连接后保存。</p>}
    {rooms.length>0&&!editingGroup&&<label>使用哪台设备当前的队列<select value={source} disabled={busy||!!pending.current} onChange={e=>setSource(e.target.value)}>{rooms.map(id=><option key={id} value={id}>{zoneById.get(id)?.name}</option>)}</select><small>其他成员的队列会保留，拆开组合后恢复。</small></label>}
    {notice&&<p role="alert" className="form-error">{notice}</p>}
    </div>
    <div className="form-actions">{pending.current?.create&&<button disabled={busy} onClick={()=>void load().catch(onError)}><RefreshCw size={16}/>查询结果</button>}<button className="primary" disabled={busy||!pending.current&&(!name.trim()||rooms.length<2||pickedOutputs.size!==rooms.length||!allSelectedOnline||!editingGroup&&!source)} onClick={()=>void create()}>{busy?"正在保存…":pending.current?"重试本次操作":editingGroup?"保存成员":"组合为一个设备"}</button></div>
   </div>
  </Modal>}
  {dissolve&&<Modal portal title={`拆开 ${dissolve.name}`} onClose={()=>!busy&&setDissolve(undefined)}><p>组合播放将停止。各设备会恢复组合前的队列、顺序和位置；原来在播放的设备会保持暂停。</p><div className="form-actions"><button disabled={busy} onClick={()=>setDissolve(undefined)}>保留组合</button><button className="primary" disabled={busy} onClick={()=>void remove()}>{busy?"正在拆开…":"拆开并恢复原队列"}</button></div></Modal>}
 </section>;
}
