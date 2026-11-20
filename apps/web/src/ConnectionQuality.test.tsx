// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect,it,vi} from "vitest";
import {RMusicClient} from "@r-music/sdk";
import {ConnectionQuality} from "./ConnectionQuality";

vi.mock("./native", () => ({isNative:false}));

it("打开连接页显示延迟且不测速，点按后只下载一次",async()=>{
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
 const requests:{url:string;method:string}[]=[];
 vi.stubGlobal("fetch",async(url:string,options:RequestInit)=>{
  requests.push({url,method:options.method||"GET"});
  return url.endsWith("/payload")?new Response(new Uint8Array(1024*1024)):Response.json({ingress:"reverse_proxy"});
 });
 const host=document.createElement("div");const root=createRoot(host);
 try{
  const api=new RMusicClient("https://core.example.com","fixture");
  await act(async()=>root.render(<ConnectionQuality api={api}/>));
  expect(host.textContent).toContain("HTTPS 反代");expect(host.textContent).toContain("ms");
  expect(requests).toHaveLength(3);expect(requests.every(v=>!v.url.endsWith("/payload"))).toBe(true);
  await act(async()=>host.querySelector("button")!.click());
  expect(requests.filter(v=>v.url.endsWith("/payload"))).toHaveLength(1);
  expect(requests.every(v=>v.method==="GET")).toBe(true);expect(host.textContent).toContain("Mbps");
 }finally{await act(async()=>root.unmount());vi.unstubAllGlobals();}
});
