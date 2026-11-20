// @vitest-environment jsdom
import {afterEach,describe,expect,it,vi} from "vitest";
import {act} from "react";
import {createRoot,type Root} from "react-dom/client";
import {CacheRepair} from "./CacheRepair";
import type {RMusicClient} from "@r-music/sdk";
(globalThis as {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
let root:Root;let host:HTMLDivElement;
afterEach(async()=>{if(root)await act(async()=>root.unmount());host?.remove();});
async function render(enabled:boolean){host=document.createElement("div");document.body.append(host);root=createRoot(host);const api={request:vi.fn().mockResolvedValue({enabled,job:null}),post:vi.fn().mockResolvedValue({job_id:"repair"})};await act(async()=>root.render(<CacheRepair api={api as unknown as RMusicClient} onError={()=>{}}/>));return api;}
describe("missing cache repair",()=>{
 it("does not offer a download while Core caching is disabled",async()=>{const api=await render(false);const button=Array.from(host.querySelectorAll("button")).find(value=>value.textContent==="补齐所有缺失")!;expect(button.disabled).toBe(true);expect(api.post).not.toHaveBeenCalled();});
 it("starts only after confirming disk use and sends selected concurrency",async()=>{const api=await render(true);await act(async()=>Array.from(host.querySelectorAll("button")).find(value=>value.textContent==="补齐所有缺失")!.click());expect(api.post).not.toHaveBeenCalled();await act(async()=>Array.from(host.querySelectorAll("button")).find(value=>value.textContent==="确认开始补齐")!.click());expect(api.post).toHaveBeenCalledWith("/library/cache-repair",{concurrency:1});});
 it("aborts an outstanding refresh when the page is released",async()=>{const api=await render(true);const signal=(api.request.mock.calls[0] as unknown as [string,{signal:AbortSignal}])[1].signal;await act(async()=>root.unmount());root=undefined as unknown as Root;expect(signal.aborted).toBe(true);});
});
