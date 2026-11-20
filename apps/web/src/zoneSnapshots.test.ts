import {afterEach,expect,it,vi} from "vitest";
import {RMusicClient,type Zone} from "@r-music/sdk";
afterEach(()=>vi.unstubAllGlobals());
const reply=(value:unknown,status=200)=>new Response(JSON.stringify(value),{status});

it("reuses ordered queue pages for progress and volume changes, including duplicate tracks",async()=>{
 let revision=1,total=501,volume=.8,reads=0;
 vi.stubGlobal("fetch",vi.fn(async(input:string,options:RequestInit)=>{
  expect(new Headers(options.headers).get("authorization")).toBe("Bearer account-a");
  const url=new URL(input);
  if(url.pathname.endsWith("/zones"))return reply({queue_view:"versioned",items:[{id:"room",queue_revision:revision,queue_length:total,queue:[],volume}]});
  reads++;const offset=Number(url.searchParams.get("offset"));
  return reply({revision,offset,total,items:Array.from({length:Math.min(500,total-offset)},(_,i)=>({id:"same-track",title:`${revision}-${offset+i}`,provider_data:{source_audio:{verified:true,sample_rate:96000}}}))});
 }));
 const api=new RMusicClient("https://core.test","account-a");
 const first=await api.get<{items:Zone[]}>("/zones");expect(first.items[0].queue).toHaveLength(501);expect(reads).toBe(2);
 volume=.4;const second=await api.get<{items:Zone[]}>("/zones");expect(second.items[0].volume).toBe(.4);expect(reads).toBe(2);
 expect(second.items[0].queue[500].title).toBe("1-500");expect(second.items[0].queue[0].provider_data?.source_audio).toEqual({verified:true,sample_rate:96000});
 second.items[0].queue.splice(0,1);expect((await api.get<{items:Zone[]}>("/zones")).items[0].queue).toHaveLength(501);
 revision=2;total=0;expect((await api.get<{items:Zone[]}>("/zones")).items[0].queue).toEqual([]);expect(reads).toBe(2);
});

it("restarts a queue read when it changes between pages instead of mixing revisions",async()=>{
 let revision=1,states=0,pages=0;
 vi.stubGlobal("fetch",vi.fn(async(input:string)=>{
  const url=new URL(input);
  if(url.pathname.endsWith("/zones")){states++;return reply({queue_view:"versioned",items:[{id:"room",queue_revision:revision,queue_length:501,queue:[]}]});}
  pages++;const expected=Number(url.searchParams.get("revision")),offset=Number(url.searchParams.get("offset"));
  if(expected!==revision)return reply({error:{code:"queue_revision_changed",message:"changed"}},409);
  const result=reply({revision,offset,total:501,items:Array.from({length:Math.min(500,501-offset)},(_,i)=>({id:`${revision}-${offset+i}`}))});
  if(revision===1)revision=2;return result;
 }));
 const page=await new RMusicClient("https://core.test","account-a").get<{items:Zone[]}>("/zones");
 expect(states).toBe(2);expect(pages).toBe(4);expect(page.items[0].queue).toHaveLength(501);expect(page.items[0].queue.every(t=>t.id.startsWith("2-"))).toBe(true);
});

it("keeps queue caches inside the current account and cancels an older response after switching",async()=>{
 let release!:(value:Response)=>void;
 vi.stubGlobal("fetch",vi.fn(async(input:string,options:RequestInit)=>{
  const owner=new Headers(options.headers).get("authorization");
  if(owner==="Bearer old")return new Promise<Response>(resolve=>{release=resolve;});
  expect(input).toContain("/zones?view=state");return reply({queue_view:"versioned",items:[{id:"private",queue_revision:1,queue_length:0,queue:[]}]});
 }));
 const api=new RMusicClient("https://core.test","old"),pending=api.get("/zones");const rejected=expect(pending).rejects.toMatchObject({name:"AbortError"});
 api.token="new";expect((await api.get<{items:Zone[]}>("/zones")).items[0].queue).toEqual([]);
 release(reply({queue_view:"versioned",items:[{id:"private",queue_revision:1,queue_length:1,queue:[]}]}));await rejected;
});

it("keeps an existing Core's complete snapshot readable during an upgrade",async()=>{
 const fetch=vi.fn(async()=>reply({items:[{id:"old-core",queue:[{id:"song"}]}]}));vi.stubGlobal("fetch",fetch);
 const result=await new RMusicClient("https://core.test","account-a").get<{items:Zone[]}>("/zones");
 expect(result.items[0].queue[0].id).toBe("song");expect(fetch).toHaveBeenCalledTimes(1);
});

it("invalidates cached queues after Core restores an older history with the same revision",async()=>{
 let epoch="before",reads=0;
 vi.stubGlobal("fetch",vi.fn(async(input:string)=>{
  const url=new URL(input);
  if(url.pathname.endsWith("/zones"))return reply({queue_view:"versioned",queue_epoch:epoch,items:[{id:"room",queue_revision:1,queue_length:1,queue:[]}]});
  reads++;expect(url.searchParams.get("epoch")).toBe(epoch);
  return reply({epoch,revision:1,offset:0,total:1,items:[{id:epoch}]});
 }));
 const api=new RMusicClient("https://core.test","account-a");
 expect((await api.get<{items:Zone[]}>("/zones")).items[0].queue[0].id).toBe("before");
 epoch="restored";
 expect((await api.get<{items:Zone[]}>("/zones")).items[0].queue[0].id).toBe("restored");expect(reads).toBe(2);
});

it("queue writes return compact state and load the current ordered page once",async()=>{
 let revision=1,pages=0;
 vi.stubGlobal("fetch",vi.fn(async(input:string,options:RequestInit={})=>{
  const url=new URL(input);
  if(options.method==="POST"){expect(url.searchParams.get("view")).toBe("state");revision=2;return reply({id:"room",queue_revision:2,queue_length:2,queue:[]});}
  if(url.pathname.endsWith("/zones"))return reply({queue_view:"versioned",queue_epoch:"core",items:[{id:"room",queue_revision:revision,queue_length:revision,queue:[]}]});
  pages++;return reply({epoch:"core",revision,offset:0,total:revision,items:Array.from({length:revision},(_,i)=>({id:`song-${i}`}))});
 }));
 const api=new RMusicClient("https://core.test","account-a");await api.get("/zones");
 const zone=await api.post<Zone>("/zones/room/queue",{track_ids:["song-1"]});expect(zone.queue.map(t=>t.id)).toEqual(["song-0","song-1"]);expect(pages).toBe(2);
 await api.get("/zones");expect(pages).toBe(2);
});
