import {afterEach,describe,expect,it,vi} from "vitest";
import {ApiError,RMusicClient} from "@r-music/sdk";

afterEach(()=>vi.unstubAllGlobals());
describe("impulse asset SDK",()=>{
  it("requests only the snapshotted selection with control auth and cancellation",async()=>{
    const manifest={items:[{asset:{id:"a".repeat(64)},api_path:"api/v1/dsp/impulses/a/file?ticket=scoped"}],expires_at:123};const fetcher=vi.fn().mockResolvedValue(new Response(JSON.stringify(manifest)));vi.stubGlobal("fetch",fetcher);
    const client=new RMusicClient("https://core.invalid/music","synthetic-control"),controller=new AbortController();expect(await client.resolveImpulses("zone /A",["a".repeat(64)],controller.signal)).toEqual(manifest);
    const[url,options]=fetcher.mock.calls[0];expect(url).toBe("https://core.invalid/music/api/v1/zones/zone%20%2FA/dsp/impulses");expect(options.method).toBe("POST");expect(JSON.parse(options.body)).toEqual({response_ids:["a".repeat(64)]});expect(options.headers.get("Authorization")).toBe("Bearer synthetic-control");expect(options.signal).toBe(controller.signal);
  });
  it("keeps a changed-selection conflict explicit instead of requesting another filter",async()=>{
    const fetcher=vi.fn().mockResolvedValue(new Response(JSON.stringify({error:{code:"impulse_selection_changed",message:"音区响应组已改变"}}),{status:409}));vi.stubGlobal("fetch",fetcher);
    await expect(new RMusicClient().resolveImpulses("zone",["a".repeat(64)])).rejects.toMatchObject({status:409,code:"impulse_selection_changed"});expect(fetcher).toHaveBeenCalledTimes(1);
  });
  it("uploads original bytes with an encoded name, selected Core prefix, auth and cancellation",async()=>{
    const fetcher=vi.fn().mockResolvedValue(new Response(JSON.stringify({id:"a".repeat(64)}),{headers:{"Content-Type":"application/json"}}));vi.stubGlobal("fetch",fetcher);
    const client=new RMusicClient("https://core.invalid/music/","synthetic-token");const bytes=new Uint8Array([0,255,128,17]).buffer;const cancel=new AbortController();
    await client.importImpulse(bytes,"房间 A&右.wav",cancel.signal);
    const [url,options]=fetcher.mock.calls[0];expect(url).toBe("https://core.invalid/music/api/v1/dsp/impulses?name="+encodeURIComponent("房间 A&右.wav"));expect(options.body).toBe(bytes);expect(options.signal).toBe(cancel.signal);expect(options.headers.get("Authorization")).toBe("Bearer synthetic-token");expect(options.headers.get("Content-Type")).toBe("audio/wav");
  });
  it("downloads exact binary bytes and retains server errors instead of treating audio as JSON",async()=>{
    const bytes=new Uint8Array([0,255,13,10]);const fetcher=vi.fn().mockResolvedValueOnce(new Response(bytes)).mockResolvedValueOnce(new Response(JSON.stringify({error:{code:"impulse_in_use",message:"used"}}),{status:409}));vi.stubGlobal("fetch",fetcher);
    const client=new RMusicClient("","synthetic-token");expect(new Uint8Array(await client.impulseFile("b".repeat(64)))).toEqual(bytes);
    await expect(client.deleteImpulse("b".repeat(64))).rejects.toMatchObject({status:409,code:"impulse_in_use",message:"used"});
  });
  it("keeps JSON and no-content request behavior and handles plain upload rejection",async()=>{
    const fetcher=vi.fn().mockResolvedValueOnce(new Response(JSON.stringify({items:[],limits:{max_files:128,max_storage_bytes:268435456,max_upload_bytes:4194304}}))).mockResolvedValueOnce(new Response(null,{status:204})).mockResolvedValueOnce(new Response("too large",{status:413}));vi.stubGlobal("fetch",fetcher);
    const client=new RMusicClient();expect((await client.listImpulses()).items).toEqual([]);await expect(client.deleteImpulse("c".repeat(64))).resolves.toBeUndefined();
    await expect(client.importImpulse(new ArrayBuffer(0))).rejects.toBeInstanceOf(ApiError);expect(fetcher.mock.calls[2][1].headers.get("Authorization")).toBeNull();
  });
});
