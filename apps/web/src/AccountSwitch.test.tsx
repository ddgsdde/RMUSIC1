// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect,it,vi} from "vitest";
import {ApiError,RMusicClient} from "@r-music/sdk";
import {App} from "./App";
const identity=vi.hoisted(()=>({user:"a"}));
vi.mock("./Auth",()=>({Auth:({onAuth}:{onAuth:(v:unknown)=>void})=><div>欢迎回来<button onClick={()=>{identity.user="b";onAuth({token:"new-user-b",user:{id:"b",username:"B",role:"external"}});}}>登录测试 B</button></div>}));
vi.mock("./native",()=>({isNative:false,isAndroidClient:false,restoreWindowAppearance:()=>{},nativeProfiles:async()=>[]}));
vi.mock("./useOutputOwner",()=>({useOutputOwner:()=>false}));
vi.mock("./useAudio",()=>({useAudio:()=>({position:0,playing:false,local:false,adaptive:0,nativeState:{},sourceInfo:null,sourceError:"",contextSampleRate:0,analyserRef:{current:null},interrupt:async()=>{},unlock:async()=>{},seek:()=>{},finishChange:()=>{}})}));
vi.mock("./browserCache",()=>({browserCache:{bind:vi.fn(),register:async()=>false,list:async()=>[],protect:()=>{}},formatBytes:(n:number)=>`${n} B`}));
it("会话失效后切换账号，不保留旧账号的歌曲详情、队列和导航状态",async()=>{
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);vi.useFakeTimers();localStorage.clear();sessionStorage.clear();sessionStorage.setItem("rmusic.token","old-user-a");localStorage.setItem("rmusic.endpoint-id","browser-qa");
 vi.stubGlobal("matchMedia",()=>({matches:false,addEventListener(){},removeEventListener(){}}));vi.stubGlobal("WebSocket",class{close(){}});
 let rejectEndpoints!:(v:unknown)=>void;const endpoints=new Promise((_,reject)=>{rejectEndpoints=reject;});
 const track={id:"private-a",title:"PRIVATE A TRACK",artist:"Private artist",album:"Private album",album_artist:"Private artist",format:"flac",source:"local",duration:30,sample_rate:44100,bit_depth:16,channels:2,available:true,created_at:0,provider_data:{}};
 const request=vi.spyOn(RMusicClient.prototype,"request").mockImplementation(async(path:string)=>{
  if(path==="/status")return{initialized:true,core_id:"core"};if(path==="/auth/me")return{id:identity.user,username:identity.user,role:"external"};if(path==="/preferences")return{};if(path==="/events/ticket")return{ticket:"qa"};if(path==="/endpoints")return identity.user==="a"?endpoints:{items:[]};
  if(path==="/zones")return identity.user==="a"?{items:[{id:"zone-a",name:"A device",endpoint_id:"browser-qa-u-a",track,queue:[track],queue_index:0,status:"paused",position:3,shuffle:false,repeat:"off",dsp:{enabled:false},volume:1}]}:new Promise(()=>{});
  if(path.endsWith("/background?refresh=false"))return{background:{}};return{items:[]};
 });
 const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 const click=async(name:string)=>{const b=[...host.querySelectorAll("button")].find(b=>b.getAttribute("aria-label")===name||b.textContent===name);expect(b,`missing ${name}`).toBeTruthy();await act(async()=>b!.click());};
 try{
  await act(async()=>root.render(<App/>));expect(host.textContent).toContain("PRIVATE A TRACK");await click("关于当前歌曲");expect(host.querySelector('[role="dialog"][aria-label="关于这首歌"]')).toBeTruthy();
  await act(async()=>rejectEndpoints(new ApiError(401,"unauthorized","会话已失效")));expect(host.textContent).toContain("欢迎回来");await click("登录测试 B");expect(host.textContent).toContain("核心已连接");
  expect(host.textContent).not.toContain("PRIVATE A TRACK");expect(host.querySelector('[role="dialog"][aria-label="关于这首歌"]')).toBeNull();
 }finally{await act(async()=>root.unmount());host.remove();request.mockRestore();vi.useRealTimers();vi.unstubAllGlobals();}
});
