// @vitest-environment jsdom
import {act} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {BrowserOffline} from "./BrowserOffline";
import {listenJournal,rememberListenOwner} from "./offlineStats";
const cache=vi.hoisted(()=>({list:vi.fn(),blob:vi.fn(),protect:vi.fn()}));
vi.mock("./browserCache",()=>({browserCache:cache,formatBytes:(n:number)=>`${n} B`}));
const entry={id:"cached-a",track_id:"song-a",title:"本机验收曲",artist:"QA",complete:true,size:100};
const onError=vi.fn(),create=vi.fn(),revoke=vi.fn(),play=vi.fn(),pause=vi.fn(),load=vi.fn();
let host:HTMLDivElement,root:Root;
beforeEach(()=>{
 localStorage.clear();vi.spyOn(listenJournal,"save").mockResolvedValue(undefined);
 vi.useFakeTimers();vi.clearAllMocks();vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);const OriginalURL=URL;vi.stubGlobal("URL",class extends OriginalURL{static createObjectURL=create;static revokeObjectURL=revoke;});create.mockReturnValue("blob:cached-a");
 cache.list.mockResolvedValue([entry]);cache.blob.mockResolvedValue(new Blob(["audio"]));
 play.mockImplementation(function(this:HTMLAudioElement){Object.defineProperty(this,"paused",{value:false,configurable:true});this.dispatchEvent(new Event("play"));return Promise.resolve();});pause.mockImplementation(function(this:HTMLAudioElement){Object.defineProperty(this,"paused",{value:true,configurable:true});this.dispatchEvent(new Event("pause"));});
 vi.spyOn(HTMLMediaElement.prototype,"play").mockImplementation(play);vi.spyOn(HTMLMediaElement.prototype,"pause").mockImplementation(pause);vi.spyOn(HTMLMediaElement.prototype,"load").mockImplementation(load);
 host=document.createElement("div");document.body.append(host);root=createRoot(host);
});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.restoreAllMocks();vi.useRealTimers();vi.unstubAllGlobals();});
async function render(hidden=false){await act(async()=>root.render(<div hidden={hidden}><BrowserOffline onError={onError}/></div>));}
async function start(){await act(async()=>host.querySelector<HTMLButtonElement>(".download-list button")!.click());}
it("关闭控制层保留离线音频和保护标记，不重新加载文件",async()=>{
 await render();await start();const audio=host.querySelector('audio');await render(true);expect(host.querySelector('audio')).toBe(audio);expect(audio?.getAttribute('src')).toBe("blob:cached-a");expect(play).toHaveBeenCalledTimes(1);expect(cache.protect).toHaveBeenCalledWith("song-a","offline");
});
it("选择另一首而读取失败时不能继续播放上一首",async()=>{
 await render();await start();cache.blob.mockRejectedValueOnce(new Error("缓存存在缺口"));await start();
 expect(host.querySelector('audio')?.getAttribute('src')).toBeNull();expect(host.querySelector<HTMLButtonElement>('[aria-label="继续离线播放"]')?.disabled).toBe(true);expect(revoke).toHaveBeenCalledWith("blob:cached-a");expect(onError).toHaveBeenCalledTimes(1);
});
it("停止或显式切回在线后忽略迟到的缓存读取",async()=>{
 let resolve!:(value:Blob)=>void;cache.blob.mockReturnValue(new Promise<Blob>(yes=>{resolve=yes;}));await render();await start();await act(async()=>window.dispatchEvent(new Event("rmusic-online")));await act(async()=>resolve(new Blob(["late"])));
 expect(create).not.toHaveBeenCalled();expect(play).not.toHaveBeenCalled();expect(host.querySelector('.offline-player')).toBeNull();expect(cache.protect).toHaveBeenCalledWith(undefined,"offline");
});
it("进度、定位、暂停和静音使用实际媒体元素",async()=>{
 await render();await start();const audio=host.querySelector('audio')!;Object.defineProperty(audio,"duration",{value:12,configurable:true});audio.currentTime=6;await act(async()=>{audio.dispatchEvent(new Event("loadedmetadata"));audio.dispatchEvent(new Event("timeupdate"));});
 expect(host.querySelector<HTMLInputElement>('[aria-label="离线播放进度"]')?.value).toBe("6");
 await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="暂停离线播放"]')!.click());expect(host.querySelector('[aria-label="继续离线播放"]')).not.toBeNull();
 await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="离线静音"]')!.click());expect(audio.muted).toBe(true);
 const input=host.querySelector<HTMLInputElement>('[aria-label="离线播放进度"]')!;await act(async()=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value')!.set!.call(input,"8");input.dispatchEvent(new Event("input",{bubbles:true}));});expect(audio.currentTime).toBe(8);
});
it("缓存统计采样实际进度，暂停和定位不增加秒数，退出账号后不再归入旧账号",async()=>{
 const owner={core_id:"core-a",user_id:"user-a"};rememberListenOwner("",owner);cache.list.mockResolvedValue([{...entry,core_id:owner.core_id,user_id:owner.user_id}]);
 vi.setSystemTime(Date.parse("2026-10-13T12:00:00Z"));let mono=0;vi.spyOn(performance,"now").mockImplementation(()=>mono);
 await render();await start();const audio=host.querySelector('audio')!;Object.defineProperty(audio,"duration",{value:120,configurable:true});await act(async()=>audio.dispatchEvent(new Event("loadedmetadata")));
 const advance=async(seconds:number,position:number)=>{mono+=seconds*1000;vi.setSystemTime(Date.now()+seconds*1000);audio.currentTime=position;await act(async()=>audio.dispatchEvent(new Event("timeupdate")));};
 await advance(10,10);await act(async()=>audio.pause());await advance(10,10);await act(async()=>audio.play());
 audio.currentTime=90;await act(async()=>{audio.dispatchEvent(new Event("seeking"));audio.dispatchEvent(new Event("seeked"));});await advance(10,100);
 await act(async()=>window.dispatchEvent(new Event("rmusic-listen-owner-changed")));const final=vi.mocked(listenJournal.save).mock.calls.at(-1)!;
 expect(final[0]).toEqual(owner);expect(Object.values(final[1].days).reduce((sum,day)=>sum+day.seconds,0)).toBe(20);expect(final[1].reason).toBe("interrupted");
 const saved=vi.mocked(listenJournal.save).mock.calls.length;await advance(5,105);expect(listenJournal.save).toHaveBeenCalledTimes(saved);
});
