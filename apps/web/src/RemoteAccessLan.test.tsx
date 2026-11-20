// @vitest-environment jsdom
// @vitest-environment-options {"url":"http://192.168.50.10:7800"}
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect,it,vi} from "vitest";
import type {RMusicClient,User} from "@r-music/sdk";
import {RemoteAccessSettings,newCoreIngress} from "./RemoteAccessSettings";

vi.mock("./native",()=>({isNative:false}));

it("内网页面使用 Core 加密通道时默认允许保存",async()=>{
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
 const configuration={revision:2,core:{...newCoreIngress(),mode:"local_relay"}};
 const calls:RequestInit[]=[];
 const request=async(_path:string,init?:RequestInit)=>{
  if(init?.method==="PUT")calls.push(init);
  return {core_settings_transport:"secure",configuration,status:{state:"disabled"},secrets:{}};
 };
 const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 const button=(name:string)=>Array.from(host.querySelectorAll("button")).find(v=>v.textContent===name)!;
 try{
  await act(async()=>root.render(<RemoteAccessSettings api={{request,baseUrl:"http://192.168.50.10:7800"} as unknown as RMusicClient} user={{id:"admin",role:"admin"} as User} notify={()=>{}} onError={e=>{throw e;}}/>));
  expect(button("保存连接配置").disabled).toBe(false);
  expect(host.querySelector('input[type="checkbox"]')).toBeNull();
  const addresses = host.querySelector<HTMLTextAreaElement>("textarea")!;
  await act(async()=>{
   Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,"value")!.set!.call(addresses,"https://second.example/music\nhttps://first.example/music");
   addresses.dispatchEvent(new Event("input",{bubbles:true}));
  });
  await act(async()=>button("保存连接配置").click());
  const input=document.querySelector<HTMLInputElement>('input[autocomplete="current-password"]')!;
  await act(async()=>{
   Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(input,"fixture-password");
   input.dispatchEvent(new Event("input",{bubbles:true}));
  });
  await act(async()=>input.closest("form")!.requestSubmit());
  expect(calls).toHaveLength(1);
  const saved=JSON.parse(String(calls[0].body));
  expect(saved).toMatchObject({expected_revision:2,configuration:{direct_urls:["https://second.example/music","https://first.example/music"]},credentials:{current_password:"fixture-password"}});
  expect(saved.allow_lan_http).toBeUndefined();
  expect(saved.allow_tailscale_http).toBeUndefined();
 }finally{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();}
});
