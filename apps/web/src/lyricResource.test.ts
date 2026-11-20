import {RMusicClient} from "@r-music/sdk";
import {expect,it,vi} from "vitest";
import {cachedLyrics,loadLyrics} from "./lyricResource";
it("预取、打开播放器与重复切歌复用同一真实歌词请求，账号改变不复用",async()=>{
 const api=new RMusicClient("https://test.invalid","first");let finish!:(v:unknown)=>void;const request=vi.spyOn(api,"request").mockImplementation(()=>new Promise(resolve=>{finish=resolve;}) as never);
 const a=loadLyrics(api,"song"),b=loadLyrics(api,"song");expect(request).toHaveBeenCalledTimes(1);
 const document={format:"lrc",text:"[00:01]真实行",word_timing:[{time:1,text:"真实行",words:[{time:1,duration:.3,text:"真实"},{time:1.4,duration:.2,text:"行"}]}]};finish(document);expect(await a).toEqual(await b);expect(cachedLyrics(api,"song")).toEqual(document);await loadLyrics(api,"song");expect(request).toHaveBeenCalledTimes(1);
 api.token="second";expect(cachedLyrics(api,"song")).toBeUndefined();
});
