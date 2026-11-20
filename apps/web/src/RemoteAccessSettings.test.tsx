// @vitest-environment jsdom
// @vitest-environment-options {"url":"https://core.example.com"}
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect,it,vi} from "vitest";
import type {RMusicClient,User} from "@r-music/sdk";
import {RemoteAccessSettings,newCoreIngress,isSecureRemoteManagement} from "./RemoteAccessSettings";

it("反代模式隐藏 Core 证书任务，端口字段均保留自定义能力，离页取消读取",async()=>{
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);const ingress=newCoreIngress();ingress.mode="reverse_proxy";ingress.public_url="https://core.example.com:4443/music";let signal:AbortSignal|undefined;
 const request=vi.fn(async(_path:string,options?:RequestInit)=>{signal=options?.signal as AbortSignal;return{configuration:{revision:2,core:ingress,active:null,candidate:null,previous:null},core_public_key:"public",status:{state:"disabled"},secrets:{}};});const api={request} as unknown as RMusicClient;const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 try{await act(async()=>root.render(<RemoteAccessSettings api={api} user={{id:"admin",role:"admin",username:"admin"} as User} notify={()=>{}} onError={()=>{}}/>));expect(host.textContent).toContain("外部 HTTPS 地址");expect(host.querySelector('input[value="https://core.example.com:4443/music"]')).not.toBeNull();expect(host.textContent).not.toContain("联系邮箱");expect(host.querySelectorAll('input[type="number"]').length).toBe(4);await act(async()=>root.render(null));expect(signal?.aborted).toBe(true);}finally{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();}
});


it("明文网页与跨域入口不能提交管理凭据，原生仍由 Core 验证加密通道",()=>{
 expect(isSecureRemoteManagement("https://core.example.com/music","https://core.example.com/",false)).toBe(true);
 expect(isSecureRemoteManagement("https://core.example.com","http://core.example.com/",false)).toBe(false);
 expect(isSecureRemoteManagement("http://core.example.com","https://core.example.com/",false)).toBe(false);
 expect(isSecureRemoteManagement("https://another.example.com","https://core.example.com/",false)).toBe(false);
 expect(isSecureRemoteManagement("http://nas.local","http://tauri.localhost/",true)).toBe(true);
});

it("HTTP 管理页面只显示状态和安全指引，不显示秘密表单",async()=>{
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);const ingress=newCoreIngress();const request=vi.fn(async()=>({configuration:{revision:0,core:ingress,active:null,candidate:null,previous:null},status:{state:"disabled"},secrets:{}}));
 const api={baseUrl:"http://core.example.com",request} as unknown as RMusicClient;const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 try{await act(async()=>root.render(<RemoteAccessSettings api={api} user={{id:"admin",role:"admin"} as User} notify={()=>{}} onError={()=>{}}/>));expect(host.textContent).toContain("此连接尚未通过安全管理验证");expect(host.querySelector("form")).toBeNull();expect(host.querySelector('input[type="password"]')).toBeNull();expect(request).toHaveBeenCalledTimes(1);}finally{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();}
});
