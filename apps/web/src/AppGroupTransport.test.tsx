// @vitest-environment jsdom
import {act} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {RMusicClient,type Zone,type SyncStart} from "@r-music/sdk";
import {App} from "./App";
const audio=vi.hoisted(()=>({interrupt:vi.fn(async()=>{}),unlock:vi.fn(async()=>{}),seek:vi.fn(),finishChange:vi.fn()}));
vi.mock("./native",async()=>({isNative:true,isAndroidClient:false,restoreWindowAppearance:()=>{},NativeClient:(await import("@r-music/sdk")).RMusicClient,nativeProfiles:async()=>[],nativeProfile:async()=>({id:"profile"}),nativeOutput:()=>({}),invokeNative:async(cmd:string)=>cmd==="native_capabilities"?{native_group_v1:true}:cmd==="native_endpoint_register"?{id:"browser-qa",capabilities:{native_group_v1:true}}:undefined}));
vi.mock("./NativeOffline",()=>({NativeOffline:()=>null}));
vi.mock("./useOutputOwner",()=>({useOutputOwner:()=>true}));
vi.mock("./useAudio",()=>({useAudio:(_api:unknown,z:Zone)=>({position:0,playing:z?.status==="playing",local:true,adaptive:0,nativeState:{group_managed:true,zone_id:z?.id},sourceInfo:null,sourceError:"",contextSampleRate:0,analyserRef:{current:null},...audio})}));
vi.mock("./browserCache",()=>({browserCache:{register:async()=>false,list:async()=>[],protect:()=>{}},formatBytes:(n:number)=>`${n} B`}));
let root:Root,host:HTMLDivElement,zone:Zone,group:SyncStart,request:ReturnType<typeof vi.spyOn>,grouped:boolean;
const calls=()=>request.mock.calls as [string,RequestInit|undefined][];
class Socket {onopen=null;onclose=null;onmessage=null;close(){}}
beforeEach(()=>{
 vi.useFakeTimers();vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);vi.stubGlobal("WebSocket",Socket);vi.stubGlobal("matchMedia",()=>({matches:false,addEventListener:()=>{},removeEventListener:()=>{}}));localStorage.clear();sessionStorage.clear();localStorage.setItem("rmusic.endpoint-id","browser-qa");sessionStorage.setItem("rmusic.token","synthetic");for(const value of Object.values(audio))value.mockClear();
 const track={id:"a",title:"第一首",artist:"QA",duration:4,format:"wav",sample_rate:96000,bit_depth:32,available:true};zone={id:"z",name:"本机",endpoint_id:"browser-qa",status:"playing",volume:1,muted:false,position:0,queue_index:0,track,queue:[track,{...track,id:"b",title:"第二首"}],dsp:{enabled:false},transitions:{},updated_at:Date.now()} as Zone;group={id:"11111111-1111-4111-8111-111111111111",phase:"playing",queue:{length:2,segment_start:0},playhead:{queue_index:0,position:0},members:[{zone_id:"z"}]} as SyncStart;grouped=true;
 request=vi.spyOn(RMusicClient.prototype,"request").mockImplementation(async(path,options)=>{
  if(path==="/status")return {initialized:true,core_id:"core",capabilities:{group_start_list_v1:true,group_transport_v1:true,group_queue_jump_v1:true}};
  if(path==="/auth/me")return {id:"u",username:"qa",role:"admin"};if(path==="/events/ticket")return {ticket:"synthetic"};if(path==="/preferences")return {};
  if(path==="/zones")return {items:[zone]};if(path==="/endpoints")return {items:[{id:"browser-qa",online:true,capabilities:{native_group_v1:true}}]};if(path.startsWith("/sync/starts?"))return {items:grouped?[group]:[],total:grouped?1:0};
  if(path.endsWith("/control")){const b=JSON.parse(options!.body as string);zone={...zone,status:b.action==="pause"?"paused":b.action==="resume"?"playing":zone.status,queue_index:b.queue_index??zone.queue_index,position:b.position??zone.position};zone.track=zone.queue[zone.queue_index];group={...group,phase:zone.status as SyncStart["phase"],playhead:{queue_index:zone.queue_index,position:zone.position},transport:{request_id:b.request_id,action:b.action,state:"applied",revision:1,error:null,incomplete_final_reports:false}};return group;}
  if(path.startsWith("/zones/z/"))return zone;return {items:[]};
 });host=document.createElement("div");document.body.append(host);root=createRoot(host);
});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.restoreAllMocks();vi.useRealTimers();vi.unstubAllGlobals();});
async function press(label:string){await act(async()=>host.querySelector<HTMLButtonElement>(`footer button[aria-label="${label}"]`)!.click());}
async function seek(value:number){const input=host.querySelector<HTMLInputElement>('footer input[aria-label="播放进度"]')!;await act(async()=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(input,String(value));input.dispatchEvent(new Event("input",{bubbles:true}));});}
it("原生底部暂停继续定位和上下曲使用整组协议，不提前中断或定位本机输出",async()=>{
 await act(async()=>root.render(<App/>));await press("暂停");expect(calls().filter(([p,o])=>o?.method==="POST"&&(p.startsWith("/zones/z/")||p.endsWith("/control"))).map(([p])=>p)).toEqual([`/sync/starts/${group.id}/control`]);await seek(1.25);expect(host.querySelector<HTMLInputElement>('footer input[aria-label="播放进度"]')!.value).toBe("1.25");await press("下一首");await press("上一首");await press("播放");
 const controls=calls().filter(([p])=>p.endsWith("/control")).map(([,o])=>JSON.parse(o!.body as string));expect(controls.map(b=>b.action)).toEqual(["pause","seek","jump","jump","resume"]);expect(controls[1]).toMatchObject({queue_index:0,position:1.25});expect(calls().some(([p,o])=>p.startsWith("/zones/z/")&&o?.method==="POST")).toBe(false);for(const value of Object.values(audio))expect(value).not.toHaveBeenCalled();
});
it("无活动分组时保留普通单音区控制和本机定位",async()=>{
 grouped=false;await act(async()=>root.render(<App/>));await press("暂停");await seek(1.25);expect(calls().some(([p])=>p==="/zones/z/pause")).toBe(true);expect(calls().some(([p])=>p==="/zones/z/seek")).toBe(true);expect(audio.seek).toHaveBeenCalledWith(1.25);expect(calls().some(([p])=>p.endsWith("/control"))).toBe(false);
});
