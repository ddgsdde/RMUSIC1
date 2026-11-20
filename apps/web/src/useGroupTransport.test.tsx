// @vitest-environment jsdom
import {act} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {ApiError,RMusicClient,type SyncStart,type Zone} from "@r-music/sdk";
import {useGroupTransport} from "./useGroupTransport";
let root:Root,host:HTMLDivElement,view:ReturnType<typeof useGroupTransport>,group:SyncStart;
const error=vi.fn(),change=vi.fn(),id="11111111-1111-4111-8111-111111111111";
const zone={id:"z",queue_index:1,queue:[],endpoint_id:"a"} as unknown as Zone;
function Harness({api,user="u",core="c",supported=true}:{api:RMusicClient;user?:string;core?:string;supported?:boolean}){view=useGroupTransport({api,coreId:core,userId:user,list:true,supported,jump:true,recovery:true,onError:error,onChange:change});return <div>{view.pending?.title}</div>;}
function client(base="http://qa") {
 const api=new RMusicClient(base,"synthetic"),request=vi.spyOn(api,"request").mockImplementation(async(path,options)=>{
  if(path.startsWith("/sync/starts?"))return {items:[group],total:1};
  if(path.endsWith("/control")){const body=JSON.parse(options!.body as string);group={...group,phase:body.action==="pause"?"paused":body.action==="resume"?"playing":group.phase,playhead:body.queue_index===undefined?group.playhead:{queue_index:body.queue_index,position:body.position},transport:{request_id:body.request_id,action:body.action,state:"applied",revision:1,error:null,incomplete_final_reports:false}};}
  return group;
 });return {api,request};
}
async function route(action:string,body?:unknown,z=zone){let handled:boolean|undefined;await act(async()=>{handled=await view.route(z,action,body,true);});return handled;}
beforeEach(()=>{vi.useFakeTimers();vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);sessionStorage.clear();error.mockClear();change.mockClear();group={id,title:"两房分组",phase:"playing",queue:{length:3,segment_start:0},playhead:{queue_index:1,position:2},members:[{zone_id:"z"}]} as SyncStart;host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.restoreAllMocks();vi.useRealTimers();vi.unstubAllGlobals();});
it("暂停继续定位与曲序选择均生成整组请求，已接受的准备只轮询",async()=>{
 const {api,request}=client(),normal=request.getMockImplementation()!;let preparing=true;
 request.mockImplementation(async(path,options)=>{const value=await normal(path,options);if(path.endsWith("/control")&&preparing){group={...group,phase:"pausing",transport:{...group.transport!,state:"pending"}};return group;}return value;});
 await act(async()=>root.render(<Harness api={api}/>));expect(await route("pause")).toBe(true);expect(view.pending?.confirmed).toBe(true);expect(view.busy).toBe(true);await route("next");expect(request.mock.calls.filter(([p])=>p.endsWith("/control"))).toHaveLength(1);
 preparing=false;group={...group,phase:"paused",transport:{...group.transport!,state:"applied"}};await act(async()=>vi.advanceTimersByTimeAsync(1100));expect(view.pending).toBeUndefined();
 await route("play");await route("seek",{position:1.25});await route("next");await route("previous",undefined,{...zone,queue_index:2});await route("play",{queue_index:1});await route("play",{queue_index:0});
 const bodies=request.mock.calls.filter(([p])=>p.endsWith("/control")).map(([,o])=>JSON.parse(o!.body as string));expect(bodies.map(b=>b.action)).toEqual(["pause","resume","seek","jump","jump","seek","jump"]);expect(bodies[2]).toMatchObject({queue_index:1,position:1.25});expect(bodies[3]).toMatchObject({from_queue_index:1,queue_index:2,position:0});expect(new Set(bodies.map(b=>b.request_id)).size).toBe(7);
 expect(view.route(zone,"play",{track_id:"new"},true)).toBe(false);expect(view.route(zone,"play",undefined,false)).toBe(false);expect(request.mock.calls.some(([p])=>p.startsWith("/zones/"))).toBe(false);
});
it("丢失响应后重载仅查询，显式重试保留 UUID、曲序和正文",async()=>{
 const {api,request}=client(),normal=request.getMockImplementation()!;let lost=true;request.mockImplementation(async(path,options)=>{if(path.endsWith("/control")&&lost){lost=false;throw new TypeError("lost");}return normal(path,options);});
 await act(async()=>root.render(<Harness api={api}/>));await route("seek",{position:1.25});const posts=()=>request.mock.calls.filter(([p])=>p.endsWith("/control"));const frozen=posts()[0][1]!.body;expect(view.pending?.confirmed).not.toBe(true);
 await act(async()=>root.unmount());root=createRoot(host);await act(async()=>root.render(<Harness api={api}/>));await act(async()=>vi.advanceTimersByTimeAsync(2100));expect(posts()).toHaveLength(1);await act(async()=>view.retry());expect(posts()).toHaveLength(2);expect(posts()[1][1]!.body).toBe(frozen);expect(view.pending).toBeUndefined();
});
it("切换核心或账号取消旧提交，旧响应不能写入新会话或跨核心查询",async()=>{
 const old=client(),next=client("http://next"),normal=old.request.getMockImplementation()!;let resolve!:(value:unknown)=>void;
 old.request.mockImplementation((path,options)=>path.endsWith("/control")?new Promise(r=>{resolve=r;}):normal(path,options));await act(async()=>root.render(<Harness api={old.api}/>));let pending:unknown;await act(async()=>{pending=view.route(zone,"pause",undefined,true);await Promise.resolve();});const post=old.request.mock.calls.find(([p])=>p.endsWith("/control"))!;
 await act(async()=>root.render(<Harness api={next.api} core="next" user="other"/>));expect(post[1]!.signal!.aborted).toBe(true);expect(view.pending).toBeUndefined();expect(next.request).not.toHaveBeenCalled();await act(async()=>{resolve(group);await pending;});expect(view.pending).toBeUndefined();expect(next.request).not.toHaveBeenCalled();expect(host.textContent).not.toContain("两房分组");
});
it("查询失败不降级单端；明确拒绝后返回状态才能新建请求，中断组不自动恢复",async()=>{
 const {api,request}=client(),normal=request.getMockImplementation()!;let lookupFails=true,deny=true;
 request.mockImplementation(async(path,options)=>{if(path.startsWith("/sync/starts?")&&lookupFails)throw new TypeError("offline");if(path.endsWith("/control")&&deny)throw new ApiError(409,"conflict","曲序已变化");return normal(path,options);});await act(async()=>root.render(<Harness api={api}/>));expect(await route("pause")).toBe(true);expect(request.mock.calls.some(([,o])=>o?.method==="POST")).toBe(false);
 lookupFails=false;await route("pause");expect(view.pending?.rejected).toBe(true);await act(async()=>vi.advanceTimersByTimeAsync(5100));const first=JSON.parse(request.mock.calls.find(([p])=>p.endsWith("/control"))![1]!.body as string);await route("pause");expect(request.mock.calls.filter(([p])=>p.endsWith("/control"))).toHaveLength(1);await act(async()=>view.dismiss());deny=false;await route("pause");const second=JSON.parse(request.mock.calls.filter(([p])=>p.endsWith("/control"))[1][1]!.body as string);expect(second.request_id).not.toBe(first.request_id);
 group={...group,phase:"interrupted"};const count=request.mock.calls.length;expect(await route("play")).toBe(true);expect(request.mock.calls.slice(count).some(([,o])=>o?.method==="POST")).toBe(false);
});
it("无分组走普通路径，旧核心能力、队列边界与存储失败不发送组控制",async()=>{
 const {api,request}=client(),normal=request.getMockImplementation()!;let empty=true;request.mockImplementation((path,options)=>path.startsWith("/sync/starts?")&&empty?Promise.resolve({items:[],total:0}):normal(path,options));await act(async()=>root.render(<Harness api={api}/>));expect(await route("pause")).toBe(false);empty=false;
 await act(async()=>root.render(<Harness api={api} supported={false}/>));expect(await route("pause")).toBe(true);await act(async()=>root.render(<Harness api={api}/>));group={...group,playhead:{queue_index:0,position:0}};expect(await route("previous",undefined,{...zone,queue_index:0})).toBe(true);
 const store=vi.spyOn(Storage.prototype,"setItem").mockImplementation(()=>{throw new Error("quota");});expect(await route("pause")).toBe(true);expect(view.pending).toBeUndefined();expect(request.mock.calls.some(([,o])=>o?.method==="POST")).toBe(false);store.mockRestore();
});
