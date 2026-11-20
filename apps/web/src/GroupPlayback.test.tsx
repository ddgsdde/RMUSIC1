// @vitest-environment jsdom
import {act} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,describe,expect,it,vi} from "vitest";
import {ApiError,RMusicClient,type Endpoint,type SyncStart,type Track,type Zone} from "@r-music/sdk";
import {GroupPlayback} from "./GroupPlayback";
let host:HTMLDivElement,root:Root;
const refresh=vi.fn(async()=>{});
const endpoints=["a","b"].map(id=>({id,name:id,kind:"alsa",online:true,verified:false,capabilities:{scheduled_start_v1:true,scheduled_queue_v1:true,continuous_pcm:true},clock_calibration:{version:1,core_clock_id:"core-clock",host_clock_id:"host-clock",host_minus_core_ns:0,network_rtt_ns:100,network_uncertainty_ns:50,observed_at_core_ns:1,valid_until_core_ns:1e12,sample_count:2,device_clock_mapped:false}})) as Endpoint[];
const zones=["a","b"].map(id=>({id,name:"房间 "+id,endpoint_id:id,status:"stopped",playback_id:null})) as Zone[];
const song={id:"pcm",title:"夜航",artist:"测试艺术家",format:"wav",available:true,sample_rate:768000,bit_depth:32} as Track;
const jobs=new Map<string,SyncStart>();
function job(id:string,phase:SyncStart["phase"]="preparing"):SyncStart{return {id,track_id:song.id,position:0,phase,error:null,statistics_scope:"group",strict_sync:false,playhead:{queue_index:0,position:0},listened_seconds:0,members:zones.map(z=>({zone_id:z.id,endpoint_id:z.endpoint_id,phase:"ready",frames:0,sample_rate:768000,position:0}))} as SyncStart;}
function client(base="http://qa") {
 const commands=new Map<string,string>();
 const api=new RMusicClient(base,"synthetic"),request=vi.spyOn(api,"request").mockImplementation(async(path,options)=>{
  if(path==="/endpoints")return {items:endpoints};
  if(path.startsWith("/sync/starts?"))return {items:[...jobs.values()].filter(j=>["preparing","committed","playing","pausing","paused","seeking"].includes(j.phase)),total:jobs.size};
  if(path.startsWith("/library/tracks"))return {items:[song,{...song,id:"low",title:"晨光",sample_rate:48000},{...song,id:"dsd",title:"DSD",format:"dsf",bit_depth:1}]};
  if(path==="/sync/starts"){const body=JSON.parse(options!.body as string),value=jobs.get(body.request_id)||job(body.request_id);jobs.set(value.id,value);return value;}
  const id=path.split("/")[3];if(jobs.has(id)){
   if(path.endsWith("/control")){const body=JSON.parse(options!.body as string),encoded=JSON.stringify(body);if(commands.has(body.request_id)){if(commands.get(body.request_id)!==encoded)throw new ApiError(409,"conflict","请求冲突");return jobs.get(id);}commands.set(body.request_id,encoded);
    const old=jobs.get(id)!;jobs.set(id,{...old,members:body.action==="members"?body.zone_ids.map((zone_id:string)=>old.members.find(m=>m.zone_id===zone_id)||{...job(id).members[0],zone_id,endpoint_id:zone_id}):old.members,phase:body.action==="pause"?"paused":body.action==="resume"?"playing":old.phase,playhead:["seek","jump"].includes(body.action)?{queue_index:body.queue_index,position:body.position}:old.playhead,transport:{request_id:body.request_id,action:body.action,revision:(old.transport?.revision||0)+1,state:"applied",error:null,incomplete_final_reports:false}});
   }
   if(path.endsWith("/cancel"))jobs.set(id,job(id,"cancelled"));return jobs.get(id);
  }
  throw new ApiError(404,"not_found","未找到请求");
 });return {api,request};
}
function render(api:RMusicClient,coreId="core",userId="user",supported=true,z=zones,listSupported=false,queueSupported=false,transportSupported=false,queueJumpSupported=false,membershipSupported=false,clockDiagnosticsSupported=false){return <GroupPlayback api={api} coreId={coreId} userId={userId} supported={supported} listSupported={listSupported} queueSupported={queueSupported} transportSupported={transportSupported} queueJumpSupported={queueJumpSupported} membershipSupported={membershipSupported} clockDiagnosticsSupported={clockDiagnosticsSupported} zones={z} endpoints={endpoints} refresh={refresh}/>;}
function selected(api:RMusicClient,value:SyncStart){jobs.set(value.id,value);sessionStorage.setItem("rmusic.group-start.v1:"+JSON.stringify(["core","user",api.baseUrl]),JSON.stringify({request:{request_id:value.id,track_id:value.track_id,zone_ids:zones.map(z=>z.id),position:0},title:song.title,external:true}));}
async function click(label:string){const b=[...host.querySelectorAll("button")].find(b=>b.textContent===label||b.getAttribute("aria-label")===label)!;expect(b).toBeDefined();await act(async()=>b.click());}
async function choose(){await click("选择音区和歌曲");await act(async()=>vi.advanceTimersByTimeAsync(300));const checks=[...host.querySelectorAll<HTMLInputElement>('.group-room input:not([name="group-queue-mode"])')];await act(async()=>{checks[0].click();checks[1].click();});await act(async()=>[...host.querySelectorAll<HTMLButtonElement>('.group-tracks button')].find(b=>b.textContent?.includes("夜航"))!.click());}
async function submit(){await act(async()=>host.querySelector("form")!.dispatchEvent(new Event("submit",{bubbles:true,cancelable:true})));}
beforeEach(()=>{vi.useFakeTimers();vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);sessionStorage.clear();jobs.clear();refresh.mockClear();host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.useRealTimers();vi.unstubAllGlobals();});
describe("分组播放",()=>{
 it("同步保护默认关闭，选择后提交容差且不隐式启用样本处理",async()=>{
  const {api,request}=client(),normal=request.getMockImplementation()!;
  const outputs=endpoints.map(endpoint=>({...endpoint,capabilities:{...endpoint.capabilities,device_clock_v1:true,programme_clock_v1:true}}));
  request.mockImplementation(async(path,options)=>path==="/endpoints"?{items:outputs}:normal(path,options));
  await act(async()=>root.render(<GroupPlayback api={api} coreId="core" userId="user" supported syncPolicySupported zones={zones} endpoints={outputs} refresh={refresh}/>));await choose();
  const toggle=[...host.querySelectorAll('label')].find(label=>label.textContent?.includes('超限停止整组'))!.querySelector<HTMLInputElement>('input')!;expect(toggle.checked).toBe(false);await act(async()=>toggle.click());await submit();
  const body=JSON.parse(request.mock.calls.find(([path])=>path==="/sync/starts")![1]!.body as string);expect(body.sync_policy).toEqual({max_known_error_us:1000,warmup_ms:3000});expect(body.pcm_rate_adjustment).toBeUndefined();expect(body.automatic_rate_adjustment).toBeUndefined();
 });

 it("实验自动校正默认关闭，显式选择保留完整起播和重试正文",async()=>{
  const {api,request}=client(),normal=request.getMockImplementation()!;let lost=true;
  const outputs=endpoints.map(endpoint=>({...endpoint,capabilities:{...endpoint.capabilities,group_output_v2:true,measured_listens_v2:true,pcm_rate_adjustment_v1:true,programme_clock_v2:true,group_rate_lease_v1:true}}));
  request.mockImplementation(async(path,options)=>{if(path==="/endpoints")return {items:outputs};if(path==="/sync/starts"&&lost){lost=false;throw new TypeError("lost auto response");}return normal(path,options);});
  const view=(enabled=true)=><GroupPlayback api={api} coreId="core" userId="user" supported queueSupported outputTimingSupported automaticRateSupported={enabled} zones={zones} endpoints={outputs} refresh={refresh}/>;
  await act(async()=>root.render(view()));await choose();const mode=host.querySelector<HTMLInputElement>('[name="group-automatic-rate"]')!;expect(mode.checked).toBe(false);expect(mode.disabled).toBe(false);await act(async()=>mode.click());expect(host.querySelector<HTMLInputElement>('[name="group-output-timing"]')!.checked).toBe(true);expect(host.textContent).toContain("不走原样路径");await submit();
  const posts=()=>request.mock.calls.filter(([path])=>path==="/sync/starts"),body=posts()[0][1]!.body;expect(JSON.parse(body as string)).toMatchObject({measured_version:2,queue:["pcm"],pcm_rate_adjustment:true,automatic_rate_adjustment:true});
  await act(async()=>root.unmount());root=createRoot(host);await act(async()=>root.render(view(false)));expect([...host.querySelectorAll<HTMLButtonElement>('button')].find(button=>button.textContent==="重试本次请求")!.disabled).toBe(true);expect(posts()).toHaveLength(1);
  await act(async()=>root.render(view()));await click("重试本次请求");expect(posts()).toHaveLength(2);expect(posts()[1][1]!.body).toBe(body);
 });
 it("实验自动校正能力缺失或变化时不提交，取消选择不携带处理标记",async()=>{
  const {api,request}=client(),normal=request.getMockImplementation()!;let missing=true;
  const outputs=()=>endpoints.map((endpoint,index)=>({...endpoint,capabilities:{...endpoint.capabilities,group_output_v2:true,measured_listens_v2:true,pcm_rate_adjustment_v1:true,programme_clock_v2:true,group_rate_lease_v1:!missing||index===0}}));
  request.mockImplementation(async(path,options)=>path==="/endpoints"?{items:outputs()}:normal(path,options));const view=(enabled=true)=><GroupPlayback api={api} coreId="core" userId="user" supported queueSupported outputTimingSupported automaticRateSupported={enabled} zones={zones} endpoints={outputs()} refresh={refresh}/>;
  await act(async()=>root.render(view(false)));await choose();const mode=host.querySelector<HTMLInputElement>('[name="group-automatic-rate"]')!;expect(mode.disabled).toBe(true);await act(async()=>root.render(view()));expect(mode.disabled).toBe(true);
  missing=false;await act(async()=>vi.advanceTimersByTimeAsync(2100));expect(mode.disabled).toBe(false);await act(async()=>mode.click());missing=true;await act(async()=>vi.advanceTimersByTimeAsync(2100));expect(mode.checked).toBe(true);expect(host.textContent).toContain("此输出暂不支持实验自动校正");expect(host.querySelector<HTMLButtonElement>('button.primary')!.disabled).toBe(true);await submit();expect(request.mock.calls.some(([path])=>path==="/sync/starts")).toBe(false);
  await act(async()=>mode.click());await submit();const body=JSON.parse(request.mock.calls.find(([path])=>path==="/sync/starts")![1]!.body as string);expect(body.measured_version).toBe(2);expect(body.pcm_rate_adjustment).toBeUndefined();expect(body.automatic_rate_adjustment).toBeUndefined();
 });
 it("实验自动校正显示处理与归零状态，改组拒绝不支持租约的新输出",async()=>{
  const {api,request}=client(),normal=request.getMockImplementation()!,id=crypto.randomUUID();
  request.mockImplementation(async(path,options)=>path==="/endpoints"?{items:endpoints.map((endpoint,index)=>({...endpoint,capabilities:{...endpoint.capabilities,group_output_v2:true,measured_listens_v2:true,pcm_rate_adjustment_v1:true,programme_clock_v2:true,group_rate_lease_v1:index===0}}))}:normal(path,options));
  selected(api,{...job(id,"playing"),measured_version:2,queue:{length:1,segment_start:0},members:[{...job(id).members[0],requested_rate_ppm:0}],rate_adjustment:{enabled:true,automatic:true,mode:"experimental_auto",last_request_id:null,reset_on_rearm:true,controller:{state:"held",reason:"等待有效反馈",lease_ms:1500,max_ppm:500,max_slew_ppm_per_second:20,driver_accuracy_verified:false}},clock_diagnostics:{version:1,state:"unavailable",evaluated_at_core_ns:0,valid_for_ms:0,reference_zone_id:null,members:[],total_uncertainty_ns:null,driver_accuracy_verified:false,strict_sync:false,correction:"experimental"}});
  await act(async()=>root.render(<GroupPlayback api={api} coreId="core" userId="user" supported transportSupported membershipSupported clockDiagnosticsSupported zones={zones} endpoints={endpoints} refresh={refresh}/>));expect(host.textContent).toContain("反馈不足，目标归零");expect(host.textContent).toContain("请求目标 0 ppm");expect(host.textContent).not.toContain("当前未启用补偿");expect(host.textContent).toContain("PCM 样本处理");
  await click("调整分组成员");const room=[...host.querySelectorAll('label')].find(label=>label.textContent?.includes("房间 b"))!;expect(room.querySelector<HTMLInputElement>('input')!.disabled).toBe(true);expect(room.textContent).toContain("此输出暂不支持实验自动校正");
 });
 it("手动恢复可从重开后的分组列表找到中断分组且不自动提交",async()=>{
  const {api,request}=client(),normal=request.getMockImplementation()!,id=crypto.randomUUID(),value:SyncStart={...job(id,"interrupted"),measured_version:2};jobs.set(id,value);
  request.mockImplementation(async(path,options)=>path.startsWith("/sync/starts?")&&path.includes("recoverable=true")?{items:[value],total:1}:path.endsWith("/recovery")?{available:true,checkpoint_id:"a".repeat(64),queue_index:0,position:0.2}:normal(path,options));
  await act(async()=>root.render(<GroupPlayback api={api} coreId="core" userId="user" supported listSupported recoverySupported zones={zones} endpoints={endpoints} refresh={refresh}/>));await click("查看其他分组");await act(async()=>host.querySelector<HTMLButtonElement>(".group-list button")!.click());
  expect(host.textContent).toContain("手动恢复整组");expect(host.textContent).toContain("0.200 秒继续");expect(request.mock.calls.some(([,o])=>o?.method==="POST")).toBe(false);
 });
 it("手动恢复等待最终帧，刷新只查询，旧核心或旧计时不显示入口",async()=>{
  const {api,request}=client(),normal=request.getMockImplementation()!,id=crypto.randomUUID();let point:any={available:false,reason:"仍在等待原输出的最终帧确认"};selected(api,{...job(id,"interrupted"),measured_version:2});
  request.mockImplementation(async(path,options)=>path.endsWith("/recovery")?point:normal(path,options));const view=(enabled=true)=><GroupPlayback api={api} coreId="core" userId="user" supported recoverySupported={enabled} zones={zones} endpoints={endpoints} refresh={refresh}/>;
  await act(async()=>root.render(view()));expect(host.textContent).toContain("等待原输出的最终帧");expect([...host.querySelectorAll<HTMLButtonElement>("button")].find(b=>b.textContent==="手动恢复整组")!.disabled).toBe(true);
  point={available:true,checkpoint_id:"a".repeat(64),queue_index:2,position:1.25};await act(async()=>vi.advanceTimersByTimeAsync(2100));expect(host.textContent).toContain("从第 3 首 · 1.250 秒继续");expect([...host.querySelectorAll<HTMLButtonElement>("button")].find(b=>b.textContent==="手动恢复整组")!.disabled).toBe(false);expect(request.mock.calls.some(([,o])=>o?.method==="POST")).toBe(false);
  await act(async()=>root.render(view(false)));expect(host.textContent).not.toContain("手动恢复整组");jobs.set(id,{...job(id,"interrupted"),measured_version:1});await act(async()=>root.unmount());root=createRoot(host);await act(async()=>root.render(view()));expect(host.textContent).not.toContain("手动恢复整组");
 });
 it("手动恢复丢失响应后重载不重提，重试保留原 UUID 和检查点并隔离账号",async()=>{
  const {api,request}=client(),normal=request.getMockImplementation()!,id=crypto.randomUUID();selected(api,{...job(id,"interrupted"),measured_version:2});let lost=true;
  request.mockImplementation(async(path,options)=>{if(path.endsWith("/recovery")){if(options?.method==="POST"){if(lost){lost=false;throw new TypeError("lost recovery response");}jobs.set(id,{...job(id,"preparing"),measured_version:2});return jobs.get(id);}return {available:true,checkpoint_id:"a".repeat(64),queue_index:0,position:0.2};}return normal(path,options);});
  const view=(user="user")=><GroupPlayback api={api} coreId="core" userId={user} supported recoverySupported zones={zones} endpoints={endpoints} refresh={refresh}/>;const posts=()=>request.mock.calls.filter(([path,o])=>path.endsWith("/recovery")&&o?.method==="POST");
  await act(async()=>root.render(view()));await click("手动恢复整组");expect(posts()).toHaveLength(1);const body=posts()[0][1]!.body;expect(JSON.parse(body as string)).toMatchObject({checkpoint_id:"a".repeat(64)});
  await act(async()=>root.render(view("other-user")));expect(host.textContent).not.toContain("重试本次恢复");await act(async()=>root.unmount());root=createRoot(host);await act(async()=>root.render(view()));expect(posts()).toHaveLength(1);expect(host.textContent).toContain("尚未收到恢复结果；刷新只查询状态");await click("重试本次恢复");expect(posts()).toHaveLength(2);expect(posts()[1][1]!.body).toBe(body);expect(host.textContent).toContain("正在准备各音区");
 });
 it("恢复检查点拒绝后只在放弃并重新点击时创建新请求",async()=>{
  const {api,request}=client(),normal=request.getMockImplementation()!,id=crypto.randomUUID();selected(api,{...job(id,"interrupted"),measured_version:2});let rejected=false;
  request.mockImplementation(async(path,options)=>{if(path.endsWith("/recovery")){if(options?.method==="POST"){if(!rejected){rejected=true;throw new ApiError(409,"group_recovery_unavailable","检查点已变化，请重新查询");}jobs.set(id,{...job(id,"preparing"),measured_version:2});return jobs.get(id);}return {available:true,checkpoint_id:(rejected?"b":"a").repeat(64),queue_index:0,position:0.2};}return normal(path,options);});const posts=()=>request.mock.calls.filter(([path,o])=>path.endsWith("/recovery")&&o?.method==="POST");
  await act(async()=>root.render(<GroupPlayback api={api} coreId="core" userId="user" supported recoverySupported zones={zones} endpoints={endpoints} refresh={refresh}/>));await click("手动恢复整组");await act(async()=>vi.advanceTimersByTimeAsync(2100));expect(posts()).toHaveLength(1);expect([...host.querySelectorAll<HTMLButtonElement>("button")].find(b=>b.textContent==="重试本次恢复")!.disabled).toBe(true);await click("放弃被拒绝的请求");expect(posts()).toHaveLength(1);await click("手动恢复整组");expect(posts()).toHaveLength(2);const first=JSON.parse(posts()[0][1]!.body as string),second=JSON.parse(posts()[1][1]!.body as string);expect(second.request_id).not.toBe(first.request_id);expect(second.checkpoint_id).toBe("b".repeat(64));
 });
 it("新版计时默认关闭，显式单曲请求与丢失响应重试保留版本且重载只查询",async()=>{
  const {api,request}=client(),normal=request.getMockImplementation()!;let lost=true;
  request.mockImplementation(async(path,options)=>{if(path==="/endpoints")return {items:endpoints.map(e=>({...e,capabilities:{...e.capabilities,group_output_v2:true,measured_listens_v2:true}}))};if(path==="/sync/starts"&&lost){lost=false;throw new TypeError("lost v2 response");}return normal(path,options);});
  const view=(enabled=true)=> <GroupPlayback api={api} coreId="core" userId="user" supported queueSupported outputTimingSupported={enabled} zones={zones} endpoints={endpoints} refresh={refresh}/>;
  await act(async()=>root.render(view()));await choose();const mode=host.querySelector<HTMLInputElement>('[name="group-output-timing"]')!;expect(mode).not.toBeNull();expect(mode.checked).toBe(false);await act(async()=>mode.click());await submit();
  const posts=()=>request.mock.calls.filter(([p])=>p==="/sync/starts"),body=posts()[0][1]!.body;expect(JSON.parse(body as string)).toMatchObject({measured_version:2,queue:["pcm"]});
  await act(async()=>root.unmount());root=createRoot(host);await act(async()=>root.render(view(false)));expect(posts()).toHaveLength(1);expect([...host.querySelectorAll<HTMLButtonElement>("button")].find(b=>b.textContent==="重试本次请求")!.disabled).toBe(true);await click("重试本次请求");expect(posts()).toHaveLength(1);await act(async()=>root.render(view()));await click("重试本次请求");expect(posts()[1][1]!.body).toBe(body);
 });
 it("新版计时要求整组的新能力，能力消失时禁止提交，默认单曲仍兼容旧输出",async()=>{
  const {api,request}=client(),normal=request.getMockImplementation()!;let missing="measured_listens_v2";
  request.mockImplementation(async(path,options)=>path==="/endpoints"?{items:endpoints.map((e,i)=>({...e,capabilities:{...e.capabilities,group_output_v2:true,measured_listens_v2:true,...(i?{[missing]:false}:{})}}))}:normal(path,options));
  const view=(enabled:boolean)=> <GroupPlayback api={api} coreId="core" userId="user" supported queueSupported outputTimingSupported={enabled} zones={zones} endpoints={endpoints} refresh={refresh}/>;
  await act(async()=>root.render(view(false)));await choose();expect(host.querySelector('[name="group-output-timing"]')).toBeNull();await act(async()=>root.render(view(true)));await act(async()=>host.querySelector<HTMLInputElement>('[name="group-output-timing"]')!.click());
  expect(host.textContent).toContain("此输出暂不支持新版分组计时");await submit();expect(request.mock.calls.some(([p])=>p==="/sync/starts")).toBe(false);
  missing="scheduled_queue_v1";await act(async()=>vi.advanceTimersByTimeAsync(2100));expect(host.textContent).toContain("此输出暂不支持连续组队列");await submit();expect(request.mock.calls.some(([p])=>p==="/sync/starts")).toBe(false);
  missing="unused";await act(async()=>vi.advanceTimersByTimeAsync(2100));await act(async()=>root.render(view(false)));expect(host.querySelector<HTMLButtonElement>('button.primary')!.disabled).toBe(true);await submit();expect(request.mock.calls.some(([p])=>p==="/sync/starts")).toBe(false);
  await act(async()=>root.render(view(true)));await act(async()=>host.querySelector<HTMLInputElement>('[name="group-output-timing"]')!.click());await submit();expect(JSON.parse(request.mock.calls.find(([p])=>p==="/sync/starts")![1]!.body as string).measured_version).toBeUndefined();
 });
 it("新版分组改组拒绝旧计时端点，不能提交不兼容成员",async()=>{
  const {api,request}=client(),id=crypto.randomUUID();selected(api,{...job(id,"paused"),measured_version:2,queue:{length:1,segment_start:0},members:[job(id).members[0]]});await act(async()=>root.render(render(api,"core","user",true,zones,true,true,true,true,true)));await click("调整分组成员");
  const checks=host.querySelectorAll<HTMLInputElement>('.group-form input[type="checkbox"]');expect(checks[1].disabled).toBe(true);expect(host.textContent).toContain("此输出暂不支持新版分组计时");await submit();expect(request.mock.calls.some(([p])=>p.endsWith("/control"))).toBe(false);
 });
 it("同步诊断默认折叠，显示估计值并在有效期后隐藏，旧核心不显示",async()=>{
  const perf=vi.spyOn(performance,"now").mockImplementation(()=>Date.now());
  try{const{api,request}=client(),id=crypto.randomUUID();const value=job(id,"playing");value.clock_diagnostics={version:1,state:"available",evaluated_at_core_ns:0,valid_for_ms:500,reference_zone_id:"a",total_uncertainty_ns:null,driver_accuracy_verified:false,strict_sync:false,correction:"off",members:zones.map((z,i)=>({zone_id:z.id,endpoint_id:z.id,available:true,reason:null,estimated_offset_ns:i?2500000:0,relative_drift_ppm:i?125:0,projection:"observed"}))};selected(api,value);
   await act(async()=>root.render(render(api,"core","user",true,zones,true,true,true,true,true,true)));const details=host.querySelector<HTMLDetailsElement>("details")!;expect(details.open).toBe(false);await act(async()=>details.querySelector("summary")!.click());expect(details.open).toBe(true);expect(details.textContent).toContain("估计领先 2.5 毫秒");expect(details.textContent).toContain("+125 ppm");
   await act(async()=>vi.advanceTimersByTimeAsync(501));expect(details.textContent).toContain("反馈已过期");expect(details.textContent).not.toContain("估计领先");expect(request.mock.calls.some(([,options])=>options?.method==="POST")).toBe(false);
   await act(async()=>root.render(render(api,"core","user",true,zones,true,true,true,true,true)));expect(host.textContent).not.toContain("同步诊断");
  }finally{perf.mockRestore();}
 });
 it("同步诊断扣除完整请求耗时，迟到的有效反馈不显示为当前偏差",async()=>{
  const perf=vi.spyOn(performance,"now").mockImplementation(()=>Date.now());
  try{const{api,request}=client(),normal=request.getMockImplementation()!,id=crypto.randomUUID(),value=job(id,"playing");value.clock_diagnostics={version:1,state:"available",evaluated_at_core_ns:0,valid_for_ms:500,reference_zone_id:"a",total_uncertainty_ns:null,driver_accuracy_verified:false,strict_sync:false,correction:"off",members:[{zone_id:"b",endpoint_id:"b",available:true,reason:null,estimated_offset_ns:2500000,relative_drift_ppm:null,projection:"nominal"}]};selected(api,value);
   request.mockImplementation(async(path,options)=>{if(path===`/sync/starts/${id}`)await new Promise(resolve=>setTimeout(resolve,600));return normal(path,options);});await act(async()=>root.render(render(api,"core","user",true,zones,true,true,true,true,true,true)));await act(async()=>vi.advanceTimersByTimeAsync(600));expect(host.textContent).toContain("反馈已过期");expect(host.textContent).not.toContain("估计领先");expect(request.mock.calls.some(([,options])=>options?.method==="POST")).toBe(false);
  }finally{perf.mockRestore();}
 });
 it("动态成员只提交明确改组，暂停加入保持暂停，成员变化后拦截旧选择",async()=>{
  const{api,request}=client(),id=crypto.randomUUID();selected(api,{...job(id,"playing"),queue:{length:2,segment_start:0}});await act(async()=>root.render(render(api,"core","user",true,zones,true,true,true,true,true)));
  await click("调整分组成员");const check=()=>[...host.querySelectorAll<HTMLInputElement>('.group-form input[type="checkbox"]')];await act(async()=>check()[1].click());await submit();let bodies=request.mock.calls.filter(([path])=>path.endsWith("/control")).map(([,options])=>JSON.parse(options!.body as string));expect(bodies[0]).toMatchObject({action:"members",from_zone_ids:["a","b"],zone_ids:["a"]});
  jobs.set(id,{...jobs.get(id)!,phase:"paused",members:[jobs.get(id)!.members[0]]});await act(async()=>vi.advanceTimersByTimeAsync(1100));await click("调整分组成员");await act(async()=>check()[1].click());await submit();bodies=request.mock.calls.filter(([path])=>path.endsWith("/control")).map(([,options])=>JSON.parse(options!.body as string));expect(bodies[1]).toMatchObject({action:"members",from_zone_ids:["a"],zone_ids:["a","b"]});expect(jobs.get(id)!.phase).toBe("paused");
  await click("调整分组成员");jobs.set(id,{...jobs.get(id)!,members:[job(id).members[0]]});await act(async()=>vi.advanceTimersByTimeAsync(1100));expect(host.textContent).toContain("分组成员已变化");expect([...host.querySelectorAll<HTMLButtonElement>("button")].find(b=>b.textContent==="应用成员调整")!.disabled).toBe(true);await act(async()=>root.render(render(api,"core","user",true,zones,true,true,true,true)));expect(host.textContent).not.toContain("调整分组成员");expect(request.mock.calls.some(([path])=>path==="/sync/starts")).toBe(false);
 });
 it("改组丢失响应后重载只查询，重试保留 UUID 与原成员",async()=>{
  const{api,request}=client(),normal=request.getMockImplementation()!,id=crypto.randomUUID();selected(api,job(id,"playing"));let first=true;request.mockImplementation(async(path,options)=>{if(path.endsWith("/control")&&first){first=false;throw new TypeError("lost membership response");}return normal(path,options);});
  await act(async()=>root.render(render(api,"core","user",true,zones,true,false,true,false,true)));await click("调整分组成员");await act(async()=>host.querySelectorAll<HTMLInputElement>('.group-form input[type="checkbox"]')[1].click());await submit();const posts=()=>request.mock.calls.filter(([path])=>path.endsWith("/control"));const body=posts()[0][1]!.body;await act(async()=>root.unmount());root=createRoot(host);await act(async()=>root.render(render(api,"core","user",true,zones,true,false,true,false,true)));await act(async()=>vi.advanceTimersByTimeAsync(2100));expect(posts()).toHaveLength(1);await click("重试本次控制");expect(posts()).toHaveLength(2);expect(posts()[1][1]!.body).toBe(body);
 });
 it("跳曲按钮与跨曲定位绑定原曲序，旧核心隐藏新控制",async()=>{
  const{api,request}=client(),id=crypto.randomUUID();selected(api,{...job(id,"playing"),queue:{length:3,segment_start:0}});await act(async()=>root.render(render(api,"core","user",true,zones,true,true,true,true)));
  expect([...host.querySelectorAll("button")].find(b=>b.textContent==="整组上一首")!.disabled).toBe(true);await click("整组下一首");await click("定位整组");
  await act(async()=>{for(const[label,value]of [["队列曲序（从 1 开始）","3"],["曲内位置（秒）","1.25"]]){const input=host.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`)!;Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(input,value);input.dispatchEvent(new Event("input",{bubbles:true}));}});await submit();
  expect([...host.querySelectorAll("button")].find(b=>b.textContent==="整组下一首")!.disabled).toBe(true);await click("整组上一首");const bodies=request.mock.calls.filter(([path])=>path.endsWith("/control")).map(([,options])=>JSON.parse(options!.body as string));expect(bodies.map(b=>[b.action,b.from_queue_index,b.queue_index,b.position])).toEqual([["jump",0,1,0],["jump",1,2,1.25],["jump",2,1,0]]);expect(new Set(bodies.map(b=>b.request_id)).size).toBe(3);
  await act(async()=>root.render(render(api,"core","user",true,zones,true,true,true)));expect(host.textContent).not.toContain("整组下一首");expect(host.textContent).not.toContain("整组上一首");expect(request.mock.calls.some(([path])=>path==="/sync/starts")).toBe(false);
 });
 it("跳曲丢失响应后重载不重发，重试保留相同 UUID 和原曲序",async()=>{
  const{api,request}=client(),normal=request.getMockImplementation()!,id=crypto.randomUUID();selected(api,{...job(id,"playing"),queue:{length:3,segment_start:0}});let first=true;request.mockImplementation(async(path,options)=>{if(path.endsWith("/control")&&first){first=false;throw new TypeError("lost jump response");}return normal(path,options);});
  await act(async()=>root.render(render(api,"core","user",true,zones,true,true,true,true)));await click("整组下一首");const posts=()=>request.mock.calls.filter(([path])=>path.endsWith("/control"));const body=posts()[0][1]!.body;await act(async()=>root.unmount());root=createRoot(host);await act(async()=>root.render(render(api,"core","user",true,zones,true,true,true,true)));await act(async()=>vi.advanceTimersByTimeAsync(2100));expect(posts()).toHaveLength(1);await click("重试本次控制");expect(posts()).toHaveLength(2);expect(posts()[1][1]!.body).toBe(body);expect(JSON.parse(body as string).from_queue_index).toBe(0);
 });
 it("连续队列保留重复曲序、移动移除，丢失响应与重载仍使用同一正文",async()=>{
  const {api,request}=client();const normal=request.getMockImplementation()!;let first=true;
  request.mockImplementation(async(path,options)=>{if(path==="/sync/starts"&&first){first=false;throw new TypeError("lost queue response");}return normal(path,options);});
  await act(async()=>root.render(render(api,"core","user",true,zones,true,true)));await choose();
  await act(async()=>host.querySelector<HTMLInputElement>('[name="group-queue-mode"]')!.click());
  const add=async(title:string)=>act(async()=>[...host.querySelectorAll<HTMLButtonElement>('.group-tracks button')].find(button=>button.textContent?.includes(title))!.click());
  await add("晨光");await add("夜航");await add("晨光");await click("上移第 3 首");await click("移除第 1 首");
  expect([...host.querySelectorAll('.group-queue li > span')].map(row=>row.textContent)).toEqual([expect.stringContaining("夜航"),expect.stringContaining("晨光"),expect.stringContaining("晨光")]);
  await submit();const posts=()=>request.mock.calls.filter(([path])=>path==="/sync/starts"),body=JSON.parse(posts()[0][1]!.body as string);
  expect(body.queue).toEqual(["pcm","low","low"]);expect(body.track_id).toBe("pcm");
  await act(async()=>root.unmount());root=createRoot(host);await act(async()=>root.render(render(api,"core","user",true,zones,true,true)));expect(posts()).toHaveLength(1);
  await click("重试本次请求");expect(JSON.parse(posts()[1][1]!.body as string)).toEqual(body);
  jobs.set(body.request_id,{...job(body.request_id,"playing"),queue:{length:3,segment_start:1},members:job(body.request_id).members.map(member=>({...member,queue_index:1,sample_rate:48000,frames:333}))});
  await act(async()=>vi.advanceTimersByTimeAsync(1100));expect(host.textContent).toContain("队列共 3 首");expect(host.textContent).toContain("第 2 / 3 首");
  await click("撤销整组");expect(request.mock.calls.some(([path])=>path===`/sync/starts/${body.request_id}/cancel`)).toBe(true);
 });
 it("连续队列拒绝旧网桥，单曲仍可使用，空队列不能起播",async()=>{
  const {api,request}=client();const normal=request.getMockImplementation()!;
  request.mockImplementation(async(path,options)=>path==="/endpoints"?{items:endpoints.map((endpoint,index)=>index?{...endpoint,capabilities:{...endpoint.capabilities,scheduled_queue_v1:false}}:endpoint)}:normal(path,options));
  await act(async()=>root.render(render(api,"core","user",true,zones,false,true)));await choose();
  const mode=host.querySelector<HTMLInputElement>('[name="group-queue-mode"]')!;await act(async()=>mode.click());
  expect(host.textContent).toContain("此输出暂不支持连续组队列");expect(host.querySelector<HTMLButtonElement>('button.primary')!.disabled).toBe(true);
  await click("移除第 1 首");await submit();expect(request.mock.calls.some(([path])=>path==="/sync/starts")).toBe(false);
  await act(async()=>mode.click());expect(host.querySelector<HTMLButtonElement>('button.primary')!.disabled).toBe(false);await submit();
  const body=JSON.parse(request.mock.calls.find(([path])=>path==="/sync/starts")![1]!.body as string);expect(body.queue).toBeUndefined();
 });
 it("无效的已保存队列不会被重试或查询",async()=>{
  const {api,request}=client();const key='rmusic.group-start.v1:'+JSON.stringify(["core","user",api.baseUrl]);
  sessionStorage.setItem(key,JSON.stringify({title:"损坏队列",request:{request_id:"00000000-0000-4000-8000-000000000001",track_id:"pcm",zone_ids:["a","b"],position:0,queue:["low","pcm"]}}));
  await act(async()=>root.render(render(api,"core","user",true,zones,false,true)));
  expect(host.textContent).not.toContain("损坏队列");expect(host.textContent).toContain("选择音区和歌曲");expect(request.mock.calls.some(([path])=>path.startsWith("/sync/starts"))).toBe(false);
 });
 it("查看其他控制端分组及重载只查询，可撤销但不重发起播请求",async()=>{
  const {api,request}=client();const id="00000000-0000-4000-8000-000000000001";jobs.set(id,{...job(id),title:"另一台客户端的歌曲",position:0.25});
  await act(async()=>root.render(render(api,"core","user",true,zones,true)));await click("查看其他分组");
  expect(request.mock.calls.some(([path])=>path==="/sync/starts?active=true&limit=32")).toBe(true);await act(async()=>host.querySelector<HTMLButtonElement>('.group-list button')!.click());
  expect(host.textContent).toContain("另一台客户端的歌曲");expect(host.textContent).toContain("撤销整组");expect(request.mock.calls.filter(([path])=>path==="/sync/starts")).toHaveLength(0);
  await act(async()=>root.unmount());root=createRoot(host);await act(async()=>root.render(render(api,"core","user",true,zones,true)));expect(host.textContent).toContain("另一台客户端的歌曲");
  jobs.delete(id);await act(async()=>vi.advanceTimersByTimeAsync(1100));expect(host.textContent).toContain("重新查询");expect(host.textContent).not.toContain("重试本次请求");await click("重新查询");expect(request.mock.calls.filter(([path])=>path==="/sync/starts")).toHaveLength(0);
  jobs.set(id,{...job(id),title:"另一台客户端的歌曲"});await act(async()=>vi.advanceTimersByTimeAsync(1100));await click("撤销整组");expect(host.textContent).toContain("已撤销");expect(request.mock.calls.some(([path])=>path===`/sync/starts/${id}/cancel`)).toBe(true);expect(request.mock.calls.filter(([path])=>path==="/sync/starts")).toHaveLength(0);
 });
 it("切换核心会取消分组列表查询并丢弃迟到结果",async()=>{
  const old=client(),next=client("http://new");let resolve!:(value:unknown)=>void;const normal=old.request.getMockImplementation()!;
  old.request.mockImplementation((path,options)=>path.startsWith("/sync/starts?")?new Promise(r=>{resolve=r;}):normal(path,options));
  await act(async()=>root.render(render(old.api,"core","user",true,zones,true)));await click("查看其他分组");const action=old.request.mock.calls.find(([path])=>path.startsWith("/sync/starts?"))!;
  await act(async()=>root.render(render(next.api,"new-core","user",true,zones,true)));expect(action[1]!.signal!.aborted).toBe(true);await act(async()=>resolve({items:[{...job("late"),title:"迟到分组"}],total:1}));expect(host.textContent).not.toContain("迟到分组");
 });
 it("只允许空闲且支持起播的不同输出，DSD 和旧核心不开放",async()=>{
  const {api}=client();await act(async()=>root.render(render(api,"core","user",false)));
  expect([...host.querySelectorAll("button")].find(b=>b.textContent==="选择音区和歌曲")!.disabled).toBe(true);
  const extra=[...zones,{...zones[0],id:"duplicate",name:"同一输出"},{...zones[1],id:"busy",name:"播放中",status:"playing"}] as Zone[];
  await act(async()=>root.render(render(api,"core","user",true,extra)));await click("选择音区和歌曲");await act(async()=>vi.advanceTimersByTimeAsync(300));
  const checks=[...host.querySelectorAll<HTMLInputElement>('.group-room input:not([name="group-queue-mode"])')];await act(async()=>checks[0].click());expect(checks[2].disabled).toBe(true);expect(checks[3].disabled).toBe(true);
  expect([...host.querySelectorAll<HTMLButtonElement>('.group-tracks button')].find(b=>b.textContent?.includes("DSD"))!.disabled).toBe(true);
  expect(host.textContent).toContain("严格同步尚未验收");
 });
 it("丢失响应后重试同一 UUID，重载只查询，撤销作用于整组",async()=>{
  const {api,request}=client();const normal=request.getMockImplementation()!;let first=true;
  request.mockImplementation(async(path,options)=>{if(path==="/sync/starts"&&first){first=false;throw new TypeError("connection lost");}return normal(path,options);});
  await act(async()=>root.render(render(api)));await choose();await submit();expect(host.textContent).toContain("尚未确认起播结果");
  const post=()=>request.mock.calls.filter(([p])=>p==="/sync/starts");const firstBody=JSON.parse(post()[0][1]!.body as string);
  expect(firstBody.zone_ids).toEqual(["a","b"]);expect(firstBody.track_id).toBe("pcm");
  await click("重试本次请求");expect(JSON.parse(post()[1][1]!.body as string)).toEqual(firstBody);expect(host.textContent).toContain("正在准备各音区");
  await act(async()=>root.unmount());root=createRoot(host);await act(async()=>root.render(render(api)));expect(post()).toHaveLength(2);expect(host.textContent).toContain("夜航");
  await click("撤销整组");expect(request.mock.calls.some(([path])=>path===`/sync/starts/${firstBody.request_id}/cancel`)).toBe(true);expect(host.textContent).toContain("已撤销");await click("选择另一首歌");expect(host.querySelector('[role="dialog"]')).not.toBeNull();
 });
 it("切换核心或账号取消未完成提交，迟到响应不更新新会话",async()=>{
  const old=client(),next=client("http://new");let resolve!:(value:SyncStart)=>void;const normal=old.request.getMockImplementation()!;
  old.request.mockImplementation((path,options)=>path==="/sync/starts"?new Promise(r=>{resolve=r;}):normal(path,options));
  await act(async()=>root.render(render(old.api)));await choose();await submit();const action=old.request.mock.calls.find(([p])=>p==="/sync/starts")!;
  await act(async()=>root.render(render(next.api,"new-core","new-user")));expect(action[1]!.signal!.aborted).toBe(true);
  await act(async()=>resolve(job(JSON.parse(action[1]!.body as string).request_id,"playing")));
  expect(host.textContent).not.toContain("夜航");expect(host.textContent).not.toContain("正在播放");expect(next.request.mock.calls.some(([p])=>p.startsWith("/sync/starts"))).toBe(false);
 });
 it("撤销后继续查询最终帧，收尾完成后停止轮询",async()=>{
  const {api,request}=client();const normal=request.getMockImplementation()!;
  request.mockImplementation(async(path,options)=>{const value=await normal(path,options);if(path.endsWith("/cancel")){const cancelled={...value as SyncStart,settled:false};jobs.set(cancelled.id,cancelled);return cancelled;}return value;});
  await act(async()=>root.render(render(api)));await choose();await submit();const id=[...jobs.keys()][0];await click("撤销整组");
  expect(host.textContent).toContain("正在等待各输出的最终消费帧");
  jobs.set(id,{...job(id,"cancelled"),settled:true,members:job(id,"cancelled").members.map(m=>({...m,frames:1110,final_report_received:true}))});
  await act(async()=>vi.advanceTimersByTimeAsync(1100));expect(host.textContent).not.toContain("正在等待各输出的最终消费帧");expect(host.textContent).toContain("1,110");
  const count=()=>request.mock.calls.filter(([path])=>path===`/sync/starts/${id}`).length;const finished=count();await act(async()=>vi.advanceTimersByTimeAsync(5000));expect(count()).toBe(finished);
  jobs.set(id,{...job(id,"cancelled"),settled:true,members:job(id,"cancelled").members.map(m=>({...m,final_report_received:false}))});
  await act(async()=>root.unmount());root=createRoot(host);await act(async()=>root.render(render(api)));expect(host.textContent).toContain("部分输出未确认最终帧");
 });
 it("明确拒绝且查询无回执才允许修改，未确认的网络错误不能另建分组",async()=>{
  const {api,request}=client();const normal=request.getMockImplementation()!;
  request.mockImplementation(async(path,options)=>{if(path==="/sync/starts")throw new ApiError(400,"bad_request","请先停止音区");return normal(path,options);});
  await act(async()=>root.render(render(api)));await choose();await submit();expect(host.textContent).toContain("请先停止音区");expect(host.textContent).toContain("选择另一首歌");await click("选择另一首歌");
  await act(async()=>vi.advanceTimersByTimeAsync(300));await act(async()=>host.querySelector<HTMLButtonElement>('.group-tracks button')!.click());
  request.mockImplementation(async(path,options)=>{if(path==="/sync/starts")throw new TypeError("lost");return normal(path,options);});await submit();
  expect(host.textContent).not.toContain("选择另一首歌");expect(host.textContent).toContain("重试本次请求");expect(host.textContent).not.toContain("选择音区和歌曲");
 });
});

describe("整组控制",()=>{
 it("旧核心隐藏入口，暂停和继续保持同一分组且使用各自的控制 UUID",async()=>{
  const {api,request}=client(),id=crypto.randomUUID();selected(api,{...job(id,"playing"),playhead:{queue_index:0,position:1.25},listened_seconds:1.25});
  await act(async()=>root.render(render(api)));expect(host.textContent).not.toContain("暂停整组");expect(host.textContent).not.toContain("定位整组");
  await act(async()=>root.render(render(api,"core","user",true,zones,true,true,true)));await click("暂停整组");expect(host.textContent).toContain("整组已暂停");await click("继续整组");
  const posts=request.mock.calls.filter(([path])=>path.endsWith("/control"));expect(posts).toHaveLength(2);const bodies=posts.map(([,options])=>JSON.parse(options!.body as string));expect(bodies.map(body=>body.action)).toEqual(["pause","resume"]);expect(bodies[0].request_id).not.toBe(bodies[1].request_id);expect(bodies[0].request_id).not.toBe(id);expect(posts.every(([path])=>path===`/sync/starts/${id}/control`)).toBe(true);expect(request.mock.calls.some(([path])=>path==="/sync/starts")).toBe(false);
 });
 it("控制丢失响应后重载只查询，显式重试复用原 UUID 和正文",async()=>{
  const {api,request}=client(),normal=request.getMockImplementation()!,id=crypto.randomUUID();selected(api,job(id,"playing"));let first=true;
  request.mockImplementation(async(path,options)=>{if(path.endsWith("/control")&&first){first=false;throw new TypeError("lost control response");}return normal(path,options);});
  await act(async()=>root.render(render(api,"core","user",true,zones,true,true,true)));await click("暂停整组");expect(host.textContent).toContain("重试本次控制");
  const posts=()=>request.mock.calls.filter(([path])=>path.endsWith("/control"));const body=posts()[0][1]!.body;
  await act(async()=>root.unmount());root=createRoot(host);await act(async()=>root.render(render(api,"core","user",true,zones,true,true,true)));await act(async()=>vi.advanceTimersByTimeAsync(2100));expect(posts()).toHaveLength(1);
  await click("重试本次控制");expect(posts()).toHaveLength(2);expect(posts()[1][1]!.body).toBe(body);expect(host.textContent).toContain("整组已暂停");expect(host.textContent).not.toContain("重试本次控制");
 });
 it("已接受但丢失的响应通过回显确认，明确拒绝不自动补发",async()=>{
  const {api,request}=client(),normal=request.getMockImplementation()!,id=crypto.randomUUID();selected(api,job(id,"playing"));let lost=true,deny=false;
  request.mockImplementation(async(path,options)=>{if(path.endsWith("/control")){if(deny)throw new ApiError(403,"forbidden","没有整组控制权限");if(lost){lost=false;await normal(path,options);throw new TypeError("lost accepted response");}}return normal(path,options);});
  await act(async()=>root.render(render(api,"core","user",true,zones,true,true,true)));await click("暂停整组");expect(host.textContent).toContain("整组已暂停");expect(host.textContent).not.toContain("重试本次控制");
  deny=true;await click("继续整组");expect(host.textContent).toContain("没有整组控制权限");await act(async()=>vi.advanceTimersByTimeAsync(3100));expect(request.mock.calls.filter(([path])=>path.endsWith("/control"))).toHaveLength(2);await click("返回当前状态");expect(host.textContent).not.toContain("本次控制请求未被接受");
 });
 it("定位使用整组当前曲序，编辑期间切歌要求重新定位",async()=>{
  const {api,request}=client(),id=crypto.randomUUID();selected(api,{...job(id,"playing"),queue:{length:4,segment_start:0},playhead:{queue_index:2,position:7.0000001}});
  await act(async()=>root.render(render(api,"core","user",true,zones,true,true,true)));await click("定位整组");expect(host.querySelector<HTMLInputElement>('[aria-label="曲内位置（秒）"]')!.value).toBe("7.0000001");
  jobs.set(id,{...jobs.get(id)!,playhead:{queue_index:3,position:0.01}});await act(async()=>vi.advanceTimersByTimeAsync(1100));expect(host.textContent).toContain("歌曲已切换");expect(host.querySelector<HTMLButtonElement>('button[type="submit"]')!.disabled).toBe(true);expect(request.mock.calls.some(([path])=>path.endsWith("/control"))).toBe(false);
  await click("关闭");await click("定位整组");const input=host.querySelector<HTMLInputElement>('[aria-label="曲内位置（秒）"]')!;
  await act(async()=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(input,"0.25");input.dispatchEvent(new Event("input",{bubbles:true}));});await submit();
  const post=request.mock.calls.find(([path])=>path.endsWith("/control"))!;expect(JSON.parse(post[1]!.body as string)).toMatchObject({action:"seek",queue_index:3,position:0.25});
 });
 it("切换核心或账号会取消控制提交并丢弃迟到响应",async()=>{
  const old=client(),next=client("http://next"),id=crypto.randomUUID();selected(old.api,job(id,"playing"));const normal=old.request.getMockImplementation()!;let resolve!:(value:SyncStart)=>void;
  old.request.mockImplementation(async(path,options)=>path.endsWith("/control")?new Promise<SyncStart>(done=>{resolve=done;}):normal(path,options));
  await act(async()=>root.render(render(old.api,"core","user",true,zones,true,true,true)));await click("暂停整组");const action=old.request.mock.calls.find(([path])=>path.endsWith("/control"))!;
  await act(async()=>root.render(render(next.api,"next-core","next-user",true,zones,true,true,true)));expect(action[1]!.signal!.aborted).toBe(true);await act(async()=>resolve({...job(id,"paused"),title:"迟到控制"}));expect(host.textContent).not.toContain("迟到控制");expect(host.textContent).toContain("选择音区和歌曲");expect(next.request.mock.calls.some(([path])=>path.endsWith("/control"))).toBe(false);
 });
});
