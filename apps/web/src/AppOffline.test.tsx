// @vitest-environment jsdom
import {act} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {App} from "./App";
import {ApiError} from "@r-music/sdk";
const runtime=vi.hoisted(()=>({request:vi.fn(),invoke:vi.fn(),listen:vi.fn()}));
vi.mock("./native",async()=>{const sdk=await import("@r-music/sdk");return {isNative:true,isAndroidClient:false,restoreWindowAppearance:()=>{},NativeClient:class extends sdk.RMusicClient {override request<T>(path:string,options?:RequestInit):Promise<T>{return runtime.request(path,options);}},nativeProfiles:async()=>[],nativeProfile:async()=>({id:"offline-test"}),nativeOutput:()=>({exclusive:false,buffer_ms:100}),invokeNative:runtime.invoke,listenNative:runtime.listen};});
vi.mock("./useOutputOwner",()=>({useOutputOwner:()=>true}));
vi.mock("./useAudio",()=>({useAudio:()=>({position:0,playing:false,local:false,adaptive:0,nativeState:{},sourceInfo:null,sourceError:"",contextSampleRate:0,analyserRef:{current:null},interrupt:async()=>{},unlock:async()=>{},seek:()=>{},finishChange:()=>{}})}));
vi.mock("./browserCache",()=>({browserCache:{register:async()=>false,list:async()=>[],protect:()=>{}},formatBytes:(n:number)=>`${n} B`}));
let root:Root,host:HTMLDivElement;
function deferred<T>(){let resolve!:(value:T)=>void,reject!:(error:Error)=>void;const promise=new Promise<T>((yes,no)=>{resolve=yes;reject=no;});return{promise,resolve,reject};}
beforeEach(()=>{
 vi.useFakeTimers();vi.clearAllMocks();localStorage.clear();sessionStorage.clear();vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
 vi.stubGlobal("matchMedia",()=>({matches:false,addEventListener:()=>{},removeEventListener:()=>{}}));
 runtime.invoke.mockImplementation(async(command:string)=>command==="native_cache_status"?{entries:[{id:"cached",track_id:"offline-song",title:"离线验收曲",complete:true,verified:true,format:"flac",sample_rate:96000,channels:2,total_bytes:100}]}:{status:"stopped"});runtime.listen.mockResolvedValue(()=>{});
 host=document.createElement("div");document.body.append(host);root=createRoot(host);
});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.useRealTimers();vi.unstubAllGlobals();});
async function click(text:string){const button=[...host.querySelectorAll('button')].find(b=>b.textContent?.includes(text));expect(button,`missing button: ${text}`).toBeTruthy();await act(async()=>button!.click());}
it("核心请求仍在等待时即可访问本机缓存，连接完成不关闭离线界面",async()=>{
 const status=deferred<{initialized:boolean}>();runtime.request.mockImplementation((path:string)=>path==="/status"?status.promise:Promise.reject(new ApiError(401,"native_session","请先登录核心")));
 await act(async()=>root.render(<App/>));await click("播放本机离线音乐");expect(host.textContent).toContain("离线验收曲");
 expect(host.querySelector('[role="dialog"]')?.parentElement?.hidden).toBe(false);
 await act(async()=>status.resolve({initialized:true}));expect(host.querySelector('[role="dialog"]')?.textContent).toContain("离线验收曲");expect(host.querySelector('[role="dialog"]')?.parentElement?.hidden).toBe(false);expect(runtime.request.mock.calls.every(([path])=>path==="/status"||path==="/auth/me")).toBe(true);
});
it("新原生窗口没有临时登录标记时核验仍在运行的原生会话",async()=>{
 runtime.request.mockImplementation((path:string)=>Promise.resolve(path==="/status"?{initialized:true}:path==="/auth/me"?{id:"user",username:"qa",role:"admin"}:path==="/preferences"?{}:{items:[]}));
 await act(async()=>root.render(<App/>));
 expect(runtime.request.mock.calls.some(([path])=>path==="/auth/me")).toBe(true);
 expect(host.querySelector('nav[aria-label="主导航"]')).toBeTruthy();expect(host.querySelector('input[placeholder="你的用户名"]')).toBeNull();
 expect(sessionStorage.getItem("rmusic.token")).toBeNull();
});
it("原生进程没有会话时停在登录界面，不重试认证或读取私人曲库",async()=>{
 runtime.request.mockImplementation((path:string)=>path==="/status"?Promise.resolve({initialized:true}):Promise.reject(new ApiError(401,"native_session","请先登录核心")));
 await act(async()=>root.render(<App/>));await act(async()=>vi.advanceTimersByTime(30000));
 expect(host.querySelector("h1")?.textContent).toBe("选择音乐核心");
 expect(runtime.request.mock.calls.map(([path])=>path)).toEqual(["/status","/auth/me"]);
});
it("核心离线时保留本地入口并自动重试，不请求云端曲库",async()=>{
 runtime.request.mockRejectedValue(new TypeError("Failed to fetch"));await act(async()=>root.render(<App/>));await click("播放本机离线音乐");
 await act(async()=>vi.advanceTimersByTime(1000));expect(runtime.request.mock.calls.filter(([path])=>path==="/status").length).toBeGreaterThan(1);expect(host.textContent).toContain("离线验收曲");
 expect(runtime.request.mock.calls.every(([path])=>path==="/status")).toBe(true);
});
it("恢复已有离线播放器时展示真实进度与音量，不重新打开文件",async()=>{
 runtime.request.mockReturnValue(new Promise(()=>{}));runtime.invoke.mockImplementation(async(command:string)=>command==="native_cache_status"?{entries:[{id:"cached",track_id:"offline-song",title:"离线验收曲",complete:true,format:"flac",sample_rate:96000,channels:2,total_bytes:100}]}:{offline:true,cache_id:"cached",track_id:"offline-song",playback_id:"restored",status:"playing",position:7,duration:60,volume:.3,muted:true,paused:false});
 await act(async()=>root.render(<App/>));expect(host.querySelector('[role="dialog"]')?.parentElement?.hidden).toBe(false);expect(host.querySelector<HTMLInputElement>('[aria-label="离线播放进度"]')?.value).toBe("7");expect(host.querySelector<HTMLInputElement>('[aria-label="离线音量"]')?.value).toBe("0.3");expect(runtime.invoke.mock.calls.some(([command])=>command==="native_offline_play")).toBe(false);
});
it("离线准备期间保留记忆音量，静音与音量分别发送，迟到轮询不覆盖新控制",async()=>{
 const stale=deferred<unknown>();let reads=0;runtime.request.mockReturnValue(new Promise(()=>{}));
 runtime.invoke.mockImplementation((command:string)=>{
  if(command==="native_cache_status")return Promise.resolve({entries:[{id:"cached",track_id:"offline-song",title:"离线验收曲",complete:true,format:"flac",sample_rate:96000,channels:2,total_bytes:100}]});
  if(command==="native_status")return ++reads===1?Promise.resolve({status:"stopped",volume:.2,muted:true}):stale.promise;
  return Promise.resolve();
 });
 await act(async()=>root.render(<App/>));await click("播放本机离线音乐");
 await act(async()=>host.querySelector<HTMLButtonElement>('.download-list button')!.click());
 expect(host.querySelector<HTMLInputElement>('[aria-label="离线音量"]')?.value).toBe("0.2");
 await act(async()=>vi.advanceTimersByTime(500));
 await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="取消离线静音"]')!.click());
 expect(runtime.invoke.mock.calls.filter(([command])=>command==="native_volume").at(-1)).toEqual(["native_volume",{muted:false}]);
 const id=runtime.invoke.mock.calls.find(([command])=>command==="native_offline_play")![1].playbackId;
 await act(async()=>stale.resolve({offline:true,cache_id:"cached",track_id:"offline-song",playback_id:id,status:"playing",position:0,duration:60,volume:.8,muted:true}));
 expect(host.querySelector('[aria-label="离线静音"]')).toBeTruthy();expect(host.querySelector<HTMLInputElement>('[aria-label="离线音量"]')?.value).toBe("0.2");
});
it("手动重连后旧连接迟到的401不能清除当前登录或离线界面",async()=>{
 const old=deferred<unknown>();let statuses=0;sessionStorage.setItem("rmusic.token","native-session");
 runtime.request.mockImplementation((path:string)=>path==="/status"?(++statuses===1?old.promise:Promise.resolve({initialized:true})):Promise.resolve(path==="/auth/me"?{id:"user",username:"qa",role:"admin"}:path==="/preferences"?{}:{items:[]}));
 await act(async()=>root.render(<App/>));await click("播放本机离线音乐");await click("重试连接");
 await act(async()=>old.reject(new ApiError(401,"old_session","旧地址的错误")));expect(sessionStorage.getItem("rmusic.token")).toBe("native-session");expect(host.querySelector('[role="dialog"]')?.parentElement?.hidden).toBe(false);expect(host.textContent).toContain("离线验收曲");
});
it("原生音区轮询不叠加未完成请求，超时取消后再恢复轮询",async()=>{
 sessionStorage.setItem("rmusic.token","native-session");const signals:AbortSignal[]=[];
 runtime.request.mockImplementation((path:string,options?:RequestInit)=>{
  if(path==="/zones")return new Promise((_,reject)=>{const signal=options?.signal as AbortSignal;signals.push(signal);signal?.addEventListener('abort',()=>reject(new DOMException("已取消","AbortError")),{once:true});});
  return Promise.resolve(path==="/status"?{initialized:true}:path==="/auth/me"?{id:"user",username:"qa",role:"admin"}:path==="/preferences"?{}:{items:[]});
 });
 await act(async()=>root.render(<App/>));await act(async()=>vi.advanceTimersByTime(8000));expect(signals).toHaveLength(1);
 await act(async()=>vi.advanceTimersByTime(2000));expect(signals[0]?.aborted).toBe(true);
 await act(async()=>vi.advanceTimersByTime(2000));expect(signals).toHaveLength(2);
});
