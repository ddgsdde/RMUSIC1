// @vitest-environment jsdom
import {act} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {Auth} from "./Auth";
import {RMusicClient} from "@r-music/sdk";
const native=vi.hoisted(()=>({enabled:true,probe:vi.fn()}));
vi.mock("./native",()=>({get isNative(){return native.enabled;},saveNativeProfile:async()=>({}),NativeClient:class{request(...args:unknown[]){return native.probe(...args);}},nativeProfiles:async()=>[],invokeNative:async(command:string)=>command==="native_discovery"?{available:true,items:[{id:"nas",kind:"core",name:"NAS",addresses:["http://192.168.50.10:7800"]}]}:{}}));
let host:HTMLDivElement,root:Root;
beforeEach(()=>{vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);native.enabled=true;native.probe.mockReset().mockResolvedValue({core_id:"nas-core",name:"NAS",initialized:true});host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();});
async function render(){const connect=vi.fn();await act(async()=>root.render(<Auth api={new RMusicClient("http://old-core:18854")} initialized profiles={[]} onAuth={vi.fn()} onConnect={connect} onOffline={vi.fn()}/>));return connect;}
it("原生登录先选择核心，公开状态验证成功后才显示账号密码",async()=>{const connect=await render();expect(host.querySelector('input[autocomplete="username"]')).toBeNull();expect(host.textContent).toContain("选择音乐核心");expect(host.querySelector(".core-guide-manual")?.hasAttribute("open")).toBe(true);expect(host.querySelector<HTMLInputElement>('.core-guide-addresses input[type="url"]')?.value).toBe("");await act(async()=>host.querySelector<HTMLButtonElement>(".core-guide-item")!.click());expect(native.probe).toHaveBeenCalledWith("/status",expect.objectContaining({signal:expect.any(AbortSignal)}));expect(connect).toHaveBeenCalledWith("http://192.168.50.10:7800","",[]);expect(host.querySelector('input[autocomplete="username"]')).not.toBeNull();expect(host.textContent).toContain("NAS");expect(host.querySelector<HTMLButtonElement>(".auth-account-form>.primary")!.disabled).toBe(true);await act(async()=>host.querySelector<HTMLButtonElement>(".native-auth-back")!.click());expect(host.querySelector('input[autocomplete="username"]')).toBeNull();});
it("非核心地址拒绝进入账号步骤，保留核心选择",async()=>{native.probe.mockResolvedValue({version:"other-service"});const connect=await render();await act(async()=>host.querySelector<HTMLButtonElement>(".core-guide-item")!.click());expect(connect).not.toHaveBeenCalled();expect(host.querySelector('[role="alert"]')?.textContent).toContain("没有返回有效");expect(host.querySelector('input[autocomplete="username"]')).toBeNull();});
it("网页登录保持固定核心，不展示核心切换",async()=>{native.enabled=false;await render();expect(host.querySelector(".auth-core-connection")).toBeNull();expect(host.querySelector('input[autocomplete="username"]')).not.toBeNull();});
