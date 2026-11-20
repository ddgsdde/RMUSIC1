// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect,it,vi} from "vitest";
import {RMusicClient} from "@r-music/sdk";
import {FileManager} from "./FileManager";
it("全选筛选结果后取消一首，跨页范围和确认数量一致",async()=>{
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);const api=new RMusicClient("http://core","synthetic-session");
 const play=vi.fn(async()=>{}),enqueue=vi.fn(async()=>{});const calls:{path:string;body?:string}[]=[];
 vi.spyOn(api,"request").mockImplementation(async(path,options)=>{calls.push({path,body:options?.body as string});if(path.startsWith("/library/files?"))return {items:[{id:"one",title:"一",artist:"艺人",album:"专辑",path:"/music/one.flac",size:24,available:true},{id:"two",title:"二",artist:"艺人",album:"专辑",path:"/music/two.flac",size:24,available:true}],total:200}as never;if(path==="/jobs")return {items:[{id:"synthetic-job",status:"completed",phase:"完成"}]}as never;return {job_id:"synthetic-job"}as never;});
 const host=document.createElement("div");document.body.append(host);const root=createRoot(host);const button=(name:string)=>[...document.querySelectorAll("button")].find(v=>v.textContent===name)!;
 try{await act(async()=>root.render(<FileManager api={api} play={play} enqueue={enqueue} storage="local" album="" artist="" folder="" query="" revision={0} onError={vi.fn()} showDetails={vi.fn()}/>));
  await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="播放 一"]')!.click());expect(play).toHaveBeenCalledWith(expect.objectContaining({id:"one"}));await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="添加 二 到播放队列"]')!.click());expect(enqueue).toHaveBeenCalledWith(expect.objectContaining({id:"two"}));
  await act(async()=>button("全选筛选结果").click());
  await act(async()=>host.querySelector<HTMLInputElement>('[aria-label="选择 一"]')!.click());
  expect(host.textContent).toContain("已选 199");expect(host.querySelector<HTMLInputElement>('[aria-label="选择 二"]')!.checked).toBe(true);
  await act(async()=>button("移入回收站").click());expect(document.querySelector('[role="dialog"]')!.textContent).toContain("199 个");
  await act(async()=>button("确认移入回收站").click());
  const action=JSON.parse(calls.find(v=>v.path==="/library/files/actions")!.body!);expect(action.expected_count).toBe(199);expect(action.exclude_ids).toEqual(["one"]);expect(action.filter.storage).toBe("local");expect(action.ids).toBeUndefined();
 }finally{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();}
});
