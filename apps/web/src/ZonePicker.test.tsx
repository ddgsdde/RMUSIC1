// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect,it,vi} from "vitest";
import {normalizeDsp,RMusicClient,type Endpoint,type Zone} from "@r-music/sdk";
import {ZonePicker} from "./ZonePicker";
import {SignalPanel} from "./SignalPanel";
import {playlistPage,rememberPlaylistPage} from "./playlistPageCache";
it("底栏音区入口可选择在线输出，离线输出不触发播放，并可进入分组编排",async()=>{
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);const api=new RMusicClient("http://core","one"),select=vi.fn(),manage=vi.fn();
 const zones=[{id:"a",name:"书房",endpoint_id:"one"},{id:"b",name:"客厅",endpoint_id:"two"}] as Zone[];
 const endpoints=[{id:"one",name:"Windows",online:true},{id:"two",name:"Android",online:false}] as Endpoint[];
 const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 try{await act(async()=>root.render(<ZonePicker labels={{one:"书房 PC · USB DAC"}} api={api} zones={zones} endpoints={endpoints} selected="a" definitions={false} onSelect={select} onGroup={vi.fn()} onManage={manage} close={vi.fn()}/>));
 const rows=host.querySelectorAll<HTMLButtonElement>(".zone-picker-list>button");expect(rows[0].textContent).toContain("书房 PC · USB DAC");expect(rows[0].textContent).toContain("书房 · 在线");expect(rows[1].disabled).toBe(true);await act(async()=>rows[0].click());expect(select).toHaveBeenCalledWith("a");
 await act(async()=>host.querySelector<HTMLButtonElement>(".zone-picker-groups>button")!.click());expect(manage).toHaveBeenCalledOnce();
 }finally{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();}
});
it("旁路 DSP 不占据链路节点，已报告原生 PCM 不再误标待确认",async()=>{
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 const zone={id:"a",volume:1,dsp:normalizeDsp({enabled:false}),track:{id:"song",format:"flac",sample_rate:96000,bit_depth:24}} as Zone;
 try{await act(async()=>root.render(<SignalPanel zone={zone} info={null} error="" adaptive={0} nativeState={{track_id:"song",signal_path:{output:"WASAPI",sample_rate:96000,channels:2,exclusive:true,bit_depth:24,sample_type:"integer",bit_perfect_eligible:true,processing:[]}}} localOutput contextRate={0}/>));
 const stages=[...host.querySelectorAll(".audio-stages>li")];expect(stages.some(stage=>stage.querySelector("strong")?.textContent==="DSP")).toBe(false);
 const decode=stages.find(stage=>stage.querySelector("strong")?.textContent==="解码");expect(decode?.textContent).toContain("已报告");expect(decode?.textContent).not.toContain("待确认");
 }finally{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();}
});
it("歌单页面缓存按客户端会话隔离",()=>{const api=new RMusicClient("http://core","one");rememberPlaylistPage(api,"list",{items:[1]});expect(playlistPage(api,"list")?.value).toEqual({items:[1]});api.token="two";expect(playlistPage(api,"list")).toBeUndefined();});

it("手机设备选择器将当前在线设备放在首位并折叠离线设备",async()=>{
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
 const api=new RMusicClient("http://core","one"),host=document.createElement("div"),root=createRoot(host);
 const zones=[{id:"a",name:"离线 Mac",endpoint_id:"mac"},{id:"b",name:"客厅",endpoint_id:"living"},{id:"c",name:"本机",endpoint_id:"phone"}] as Zone[];
 const endpoints=[{id:"mac",name:"Mac",online:false},{id:"living",name:"客厅",online:true},{id:"phone",name:"Android",online:true}] as Endpoint[];
 try{
  await act(async()=>root.render(<ZonePicker api={api} compact zones={zones} endpoints={endpoints} selected="c" definitions={false} onSelect={vi.fn()} onGroup={vi.fn()} onManage={vi.fn()} close={vi.fn()}/>));
  expect(host.querySelector('.modal>.zone-picker-list>button')?.getAttribute("aria-pressed")).toBe("true");
  expect(host.querySelector(".zone-picker-offline")?.hasAttribute("open")).toBe(false);
  expect(host.querySelector(".zone-picker-offline button")?.hasAttribute("disabled")).toBe(true);
 }finally{await act(async()=>root.unmount());vi.unstubAllGlobals();}
});
