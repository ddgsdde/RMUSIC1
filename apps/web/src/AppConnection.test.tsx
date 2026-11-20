// @vitest-environment jsdom
import {act} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {ApiError,RMusicClient} from "@r-music/sdk";
import {App} from "./App";
vi.mock("./native",()=>({isNative:false,isAndroidClient:false,restoreWindowAppearance:()=>{},nativeProfiles:async()=>[]}));
const ownership=vi.hoisted(()=>({owner:true}));
vi.mock("./useOutputOwner",()=>({useOutputOwner:()=>ownership.owner}));
vi.mock("./useAudio",()=>({useAudio:()=>({position:0,playing:false,local:false,adaptive:0,nativeState:{},sourceInfo:null,sourceError:"",contextSampleRate:0,analyserRef:{current:null},interrupt:async()=>{},unlock:async()=>{},seek:()=>{},finishChange:()=>{}})}));
vi.mock("./browserCache",()=>({browserCache:{bind:vi.fn(),register:async()=>false,list:async()=>[],protect:()=>{}},formatBytes:(n:number)=>`${n} B`}));
class Socket {
 static instances:Socket[]=[];
 onopen:(()=>void)|null=null;onclose:(()=>void)|null=null;onmessage:((event:{data:string})=>void)|null=null;
 close=vi.fn();constructor(public url:string){Socket.instances.push(this);}
}
let root:Root,host:HTMLDivElement,request:ReturnType<typeof vi.spyOn>;
function deferred<T>(){let resolve!:(value:T)=>void,reject!:(error:Error)=>void;const promise=new Promise<T>((yes,no)=>{resolve=yes;reject=no;});return{promise,resolve,reject};}
beforeEach(()=>{
 ownership.owner=true;
 vi.useFakeTimers();localStorage.clear();sessionStorage.clear();sessionStorage.setItem("rmusic.token","qa-session");Socket.instances=[];
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);vi.stubGlobal("WebSocket",Socket);vi.stubGlobal("matchMedia",()=>({matches:false,addEventListener:()=>{},removeEventListener:()=>{}}));
 request=vi.spyOn(RMusicClient.prototype,"request").mockImplementation(async(path:string)=>path==="/status"?{initialized:true}:path==="/auth/me"?{id:"user",username:"qa",role:"admin"}:path==="/events/ticket"?{ticket:"qa-ticket"}:path==="/preferences"?{}:{items:[]});
 host=document.createElement("div");document.body.append(host);root=createRoot(host);
});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.restoreAllMocks();vi.useRealTimers();vi.unstubAllGlobals();});
async function click(label:string){const button=host.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)||[...host.querySelectorAll('button')].find(button=>button.textContent===label);expect(button,`missing ${label}`).toBeTruthy();await act(async()=>button!.click());}
async function loginThenLogout(){await act(async()=>root.render(<App/>));expect(host.textContent).toContain("核心已连接");await click("打开账号与导航");await click("退出登录");await click("播放本机离线音乐");expect(host.querySelector('.offline-connection')?.textContent).toContain("核心已连接");}
it("控制标签页读取音区而不抢占持有输出锁的标签页注册",async()=>{
 ownership.owner=false;await act(async()=>root.render(<App/>));await act(async()=>vi.advanceTimersByTime(31000));
 expect(request.mock.calls.some(([path]:unknown[])=>path==="/zones")).toBe(true);expect(request.mock.calls.some(([path]:unknown[])=>path==="/endpoints")).toBe(true);
 expect(request.mock.calls.some(([path]:unknown[])=>path==="/endpoints/browser")).toBe(false);
});
it("关闭输出标签页立即用 keepalive 注销自己的注册，控制标签页不注销其他输出",async()=>{
 localStorage.setItem("rmusic.endpoint-id","browser-qa");
 await act(async()=>root.render(<App/>));
 await act(async()=>window.dispatchEvent(new Event("pagehide")));
 const closed=request.mock.calls.find(([path]:unknown[])=>String(path).startsWith("/endpoints/browser/browser-qa/disconnect"));
 expect(closed).toBeTruthy();expect(closed?.[1]).toMatchObject({method:"POST",keepalive:true});
 expect(String(closed?.[0])).toContain("registration_id=");
 const count=request.mock.calls.filter(([path]:unknown[])=>path==="/endpoints/browser").length;
 await act(async()=>vi.advanceTimersByTime(31000));
 expect(request.mock.calls.filter(([path]:unknown[])=>path==="/endpoints/browser")).toHaveLength(count);
});
it("浏览器每次成功注册都通知等待恢复的本机输出，包括 Core 重启后的续约",async()=>{
 const registered=vi.fn(),original=request.getMockImplementation()!;localStorage.setItem("rmusic.endpoint-id","browser-qa");
 request.mockImplementation((path:string,options?:RequestInit)=>path==="/endpoints/browser"?Promise.resolve({id:"browser-qa",capabilities:{}}):original(path,options));
 window.addEventListener("rmusic-browser-registered",registered);
 try{await act(async()=>root.render(<App/>));await act(async()=>vi.advanceTimersByTime(30000));expect(registered).toHaveBeenCalledTimes(2);expect(registered.mock.calls.every(([event]:CustomEvent[])=>event.detail==="browser-qa")).toBe(true);}
 finally{window.removeEventListener("rmusic-browser-registered",registered);}
});
it("安全上下文在核心支持时协商浏览器持久恢复及等待恢复期间的控制",async()=>{
 vi.stubGlobal("isSecureContext",true);vi.stubGlobal("AudioContext",class{});vi.stubGlobal("AudioWorkletNode",class{});
 const original=request.getMockImplementation()!;request.mockImplementation((path:string,options?:RequestInit)=>path==="/status"?Promise.resolve({initialized:true,core_id:"core",capabilities:{browser_pcm:true,measured_listens:true,continuous_recovery:true,recovery_controls:true}}):original(path,options));
 await act(async()=>root.render(<App/>));
 const last=request.mock.calls.filter(([path]:unknown[])=>path==="/endpoints/browser").at(-1) as [string,RequestInit];
 expect(JSON.parse(last[1].body as string)).toMatchObject({continuous_pcm:true,continuous_recovery:true,recovery_controls:true});
});
it("退出会话后旧 WebSocket 的关闭和打开回调不能更改当前连接状态",async()=>{
 await loginThenLogout();const old=Socket.instances[0];expect(old).toBeTruthy();
 await act(async()=>old.onclose?.());expect(host.querySelector('.offline-connection')?.textContent).toContain("核心已连接");
 await act(async()=>old.onopen?.());await act(async()=>vi.advanceTimersByTime(31000));expect(Socket.instances).toHaveLength(1);
});
it("离开会话后迟到的事件票据失败不污染当前状态或发起重试",async()=>{
 const ticket=deferred<{ticket:string}>(),original=request.getMockImplementation()!;
 request.mockImplementation((path:string,options?:RequestInit)=>path==="/events/ticket"?ticket.promise:original(path,options));await loginThenLogout();
 await act(async()=>ticket.reject(new TypeError("Failed to fetch")));expect(host.querySelector('.offline-connection')?.textContent).toContain("核心已连接");
 await act(async()=>vi.advanceTimersByTime(31000));expect(request.mock.calls.filter(([path]:unknown[])=>path==="/events/ticket")).toHaveLength(1);
});
it("事件票据401结束当前会话，不无限重试已失效凭证",async()=>{
 const original=request.getMockImplementation()!;request.mockImplementation((path:string,options?:RequestInit)=>path==="/events/ticket"?Promise.reject(new ApiError(401,"unauthorized","会话已失效")):original(path,options));
 await act(async()=>root.render(<App/>));expect(host.textContent).toContain("欢迎回来");expect(sessionStorage.getItem("rmusic.token")).toBeNull();
 await act(async()=>vi.advanceTimersByTime(31000));expect(request.mock.calls.filter(([path]:unknown[])=>path==="/events/ticket")).toHaveLength(1);
});
it("事件票据请求超时会取消等待，再按退避重连",async()=>{
 const original=request.getMockImplementation()!,signals:AbortSignal[]=[];
 request.mockImplementation((path:string,options?:RequestInit)=>path!=="/events/ticket"?original(path,options):new Promise((_,reject)=>{
  const signal=options?.signal as AbortSignal;signals.push(signal);signal?.addEventListener('abort',()=>reject(new DOMException("已取消","AbortError")),{once:true});
 }));
 await act(async()=>root.render(<App/>));await act(async()=>vi.advanceTimersByTime(10000));expect(signals[0]?.aborted).toBe(true);
 await act(async()=>vi.advanceTimersByTime(1000));expect(signals).toHaveLength(2);expect(Socket.instances).toHaveLength(0);
});
it("旧连接关闭后迟到的事件不能替换已恢复连接的音区",async()=>{
 await act(async()=>root.render(<App/>));const old=Socket.instances[0];await act(async()=>old.onclose?.());await act(async()=>vi.advanceTimersByTime(1000));expect(Socket.instances).toHaveLength(2);
 await act(async()=>Socket.instances[1].onopen?.());expect(host.textContent).toContain("核心已连接");
 await act(async()=>old.onclose?.());expect(host.textContent).toContain("核心已连接");
 await act(async()=>old.onmessage?.({data:JSON.stringify({type:"zones",data:{items:[{id:"old",name:"旧连接音区",queue:[],status:"stopped",dsp:{enabled:false},transitions:{}}]}})}));
 expect(host.textContent).not.toContain("旧连接音区");
});

it("有已保存会话时先进入主界面，核心不可用时保留会话并允许重试",async()=>{
 const status=deferred<{initialized:boolean}>();
 const original=request.getMockImplementation()!;
 let useDeferred=true;
 request.mockImplementation((path:string,options?:RequestInit)=>path==="/status"&&useDeferred?status.promise:original(path,options));
 await act(async()=>root.render(<App/>));
 expect(host.querySelector(".studio-app")).toBeTruthy();
 expect(host.textContent).not.toContain("欢迎回来");
 await act(async()=>status.reject(new TypeError("Failed to fetch")));
 expect(host.textContent).toContain("无法连接 Core");
 expect(sessionStorage.getItem("rmusic.token")).toBe("qa-session");
 useDeferred=false;
 await click("重试连接");
 expect(host.textContent).toContain("核心已连接");
 expect(host.textContent).not.toContain("无法连接 Core");
});
it("核心不可用时退出不等待远端响应，也不会恢复已退出的会话",async()=>{
 const logout=deferred<unknown>();
 const original=request.getMockImplementation()!;
 request.mockImplementation((path:string,options?:RequestInit)=>path==="/status"?Promise.reject(new TypeError("Failed to fetch")):path==="/auth/logout"?logout.promise:original(path,options));
 await act(async()=>root.render(<App/>));
 expect(host.textContent).toContain("无法连接 Core");
 await click("退出登录");
 expect(sessionStorage.getItem("rmusic.token")).toBeNull();
 expect(host.textContent).toContain("欢迎回来");
 await act(async()=>logout.resolve({}));
 expect(host.textContent).toContain("欢迎回来");
});
