// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect,it,vi} from "vitest";
import {RMusicClient} from "@r-music/sdk";
import {TrackComments} from "./TrackComments";
import {cachedResource,loadResource} from "./resourceCache";
it("乐评分页、切换最新、强制刷新，失败时保留文字且不解释为 HTML",async()=>{
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);Object.defineProperty(document,"hidden",{value:false,configurable:true});const api=new RMusicClient("http://comments-core","session");const paths:string[]=[];let fail=false;
 vi.spyOn(api,"get").mockImplementation(async(path)=>{paths.push(path);if(fail)throw new Error("上游限流");return {supported:true,items:[{id:"1",author:"听友",text:'<img src=x onerror="bad()">文字',published_at:1,likes:2,reply_count:2,replies:[{id:"child",author:"子回复作者",text:"子回复正文",published_at:1}]}],has_more:true,next_cursor:"987",total:42,fetched_at:Math.floor(Date.now()/1000)} as never;});
 const host=document.createElement("div");document.body.append(host);const root=createRoot(host);const button=(label:string)=>host.querySelector<HTMLButtonElement>(`[aria-label="${label}"]`)!;
 try{
  await act(async()=>root.render(<TrackComments api={api} id="qqmusic:Example123"/>));expect(host.querySelector("details.comment-replies")).not.toBeNull();await act(async()=>host.querySelector<HTMLElement>("details.comment-replies summary")!.click());expect(host.querySelector<HTMLDetailsElement>("details.comment-replies")!.open).toBe(true);expect(host.textContent).toContain("子回复正文");expect(host.textContent).toContain("未完整返回其余内容");expect(host.querySelector("img")).toBeNull();expect(host.textContent).toContain("文字");
  await act(async()=>button("评论下一页").click());expect(paths.at(-1)).toContain("page=2");expect(paths.at(-1)).toContain("cursor=987");
  await act(async()=>[...host.querySelectorAll("button")].find(v=>v.textContent==="最新")!.click());expect(paths.at(-1)).toContain("kind=new&page=1");
  await act(async()=>button("获取最新评论").click());expect(paths.at(-1)).toContain("refresh=true");
  fail=true;await act(async()=>button("获取最新评论").click());expect(host.textContent).toContain("已保留上次内容");expect(host.textContent).toContain("文字");expect(host.querySelector("textarea")).toBeNull();
 }finally{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();}
});
it("共享缓存合并请求、强刷失败保留值并隔离账号令牌",async()=>{
 const api=new RMusicClient("http://cache-core","first");const load=vi.fn(async()=>42);
 expect(await Promise.all([loadResource(api,"a",load,300000),loadResource(api,"a",load,300000)])).toEqual([42,42]);expect(load).toHaveBeenCalledTimes(1);
 await expect(loadResource(api,"a",async()=>{throw new Error("offline");},300000,true)).rejects.toThrow("offline");expect(cachedResource(api,"a")).toBe(42);
 api.token="second";expect(cachedResource(api,"a")).toBeUndefined();
});
