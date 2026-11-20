// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach,expect,it,vi} from "vitest";
import {RMusicClient,type Track} from "@r-music/sdk";
import {CacheSongButton} from "./CacheSongButton";
import {saveApiFile} from "./fileDownload";
vi.mock("./fileDownload",()=>({saveApiFile:vi.fn(async()=>{})}));
afterEach(()=>{vi.useRealTimers();vi.unstubAllGlobals();});
it("下载排队不阻塞界面，完成后导出同一完整缓存文件",async()=>{
 vi.useFakeTimers();vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);const api=new RMusicClient("http://core","session"),post=vi.spyOn(api,"post").mockResolvedValue({status:"pending",job_id:"job"});vi.spyOn(api,"get").mockResolvedValue({status:"ready",cache_id:"complete-file"});
 const error=vi.fn(),track={id:"qqmusic:song",title:"原音质",format:"flac",source:"qqmusic"} as Track,host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 try{await act(async()=>root.render(<CacheSongButton api={api} track={track} onError={error}/>));await act(async()=>host.querySelector("button")!.click());expect(post).toHaveBeenCalledWith("/library/tracks/qqmusic%3Asong/cache",{});expect(host.querySelector("button")!.disabled).toBe(true);
 await act(async()=>vi.advanceTimersByTimeAsync(2500));expect(host.querySelector("button")!.getAttribute("aria-label")).toContain("导出音乐文件");await act(async()=>host.querySelector("button")!.click());expect(saveApiFile).toHaveBeenCalledWith(api,"/cache/complete-file/file","原音质.flac");expect(post).toHaveBeenCalledOnce();expect(error).not.toHaveBeenCalled();
 }finally{await act(async()=>root.unmount());host.remove();}
});
