// @vitest-environment jsdom
import {act} from "react";
import {createRoot,type Root} from "react-dom/client";
import {beforeEach,afterEach,it,expect,vi} from "vitest";
import {RMusicClient} from "@r-music/sdk";
import {StatsView} from "./Stats";
vi.mock("./CatalogLinks",()=>({CatalogLink:()=>null}));
vi.mock("./TrackContextMenu",()=>({useTrackContextMenu:()=>undefined}));
vi.mock("./components",()=>({Cover:()=> <div className="cover"/>,Empty:({headline}:{headline:string})=><p>{headline}</p>,IconButton:({label,children,...props}:{label:string;children:React.ReactNode})=><button aria-label={label} {...props}>{children}</button>}));
let host:HTMLDivElement,root:Root;
beforeEach(()=>{vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();vi.restoreAllMocks();});
it("历史所有请求完整范围，历史是独立子页并支持分页和播放",async()=>{
 const api=new RMusicClient("https://qa.invalid","qa"),play=vi.fn(async()=>{}),onError=vi.fn();
 const calls:string[]=[];vi.spyOn(api,"request").mockImplementation(async(path:string)=>{calls.push(path);if(path.startsWith("/stats/history"))return {items:[{session_id:"s",track_id:"song",title:"历史歌曲",artist:"QA",album:"Album",started_at_ms:Date.now(),listened_seconds:30,counted:true}],next_cursor:path.includes("cursor=")?null:"100:session"} as never;if(path==="/library/tracks/song")return {id:"song",title:"历史歌曲"} as never;return {timezone:"Asia/Shanghai",period_days:1,start_day:"2000-01-01",end_day:"2026-10-31",summary:{play_count:1,skip_count:0,listened_seconds:30,unique_tracks:1},days:[],tracks:[],artists:[]} as never;});
 await act(async()=>root.render(<StatsView api={api} revision={0} play={play} onError={onError}/>));
 await act(async()=>[...host.querySelectorAll("button")].find(b=>b.textContent==="历史所有")!.click());expect(calls).toContain("/stats?period_days=0");expect(host.querySelector("details")).toBeNull();
 await act(async()=>[...host.querySelectorAll("button")].find(b=>b.textContent==="聆听历史")!.click());expect(host.querySelector(".stats-summary")).toBeNull();expect(host.textContent).toContain("历史歌曲");expect(calls).toContain("/stats/history?limit=20");
 await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="播放 历史歌曲"]')!.click());expect(play).toHaveBeenCalledWith(expect.objectContaining({id:"song"}));
 await act(async()=>[...host.querySelectorAll("button")].find(b=>b.textContent?.includes("下一页"))!.click());expect(calls.some(path=>path.includes("cursor=100%3Asession"))).toBe(true);expect(onError).not.toHaveBeenCalled();
});
