// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {it,expect,vi} from "vitest";
import {RMusicClient} from "@r-music/sdk";
import {CoreResourceSettings} from "./BackgroundTasks";
it("saves only editable resource settings when the Core response includes unrelated settings",async()=>{
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);const requests:unknown[]=[];
 vi.stubGlobal("fetch",vi.fn(async(_url:unknown,options?:RequestInit)=>{if(options?.method==="PATCH")requests.push(JSON.parse(options.body as string));return new Response(JSON.stringify(options?.method==="PATCH"?{saved:true}:{name:"Core",strict_sync_available:false,managed_memory_bytes:1073741824,background_concurrency:2,analysis_threads:1,playback_cache_mode:"full",resources:{budget_bytes:1073741824,used_bytes:0,playback_reserved_bytes:268435456,active_tasks:0}}));}));
 const host=document.createElement("div"),root=createRoot(host);document.body.append(host);
 try{await act(async()=>root.render(<CoreResourceSettings api={new RMusicClient("http://core.test")} onError={error=>{throw error}} notify={()=>{}}/>));await act(async()=>host.querySelector<HTMLButtonElement>("button.primary")!.click());expect(requests).toEqual([{managed_memory_bytes:1073741824,background_concurrency:2,analysis_threads:1,playback_cache_mode:"full"}]);}
 finally{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();}
});
