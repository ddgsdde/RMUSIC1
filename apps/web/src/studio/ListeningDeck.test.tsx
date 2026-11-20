// @vitest-environment jsdom
import {act} from "react";
import {createRoot,type Root} from "react-dom/client";
import {beforeEach,afterEach,expect,it,vi} from "vitest";
import {RMusicClient,type Track} from "@r-music/sdk";
import {ListeningDeck} from "./ListeningDeck";
import {TrackMenuContext} from "../trackMenuContext";
let host:HTMLDivElement,root:Root;
const tracks=[0,1,2].map(n=>({id:`local:${n}`,title:`歌曲${n}`,artist:"艺人",available:true,duration:100,source:"local"})) as Track[];
const menu=vi.fn(),play=vi.fn(async()=>{}),scroll=vi.fn();
async function render(order=tracks){await act(async()=>root.render(<TrackMenuContext.Provider value={menu}><ListeningDeck api={new RMusicClient("https://core.test","")} tracks={order} canPlay play={play} details={()=>{}} setup={()=>{}} loading={false}/></TrackMenuContext.Provider>));}
function pointer(kind:string,x=0){const event=new MouseEvent(kind,{bubbles:true,cancelable:true,clientX:x});Object.defineProperties(event,{pointerType:{value:"touch"},isPrimary:{value:true}});return event;}
beforeEach(()=>{vi.useFakeTimers();vi.clearAllMocks();vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);vi.spyOn(HTMLElement.prototype,"clientWidth","get").mockReturnValue(330);HTMLElement.prototype.scrollTo=scroll;host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.useRealTimers();vi.restoreAllMocks();vi.unstubAllGlobals();});
it("精选自动切换、滑动更新索引，后台停止轮播",async()=>{
 await render();expect(host.querySelector('.studio-flip')).toBeNull();
 await act(async()=>vi.advanceTimersByTimeAsync(6000));expect(scroll).toHaveBeenLastCalledWith({left:330,behavior:"smooth"});
 const rail=host.querySelector<HTMLDivElement>('.studio-deck-rail')!;rail.scrollLeft=660;
 await act(async()=>rail.dispatchEvent(new Event("scroll",{bubbles:true})));expect(host.querySelectorAll('[aria-pressed=true]')[0]?.getAttribute('aria-label')).toBe('选中 歌曲2');
 vi.spyOn(document,"hidden","get").mockReturnValue(true);await act(async()=>document.dispatchEvent(new Event("visibilitychange")));scroll.mockClear();await act(async()=>vi.advanceTimersByTimeAsync(12000));expect(scroll).not.toHaveBeenCalled();
});
it("长按打开歌曲菜单并拦住播放，滑动取消长按",async()=>{
 await render();const card=host.querySelector<HTMLElement>('.studio-deck')!,button=card.querySelector<HTMLButtonElement>('.studio-play')!;
 await act(async()=>card.dispatchEvent(pointer("pointerdown")));await act(async()=>vi.advanceTimersByTimeAsync(500));expect(menu).toHaveBeenCalledWith(expect.anything(),tracks[0],undefined,undefined);
 await act(async()=>button.click());expect(play).not.toHaveBeenCalled();
 menu.mockClear();await act(async()=>{card.dispatchEvent(pointer("pointerdown"));card.dispatchEvent(pointer("pointermove",30));});await act(async()=>vi.advanceTimersByTimeAsync(500));expect(menu).not.toHaveBeenCalled();
});
it("后台补充推荐后保留正在查看的歌曲和滑动偏移",async()=>{
 await render();
 const rail=host.querySelector<HTMLDivElement>(".studio-deck-rail")!;
 rail.scrollLeft=345;
 await act(async()=>rail.dispatchEvent(new Event("scroll",{bubbles:true})));
 await render([tracks[2],tracks[0],tracks[1]]);
 expect(rail.scrollLeft).toBe(675);
 expect(host.querySelector(".studio-deck-dots [aria-pressed=true]")?.getAttribute("aria-label")).toBe("选中 歌曲1");
});
