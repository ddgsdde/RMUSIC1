// @vitest-environment jsdom
import {act} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {NativeOffline} from "./NativeCache";
const native=vi.hoisted(()=>({invoke:vi.fn()}));
vi.mock("./NativeStorageSettings",()=>({NativeStorageSettings:()=>null}));
vi.mock("./native",()=>({invokeNative:native.invoke,nativeOutput:()=>({exclusive:true,buffer_ms:100})}));
const entry={id:"cached",track_id:"song",title:"离线验收曲",complete:true,format:"flac",sample_rate:768000,channels:2,total_bytes:100};
let host:HTMLDivElement,root:Root;
beforeEach(()=>{vi.useFakeTimers();vi.clearAllMocks();vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.useRealTimers();vi.unstubAllGlobals();});
it("设备中断后手动继续保留位置，自然结束或点选歌曲从头播放，重载不自动播放",async()=>{
 const status={offline:true,cache_id:entry.id,track_id:entry.track_id,playback_id:"interrupted",status:"error",position:7.25,duration:60,paused:true};
 native.invoke.mockImplementation(async(command:string)=>command==="native_cache_status"?{entries:[entry]}:command==="native_status"?{...status}:undefined);
 await act(async()=>root.render(<NativeOffline api={{baseUrl:"http://core",token:"qa"}} onError={vi.fn()}/>));
 expect(native.invoke.mock.calls.some(([command])=>command==="native_offline_play")).toBe(false);
 await act(async()=>host.querySelector<HTMLButtonElement>(".offline-controls button")!.click());
 let args=native.invoke.mock.calls.filter(([command])=>command==="native_offline_play").at(-1)![1];
 expect(args.position).toBe(7.25);expect(host.textContent).toContain("0:07");
 Object.assign(status,{playback_id:args.playbackId,status:"ended",position:60});
 await act(async()=>vi.advanceTimersByTime(1000));
 await act(async()=>host.querySelector<HTMLButtonElement>(".offline-controls button")!.click());
 args=native.invoke.mock.calls.filter(([command])=>command==="native_offline_play").at(-1)![1];expect(args.position).toBe(0);
 await act(async()=>host.querySelector<HTMLButtonElement>(".download-list button")!.click());
 expect(native.invoke.mock.calls.filter(([command])=>command==="native_offline_play").at(-1)![1].position).toBe(0);
});
it("安卓冷启动离线入口使用本机队列，缓存播放状态可恢复并暂停",async()=>{
 vi.stubGlobal("navigator",{userAgent:"Android"});localStorage.setItem("rmusic.endpoint-id","phone");
 const cached={...entry,profile_id:"saved-core",recording_id:"recording"};let paused=false;
 const zone={id:"client-zone-phone",track:{id:entry.track_id},playback_id:"local-instance",position:4,status:"playing"};
 native.invoke.mockImplementation(async(command:string,args:any)=>{
  if(command==="native_cache_status")return {entries:[cached]};
  if(command==="native_download_tasks")return {items:[]};
  if(command==="native_status")return {status:"playing",cache_id:entry.id,track_id:entry.track_id,playback_id:zone.playback_id,position:4,duration:60,paused};
  if(command==="native_request"){
   if(args.path.endsWith("/pause"))paused=true;
   return zone;
  }
  return {};
 });
 await act(async()=>root.render(<NativeOffline api={{baseUrl:"http://offline-core",token:""}} onError={vi.fn()}/>));
 await act(async()=>host.querySelector<HTMLButtonElement>(".download-list button")!.click());
 await act(async()=>vi.advanceTimersByTime(1000));
 expect(host.textContent).toContain("0:04");expect(host.querySelector('[aria-label="离线下一首"]')).toBeTruthy();
 expect(native.invoke.mock.calls.some(([command])=>command==="native_offline_play")).toBe(false);
 await act(async()=>[...host.querySelectorAll('button')].find(b=>b.textContent?.trim()==="暂停")!.click());
 expect(paused).toBe(true);expect(host.textContent).toContain("继续");
});
