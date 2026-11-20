// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect,it,vi} from "vitest";
import {RMusicClient} from "@r-music/sdk";
import {App} from "./App";

const native=vi.hoisted(()=>({
  handlers:new Map<string,(event:{payload:unknown})=>void>(),
  visible:true,
}));
vi.mock("@tauri-apps/api/event",()=>({
  listen:async(name:string,handler:(event:{payload:unknown})=>void)=>{
    native.handlers.set(name,handler);
    return ()=>native.handlers.delete(name);
  },
}));
vi.mock("./native",async()=>({
  isNative:true,isAndroidClient:true,
  NativeClient:(await import("@r-music/sdk")).RMusicClient,
  nativeProfile:async()=>({id:"fixture"}),nativeProfiles:async()=>[],
  nativeOutput:()=>({}),restoreWindowAppearance:()=>{},nativeBufferedEnd:()=>undefined,
  invokeNative:async(command:string)=>command==="native_session_available"?true:undefined,
}));
vi.mock("./useAudio",()=>({useAudio:()=>({
  position:0,playing:false,local:false,adaptive:0,nativeState:{},analyserRef:{current:null},
})}));
vi.mock("./useOutputOwner",()=>({useOutputOwner:()=>false}));
vi.mock("./browserCache",()=>({browserCache:{register:async()=>false,protect:()=>{}},formatBytes:String}));

it("keeps the offline state when Android returns its local queue and refreshes once on resume",async()=>{
  vi.useFakeTimers();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
  vi.stubGlobal("matchMedia",()=>({matches:false,addEventListener:()=>{},removeEventListener:()=>{}}));
  vi.stubGlobal("RMusicAppearance",{isVisible:()=>native.visible});
  vi.spyOn(document,"hidden","get").mockReturnValue(false);
  localStorage.setItem("rmusic.core",JSON.stringify("https://core.test"));
  let coreReachable:boolean|null=null;
  let zones=0;
  vi.spyOn(RMusicClient.prototype,"request").mockImplementation(async path=>{
    if(path==="/status")return {initialized:true,core_id:"fixture",capabilities:{}};
    if(path==="/auth/me")return {id:"family",username:"QA",role:"admin"};
    if(path==="/zones"){zones++;return {items:[],core_reachable:coreReachable};}
    if(path==="/preferences")return {};
    return {items:[]};
  });
  const host=document.createElement("div");document.body.append(host);
  const root=createRoot(host);
  const offline=()=>host.textContent?.includes("与核心的连接已中断");
  const send=(name:string,payload?:unknown)=>native.handlers.get(name)?.({payload});
  try{
    await act(async()=>root.render(<App/>));
    expect(native.handlers.has("native-core-event")).toBe(true);
    expect(offline()).toBe(false);
    await act(async()=>send("native-core-event",{type:"connection_interrupted"}));
    expect(offline()).toBe(true);
    await act(async()=>{
      send("native-client-remote");send("native-client-remote");
      await vi.advanceTimersByTimeAsync(60);
    });
    expect(offline()).toBe(true);
    const beforeResume=zones;
    native.visible=false;
    await act(async()=>window.dispatchEvent(new Event("rmusic-activity-visibility")));
    coreReachable=true;
    await act(async()=>send("native-client-remote"));
    expect(zones).toBe(beforeResume);
    native.visible=true;
    await act(async()=>{
      window.dispatchEvent(new Event("rmusic-activity-visibility"));
      send("native-client-remote");
      await vi.advanceTimersByTimeAsync(60);
    });
    expect(zones).toBe(beforeResume+1);
    expect(offline()).toBe(false);
  }finally{
    await act(async()=>root.unmount());
    host.remove();
    localStorage.clear();
    native.handlers.clear();
    native.visible=true;
    vi.useRealTimers();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  }
});
