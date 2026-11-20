// @vitest-environment jsdom
import {act} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {RMusicClient,type User} from "@r-music/sdk";
import {AccountSecurity} from "./AccountSecurity";
import {Auth} from "./Auth";
const runtime=vi.hoisted(()=>({isNative:false}));
vi.mock("./native",()=>({get isNative(){return runtime.isNative;}}));
vi.mock("./CoreConnections",()=>({CoreConnections:()=> <div>核心地址编辑器</div>}));
let root:Root,host:HTMLDivElement;
beforeEach(()=>{runtime.isNative=false;vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.restoreAllMocks();vi.unstubAllGlobals();});
async function input(label:string,value:string){const el=[...document.querySelectorAll("label")].find(el=>el.textContent?.startsWith(label))?.querySelector("input");expect(el,label).toBeTruthy();await act(async()=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(el,value);el!.dispatchEvent(new Event("input",{bubbles:true}));});}
async function submit(button:string){const target=[...document.querySelectorAll("button")].find(el=>el.textContent===button);expect(target,button).toBeTruthy();await act(async()=>{target!.closest("form")!.dispatchEvent(new Event("submit",{bubbles:true,cancelable:true}));});}
it("网页登录页不提供核心切换，原生客户端保留",async()=>{
 const props={api:new RMusicClient("http://core"),initialized:true,onAuth:vi.fn(),profiles:[],onConnect:vi.fn(),onOffline:vi.fn()};
 await act(async()=>root.render(<Auth {...props}/>));expect(host.querySelector(".native-auth")).toBeNull();expect(host.textContent).not.toContain("选择音乐核心");
 runtime.isNative=true;await act(async()=>root.render(<Auth {...props}/>));expect(host.querySelector(".native-auth")).not.toBeNull();expect(host.textContent).toContain("选择音乐核心");
});
it("管理员修改账号与密码须确认新密码，并提交当前密码和已有第二因素",async()=>{
 const user:User={id:"a",username:"admin",role:"admin",totp_enabled:true},next={...user,username:"renamed"};const api=new RMusicClient("http://core"),notify=vi.fn(),changed=vi.fn();vi.spyOn(api,"get").mockResolvedValue(user);const patch=vi.spyOn(api,"patch").mockResolvedValue({user:next});
 await act(async()=>root.render(<AccountSecurity api={api} user={user} notify={notify} onError={vi.fn()} onUserChange={changed}/>));
 await input("登录账号","renamed");await input("当前密码","current-password");await input("新密码","new-password-123");await input("确认新密码","different-password");await input("两步验证码或恢复码","123456");await submit("保存账号与密码");expect(patch).not.toHaveBeenCalled();
 await input("确认新密码","new-password-123");await submit("保存账号与密码");expect(patch).toHaveBeenCalledWith("/auth/account",{username:"renamed",current_password:"current-password",new_password:"new-password-123",totp:"123456"});expect(changed).toHaveBeenCalledWith(next);expect(host.querySelector<HTMLInputElement>('input[autocomplete="current-password"]')!.value).toBe("");
});
it("家庭成员可自行设置和关闭两步验证，恢复码只在本次设置后显示",async()=>{
 const user:User={id:"m",username:"family",role:"member"},api=new RMusicClient("http://core"),changed=vi.fn();vi.spyOn(api,"get").mockResolvedValue(user);const post=vi.spyOn(api,"post").mockImplementation(async path=>path==="/auth/totp/start"?{secret:"synthetic-secret",uri:"otpauth://synthetic"}:path==="/auth/totp/confirm"?{recovery_codes:["synthetic-recovery"]}:{totp_enabled:false});
 await act(async()=>root.render(<AccountSecurity api={api} user={user} notify={vi.fn()} onError={vi.fn()} onUserChange={changed}/>));expect(host.textContent).not.toContain("登录账号");
 await input("验证当前密码","current-password");await submit("设置两步验证");expect(post).toHaveBeenCalledWith("/auth/totp/start",{current_password:"current-password"});await input("动态验证码","123456");await submit("验证并启用");expect(host.textContent).toContain("synthetic-recovery");expect(changed).toHaveBeenLastCalledWith({...user,totp_enabled:true});
 await act(async()=>[...document.querySelectorAll("button")].find(el=>el.textContent==="已保存恢复码")!.click());expect(host.textContent).not.toContain("synthetic-recovery");await input("验证当前密码","current-password");await input("当前动态码或恢复码","recovery-code");await submit("关闭两步验证");expect(post).toHaveBeenCalledWith("/auth/totp/disable",{current_password:"current-password",totp:"recovery-code"});expect(changed).toHaveBeenLastCalledWith({...user,totp_enabled:false});
});
