// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach,expect,it,vi} from "vitest";
import {RMusicClient} from "@r-music/sdk";
import {LibraryHome} from "./LibraryHub";
import {readableModules} from "./Player";
afterEach(()=>vi.unstubAllGlobals());
it("首页可编排且专辑点击浏览，不抢占播放",async()=>{
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);const api=new RMusicClient("http://core","session");vi.spyOn(api,"request").mockResolvedValue({albums:[{id:"album",title:"夜航",artist:"艺人",track_count:8}],artists:[],tracks:[]});
 const browse=vi.fn(),play=vi.fn(),save=vi.fn().mockResolvedValue(undefined),host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 try{await act(async()=>root.render(<LibraryHome api={api} revision={0} preferences={{theme:"graphite",mode:"dark",custom_themes:[],low_performance:false}} savePreferences={save} play={play} browse={browse} onError={vi.fn()}/>));
 await act(async()=>[...host.querySelectorAll("button")].find(b=>b.textContent?.includes("夜航"))!.click());expect(browse).toHaveBeenCalledWith("album","album");expect(play).not.toHaveBeenCalled();
 await act(async()=>[...host.querySelectorAll("button")].find(b=>b.textContent==="编排首页")!.click());
 await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="下移最新添加的专辑"]')!.click());expect(save).toHaveBeenCalledWith({library_sections:["artists","albums","tracks"]});
 }finally{await act(async()=>root.unmount());host.remove();}
});
it("旧的过矮信息模块自动增高，并将冲突的控制区下移",()=>{
 const fixed=readableModules([{id:"info",x:0,y:380,w:40,h:100,visible:true},{id:"controls",x:0,y:500,w:40,h:100,visible:true},{id:"lyrics",x:44,y:0,w:56,h:470,visible:true}]);
 const info=fixed.find(v=>v.id==="info")!,controls=fixed.find(v=>v.id==="controls")!;expect(info.h).toBeGreaterThanOrEqual(200);expect(controls.y).toBeGreaterThan(info.y+info.h);expect(fixed.find(v=>v.id==="lyrics")!.y).toBe(0);
});
