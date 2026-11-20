import {presentedZones} from "./deviceGroupModel";
import {useState} from "react";
import type {Endpoint,RMusicClient,Zone} from "@r-music/sdk";
import {ArrowRightLeft} from "lucide-react";
import {Modal} from "./components";
export function PlaybackTransfer({api,zones,endpoints,selected,onTransferred}:{api:RMusicClient;zones:Zone[];endpoints:Endpoint[];selected:string;onTransferred:(id:string)=>void}){
 const[open,setOpen]=useState(false),[source,setSource]=useState(selected),[target,setTarget]=useState(""),[busy,setBusy]=useState(false),[error,setError]=useState("");
 const transferable=presentedZones(zones,endpoints).filter(z=>endpoints.find(e=>e.id===z.endpoint_id)?.capabilities.native_client_v1!==true);
 const origin=transferable.find(z=>z.id===source),destination=transferable.find(z=>z.id===target);
 const online=(zone:Zone)=>endpoints.some(e=>e.id===zone.endpoint_id&&e.online);
 return <><button onClick={()=>{setSource(selected);setTarget("");setError("");setOpen(true);}}><ArrowRightLeft size={17}/>播放流转</button>{open&&<Modal title="播放流转" onClose={()=>{if(!busy)setOpen(false);}}>
  <p className="muted">将完整队列、顺序、循环／随机设置和当前进度交给另一设备。目标准备好后原设备停止；保留目标自己的音量与输出设置。</p>
  <label>从哪个设备<select aria-label="流转来源设备" value={source} disabled={busy} onChange={e=>{setSource(e.target.value);setTarget("");}}><option value="">选择来源</option>{transferable.filter(z=>z.queue.length>0).map(z=><option key={z.id} value={z.id}>{endpoints.find(e=>e.id===z.endpoint_id)?.name||z.name} · {z.track?.title||"已保存的队列"}</option>)}</select></label>
  <label>接着在哪个设备听<select aria-label="流转目标设备" value={target} disabled={busy} onChange={e=>setTarget(e.target.value)}><option value="">选择目标</option>{transferable.filter(z=>z.endpoint_id!==origin?.endpoint_id).map(z=><option key={z.id} value={z.id} disabled={!online(z)}>{endpoints.find(e=>e.id===z.endpoint_id)?.name||z.name}{online(z)?"":" · 离线"}</option>)}</select></label>
  {origin&&<p>{origin.queue.length} 首 · {origin.status==="playing"?"继续播放":origin.status==="paused"?"保持暂停":"保持停止"}</p>}
  {destination&&destination.queue.length>0&&<p className="muted">将替换目标设备现有的 {destination.queue.length} 首队列。</p>}
  {error&&<p role="alert">{error}</p>}
  <button className="primary" disabled={busy||!origin||!destination||!online(destination)} onClick={async()=>{if(!origin||!destination)return;setBusy(true);setError("");try{const value=await api.post<{target:Zone}>(`/zones/${encodeURIComponent(origin.id)}/transfer`,{target_zone_id:destination.id,expected_playback_id:origin.playback_id??null});onTransferred(value.target.id);setOpen(false);}catch(e){setError(e instanceof Error?e.message:"流转失败，原设备保留");}finally{setBusy(false);}}}>{busy?"正在准备目标设备…":"流转到此设备"}</button>
 </Modal>}</>;
}
