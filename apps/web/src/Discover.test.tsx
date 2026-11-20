// @vitest-environment jsdom
import {act} from "react";
import {createRoot, type Root} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {RMusicClient,type Recommendations,type Track} from "@r-music/sdk";
import {DiscoverView} from "./Discover";
import {quality} from "./components";

const track={id:"qqmusic:Track123",title:"原生歌曲",artist:"艺人",album:"专辑",duration:120,source:"qqmusic",format:"unknown",sample_rate:0,bit_depth:0,channels:0,bitrate:0,recording_id:"recording",available:true,provider_data:{quality_hint:{label:"Hi-Res",verified:false}}} satisfies Track;
const result:Recommendations={items:[track],source:"qqmusic",fallback:false,message:undefined};
let host:HTMLDivElement,root:Root;
const plugins=[{id:"qqmusic",name:"QQ",enabled:true,version:"test",account_context:"account-a",capabilities:["recommendations"]}];
const noop=vi.fn(async()=>{});
function props(api:RMusicClient){return {api,plugins:plugins as never,cacheScope:"core/user",play:noop,enqueue:noop,goSettings:noop,showDetails:noop,refreshZones:noop,onError:noop};}
beforeEach(()=>{vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);localStorage.clear();host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();});
it("自有发现独立于平台，进入平台页默认展开板块",async()=>{
 const api=new RMusicClient("https://core.test","session");
 const get=vi.spyOn(api,"get").mockImplementation(async path=>path.includes("settings")?{allowed:true,advanced:false,available:false}:path.startsWith("/providers")?{page:1,sections:[{id:"guess",title:"猜你喜欢",source:"qqmusic",items:[track]}]}:{...result,items:[{...track,title:"曲库推荐"}]});
 await act(async()=>root.render(<DiscoverView {...props(api)}/>));
 expect(host.textContent).toContain("曲库推荐");
 expect(host.textContent).not.toContain("猜你喜欢");
 expect(get.mock.calls.some(([path])=>path.startsWith("/providers"))).toBe(false);
 await act(async()=>[...host.querySelectorAll("button")].find(b=>b.textContent==="平台推荐")!.click());
 const detail=host.querySelector("details.radio-platform") as HTMLDetailsElement;
 expect(detail.open).toBe(true);
 expect(host.textContent).toContain("猜你喜欢");
 expect(host.textContent).toContain("原生歌曲");
 expect(host.textContent).not.toContain("曲库推荐");
});
it("进阶算法回退时照常展示推荐",async()=>{
 const api=new RMusicClient("https://core.test","session");
 vi.spyOn(api,"get").mockImplementation(async path=>path.includes("settings")?{allowed:true,advanced:true,available:true}:{...result,fallback:true});
 await act(async()=>root.render(<DiscoverView {...props(api)}/>));
 expect(host.textContent).not.toContain("本次使用本地算法");
 expect(host.querySelector('[aria-label="推荐算法"]')).toBeNull();
 expect(host.textContent).toContain("原生歌曲");
});
it("先显示首屏，云端检查未完成时不误报无结果",async()=>{
 const api=new RMusicClient("https://staged-recommendations.test","session");
 let finish!:(value:Recommendations)=>void;
 const pending=new Promise<Recommendations>(resolve=>{finish=resolve;});
 const get=vi.spyOn(api,"get").mockImplementation(async path=>{
  if(!path.includes("source=cloud"))return {...result,items:[{...track,title:"本地首屏",source:"local"}]};
  return path.includes("quick=true")?{...result,items:[],refreshing:true}:pending;
 });
 await act(async()=>root.render(<DiscoverView {...props(api)}/>));
 expect(host.textContent).toContain("本地首屏");
 await act(async()=>[...host.querySelectorAll("button")].find(b=>b.textContent==="云端")!.click());
 expect(host.textContent).not.toContain("本轮未找到");
 expect(host.textContent).not.toContain("本地首屏");
 expect(get.mock.calls.filter(([path])=>path.includes("source=cloud"))).toHaveLength(2);
 await act(async()=>finish(result));
 expect(host.textContent).toContain("原生歌曲");
});
it("安卓在同一份推荐中展示本地和云端，并去掉重复录音",async()=>{
 const api=new RMusicClient("https://mixed-recommendations.test","session");
 const local={...track,id:"local-id",source:"local",title:"本地歌曲"};
 const online={...track,id:"qqmusic:Unique",recording_id:"other",title:"云端歌曲"};
 const get=vi.spyOn(api,"get").mockResolvedValue({...result,items:[local],sections:[
  {id:"cloud",title:"云端探索",source:"cloud",items:[track,online]}
 ]});
 await act(async()=>root.render(<DiscoverView {...props(api)} coverActions unifiedSources/>));
 expect(host.querySelector(".discover-source")).toBeNull();
 expect(host.textContent).not.toContain("云端探索");
 expect([...host.querySelectorAll(".recommendation-rail .recommendation-title")].map(e=>e.textContent)).toEqual(["本地歌曲","云端歌曲"]);
 expect(get.mock.calls[0][0]).toBe("/recommendations/discover?limit=24&quick=true");
});
it("可用音质提示不覆盖已解析的真实格式",()=>{
 expect(quality(track)).toBe("Hi-Res · 平台声明");
 expect(quality({...track,format:"flac",sample_rate:44100,bit_depth:16})).toBe("FLAC 16 bit · 44.1 kHz");
 expect(quality({...track,provider_data:{}})).toBe("在线音源");
});

it("Android 仅长按封面打开推荐菜单，滑动取消且长按不触发播放",async()=>{
 const api=new RMusicClient("https://android-recommendations.test","session");
 vi.spyOn(api,"get").mockResolvedValue(result);
 const feedback=vi.spyOn(api,"put").mockResolvedValue({});
 const play=vi.fn(async()=>{});
 const {TrackContextMenuProvider}=await import("./TrackContextMenu");
 await act(async()=>root.render(
   <TrackContextMenuProvider api={api} location="discover" play={play} startRadio={noop} enqueue={noop} details={noop} onError={noop}>
    <DiscoverView {...props(api)} play={play} coverActions startRadio={noop}/>
   </TrackContextMenuProvider>
 ));
 expect(host.querySelector(".radio-card-actions")).toBeNull();
 const cover=host.querySelector<HTMLButtonElement>(".recommendation-rail .recommendation-cover")!;
 expect(cover.getAttribute("aria-haspopup")).toBe("menu");
 const touch=(type:string,x=20)=>{
  const event=new MouseEvent(type,{bubbles:true,cancelable:true,clientX:x,clientY:20});
  Object.defineProperties(event,{pointerType:{value:"touch"},isPrimary:{value:true}});
  cover.dispatchEvent(event);
 };
 vi.useFakeTimers();
 try{
  await act(async()=>{touch("pointerdown");touch("pointermove",55);vi.advanceTimersByTime(500);});
  expect(host.querySelector('[role="menu"]')).toBeNull();
  await act(async()=>{touch("pointerdown");vi.advanceTimersByTime(500);touch("pointerup");cover.click();});
  expect(play).not.toHaveBeenCalled();
  const menu=host.querySelector('[role="menu"]')!;
  expect(menu.textContent).toContain("启动电台");
  expect(menu.textContent).toContain("添加到队列末尾");
  await act(async()=>[...menu.querySelectorAll("button")].find(b=>b.textContent==="少推荐这首")!.click());
  expect(feedback).toHaveBeenCalledWith("/recommendations/feedback/qqmusic%3ATrack123",{less:true});
  await act(async()=>{touch("pointerdown");touch("pointerup");cover.click();});
  expect(play).toHaveBeenCalledTimes(1);
 }finally{vi.useRealTimers();}
});

it("Web 推荐将操作收进封面菜单，点击三点和右键都能提交反馈",async()=>{
 const api=new RMusicClient("https://web-recommendations.test","session");
 vi.spyOn(api,"get").mockResolvedValue(result);
 const feedback=vi.spyOn(api,"put").mockResolvedValue({});
 const {TrackContextMenuProvider}=await import("./TrackContextMenu");
 await act(async()=>root.render(
  <TrackContextMenuProvider api={api} location="discover" play={noop} startRadio={noop} enqueue={noop} details={noop} onError={noop}>
   <DiscoverView {...props(api)}/>
  </TrackContextMenuProvider>
 ));
 const card=host.querySelector<HTMLElement>(".recommendation-rail .recommendation-album")!;
 expect(card.querySelector("details,.recommendation-song-actions")).toBeNull();
 const button=card.querySelector<HTMLButtonElement>('.record-cover-menu')!;
 await act(async()=>button.click());
 const menu=()=>host.querySelector('[role="menu"]')!;
 const carousel=host.querySelector(".studio-deck-rail")!;
 await act(async()=>carousel.dispatchEvent(new Event("scroll",{bubbles:true})));
 expect(menu().textContent).toContain("启动电台");
 expect(menu().textContent).toContain("添加到队列末尾");
 await act(async()=>[...menu().querySelectorAll("button")].find(b=>b.textContent==="少推荐这首")!.click());
 expect(feedback).toHaveBeenCalledWith("/recommendations/feedback/qqmusic%3ATrack123",{less:true});
 await act(async()=>card.querySelector(".recommendation-cover")!.dispatchEvent(new MouseEvent("contextmenu",{bubbles:true,cancelable:true})));
 expect(menu().textContent).toContain("撤销减少推荐");
});

it("平台首屏提前加载，打开时直接使用同一账号的推荐快照",async()=>{
 const api=new RMusicClient("https://warm-recommendations.test","session");
 const get=vi.spyOn(api,"get").mockImplementation(async path=>path.startsWith("/providers")
  ?{page:1,sections:[{id:"guess",title:"猜你喜欢",source:"qqmusic",items:[track]}]}:{...result,items:[]});
 vi.useFakeTimers();
 try{
  await act(async()=>root.render(<DiscoverView {...props(api)}/>));
  await act(async()=>vi.advanceTimersByTime(500));
  expect(get.mock.calls.filter(([path])=>path.startsWith("/providers"))).toHaveLength(1);
  await act(async()=>[...host.querySelectorAll("button")].find(b=>b.textContent==="平台推荐")!.click());
  expect(host.textContent).toContain("原生歌曲");
  expect(host.textContent).not.toContain("正在加载推荐");
  expect(get.mock.calls.filter(([path])=>path.startsWith("/providers"))).toHaveLength(1);
 }finally{vi.useRealTimers();}
});
