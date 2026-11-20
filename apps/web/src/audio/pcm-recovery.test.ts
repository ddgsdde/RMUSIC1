// @vitest-environment jsdom
import "fake-indexeddb/auto";
import {beforeEach,expect,it,vi} from "vitest";
import type {RMusicClient,Zone} from "@r-music/sdk";
import {ListenSampler,listenJournal,observeListenCore} from "../offlineStats";
import {pcmJournal,closePcmPage} from "./pcm-journal";
import {recoverPcm} from "./pcm-recovery";
const owner={core_id:"core",user_id:"user"},zone={id:"zone",endpoint_id:"browser-qa",status:"stopped",playback_id:null} as Zone;
beforeEach(async()=>{localStorage.clear();sessionStorage.clear();await new Promise<void>((yes,no)=>{const read=indexedDB.deleteDatabase("r-music-listens-v1");read.onsuccess=()=>yes();read.onerror=()=>no(read.error);});});
async function cursor(){return pcmJournal.begin({owner,endpoint:zone.endpoint_id,zone:zone.id,plan:"plan",rate:48000,heard:[{playback_id:"source",track_id:"track",position:1,frames:48000}],terminal:false});}
function api(user="user",core="core"){
  const request=vi.fn(async(path:string,init?:RequestInit):Promise<unknown>=>{
    if(path==="/status")return {core_id:core};if(path==="/auth/me")return {id:user};
    if(path.endsWith("/unavailable"))return {zone};
    if(path==="/stats/offline"){const body=JSON.parse(init!.body as string);return {accepted:body.sessions.map((row:{id:string;version:number})=>({id:row.id,version:row.version})),rejected:[],changed:true};}
    if(path.endsWith("/recover"))return {accepted:true,finished:false,zone:{...zone,status:"paused",playback_id:"new"}};
    throw new Error(path);
  });return {client:{baseUrl:"https://core.example/music",request} as unknown as RMusicClient,request};
}
it("uploads actual history first and retries one persisted request after a lost recovery confirmation",async()=>{
  const old=await cursor(),sampler=new ListenSampler({track_id:"track"},2),snapshot=sampler.finish("interrupted");snapshot.measured={playback_id:"source",listen_session_id:"group",segments:[{start:0,end:1,last_at:Date.now()}]};await listenJournal.save(owner,snapshot);
  const f=api(),original=f.request.getMockImplementation()!;let lose=true;f.request.mockImplementation(async(path,init)=>{const result=await original(path,init);if(path.endsWith("/recover")&&lose){lose=false;throw new TypeError("lost ACK");}return result;});
  await expect(recoverPcm(f.client,zone,new AbortController().signal)).rejects.toThrow("lost ACK");expect(await listenJournal.pending(owner)).toEqual([]);
  const frozen=await pcmJournal.load(owner,old.endpoint,old.zone);expect(frozen?.request).toBeDefined();
  expect(await recoverPcm(f.client,zone,new AbortController().signal)).toMatchObject({playback_id:"new",status:"paused"});
  const calls=f.request.mock.calls.filter(([path])=>path.endsWith("/recover"));expect(calls).toHaveLength(2);expect(calls[0][1]?.body).toBe(calls[1][1]?.body);
  expect(f.request.mock.calls.findIndex(([path])=>path==="/stats/offline")).toBeLessThan(f.request.mock.calls.findIndex(([path])=>path.endsWith("/recover")));
  expect((await pcmJournal.load(owner,old.endpoint,old.zone))?.request).toEqual(frozen?.request);
});
it("a different logged-in account never reads, uploads or recovers the previous owner's checkpoint",async()=>{
  const old=await cursor(),f=api("other");await recoverPcm(f.client,zone,new AbortController().signal);
  expect(f.request.mock.calls.some(([path])=>path==="/stats/offline"||path.endsWith("/recover"))).toBe(false);expect(await pcmJournal.load(owner,old.endpoint,old.zone)).toBeDefined();
});
it("Core identity changes fail before touching local history or recovery authority",async()=>{
  const f=api();observeListenCore(f.client.baseUrl,"different");await cursor();await expect(recoverPcm(f.client,zone,new AbortController().signal)).rejects.toThrow("核心身份已改变");expect(f.request).toHaveBeenCalledTimes(2);
});
it("an explicit queue revocation retires the checkpoint instead of resuming old audio",async()=>{
  const old=await cursor(),f=api(),original=f.request.getMockImplementation()!;f.request.mockImplementation(async(path,init)=>path.endsWith("/recover")?{accepted:false,zone}:original(path,init));
  expect(await recoverPcm(f.client,zone,new AbortController().signal)).toEqual(zone);expect(await pcmJournal.load(owner,old.endpoint,old.zone)).toBeUndefined();
});
it("normal page closure requires explicit play on a later visit",async()=>{
  const old=await cursor();closePcmPage(old);const f=api();await recoverPcm(f.client,zone,new AbortController().signal);
  expect(f.request.mock.calls.some(([path])=>path.endsWith("/recover"))).toBe(false);expect(await pcmJournal.load(owner,old.endpoint,old.zone)).toBeDefined();
  expect(await recoverPcm(f.client,zone,new AbortController().signal,true)).toMatchObject({playback_id:"new"});
});
