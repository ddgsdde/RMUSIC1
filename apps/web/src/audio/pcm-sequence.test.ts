// @vitest-environment jsdom
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {ApiError,type RMusicClient,type Track,type Zone} from "@r-music/sdk";
import type {PcmSnapshot} from "./pcm-renderer";
import {PcmSequence,pcmSequenceKey} from "./pcm-sequence";
import {pcmJournal,type PcmCursor} from "./pcm-journal";
import {PcmTransportError} from "./pcm-player";
const runtime=vi.hoisted(()=>({players:[] as {emit:(state:PcmSnapshot,frame:number)=>void;fail:(error:Error)=>void;pause:ReturnType<typeof vi.fn>;stop:ReturnType<typeof vi.fn>;append:ReturnType<typeof vi.fn>}[],save:vi.fn()}));
vi.mock("../offlineStats",()=>({listenCore:()=>"core-a",listenJournal:{save:(...args:unknown[])=>runtime.save(...args)}}));
vi.mock("./pcm-player",async importOriginal=>({...await importOriginal<typeof import("./pcm-player")>(),PcmPlayer:{create:async(_context:unknown,onState:(state:PcmSnapshot,frame:number)=>void,onError:(error:Error)=>void)=>{
  const pause=vi.fn(),stop=vi.fn(async()=>undefined),append=vi.fn();runtime.players.push({emit:onState,fail:onError,pause,stop,append});
  return {node:{},append,seal:vi.fn(),setPaused:pause,ready:Promise.resolve(),stop};
}}}));
beforeEach(()=>{runtime.players=[];runtime.save.mockReset().mockResolvedValue(undefined);});
afterEach(()=>{vi.useRealTimers();vi.restoreAllMocks();vi.unstubAllGlobals();});
const tracks=[2,3,4].map((duration,i)=>({id:`track-${i}`,title:`Track ${i}`,artist:"qa",duration,format:"wav",sample_rate:48000,bit_depth:24,channels:2,integrated_lufs:null})) as Track[];
function zone(id="a",status:Zone["status"]="playing",updated_at=100):Zone{return {id:"zone",name:"qa",endpoint_id:"browser-qa",playback_id:id,track:tracks[["a","b","c"].indexOf(id)],queue:tracks,queue_index:["a","b","c"].indexOf(id),position:0,repeat:"off",shuffle:false,status,volume:1,muted:false,updated_at,dsp:{enabled:false,preamp_db:0,balance:0,normalize:false,target_lufs:-14,bands:[]},transitions:{fade_in_ms:0,fade_out_ms:0}};}
function deferred<T>(){let resolve!:(value:T)=>void;const promise=new Promise<T>(yes=>resolve=yes);return {promise,resolve};}
async function spin(){for(let i=0;i<80;i++)await Promise.resolve();}
function snapshot(id:"b"|"c",frames:number):PcmSnapshot {
  return id==="b"?{sources:[{id:"b",frames,buffered:48000},{id:"c",frames:0,buffered:48000}],ended:[{id:"a",frames:96000,buffered:0,endFrame:96000}],spans:[{id:"a",start:0,end:96000,endFrame:96000},{id:"b",start:0,end:frames,endFrame:96000+frames}],frames:96000+frames,outputFrames:96000+frames,underrunFrames:0,finished:false}:
    {sources:[{id:"c",frames,buffered:48000}],ended:[{id:"b",frames:144000,buffered:0,endFrame:240000}],spans:[{id:"b",start:24000,end:144000,endFrame:240000},{id:"c",start:0,end:frames,endFrame:240000+frames}],frames:240000+frames,outputFrames:240000+frames,underrunFrames:0,finished:false};
}
function setup(handoff:(body:Record<string,unknown>)=>Promise<unknown>,durable=false,initialZone=zone(),allowLossy=false){
  const bodies:Record<string,unknown>[]=[],adopt=vi.fn(),error=vi.fn(),view=vi.fn();
  const request=vi.fn(async(path:string,init?:RequestInit)=>{
    const body=init?.body?JSON.parse(init.body as string):undefined;
    if(path==="/status")return {core_id:"core-a",capabilities:{browser_pcm:true,browser_aac_pcm:true,measured_listens:true}};
    if(path==="/auth/me")return {id:"user-a"};
    if(path.endsWith("/prepare")){const index=["a","b","c"].indexOf(body.playback_id);return {accepted:true,continuous_recovery:durable,plan_id:"plan",from_playback_id:body.playback_id,listen_session_id:`group-${body.playback_id}`,items:["a","b","c"].slice(index+1).map((id,j)=>({playback_id:id,listen_session_id:`group-${id}`,track:tracks[index+j+1],media_path:`api/v1/media/${id}?ticket=limited`})),has_more:false};}
    if(path.endsWith("/handoff")){bodies.push(structuredClone(body));return handoff(body);}
    if(path.endsWith("/progress"))return undefined;
    if(path==="/zones/zone")return initialZone;
    throw new Error(path);
  });
  const api={baseUrl:"https://core.example/music",request,url:(path:string)=>"https://core.example/music"+path,pcmUrl:vi.fn(async()=>"https://core.example/music/api/v1/pcm/a?ticket=limited&rate=48000&start=0"),aacPcmUrl:vi.fn(async()=>"https://core.example/music/api/v1/transcode/a?ticket=fresh&pcm_rate=48000&bitrate=128")} as unknown as RMusicClient;
  const actor=new PcmSequence(api,initialZone,{sampleRate:48000,currentTime:0,state:"running"} as AudioContext,()=>{},view,adopt,error,allowLossy);
  return {actor,bodies,adopt,error,request,api,view};
}
function buffering(frames:number,underrunFrames:number):PcmSnapshot {return {sources:[{id:"a",frames,buffered:0}],ended:[],spans:[],frames,outputFrames:frames+underrunFrames,underrunFrames,finished:false};}
it("switches only the buffered current source to AAC after three distinct stalls and keeps future sources at full quality",async()=>{
  vi.stubGlobal("AudioDecoder",{isConfigSupported:vi.fn(async()=>({supported:true}))});const f=setup(async()=>({accepted:true,zone:zone("b")}),false,zone(),true);await f.actor.start();
  for(const [i,frames] of [24000,24000,24000].entries()){runtime.players[0].emit(buffering(frames,128*(i+1)),frames+384);await spin();}expect(runtime.players).toHaveLength(1);
  runtime.players[0].emit(buffering(48000,512),48512);await spin();runtime.players[0].emit(buffering(72000,640),72640);await spin();
  expect(runtime.players).toHaveLength(2);expect(f.api.aacPcmUrl).toHaveBeenCalledWith(tracks[0].id,48000,1.5,expect.any(AbortSignal));
  expect(runtime.players[1].append.mock.calls[0][0].map((x:{codec?:string})=>x.codec)).toEqual(["aac",undefined,undefined]);expect(f.view).toHaveBeenLastCalledWith(expect.objectContaining({adaptive:128}));
  runtime.players[1].emit({sources:[{id:"b",frames:12000,buffered:48000}],ended:[{id:"a",frames:24000,buffered:0,endFrame:24000}],spans:[],frames:36000,outputFrames:36000,underrunFrames:0,finished:false},108640);await spin();
  expect(f.view).toHaveBeenLastCalledWith(expect.objectContaining({adaptive:0}));await f.actor.stop();
});
it("pause does not count as weak-network stalls",async()=>{
  const support=vi.fn(async()=>({supported:true}));vi.stubGlobal("AudioDecoder",{isConfigSupported:support});const f=setup(async()=>({accepted:true,zone:zone("b")}));await f.actor.start();f.actor.update(zone("a","paused",200));
  for(const [i,frames] of [24000,48000,72000].entries()){runtime.players[0].emit(buffering(frames,(i+1)*128),frames);await spin();}
  expect(support).not.toHaveBeenCalled();expect(runtime.players).toHaveLength(1);await f.actor.stop();
});
it("stop while AAC capability detection is pending cannot open a late lossy output",async()=>{
  const held=deferred<{supported:boolean}>();vi.stubGlobal("AudioDecoder",{isConfigSupported:()=>held.promise});const f=setup(async()=>({accepted:true,zone:zone("b")}));await f.actor.start();
  for(const [i,frames] of [24000,48000,72000].entries()){runtime.players[0].emit(buffering(frames,(i+1)*128),frames);await spin();}
  await f.actor.stop("stop");held.resolve({supported:true});await spin();expect(runtime.players).toHaveLength(1);expect(f.api.aacPcmUrl).not.toHaveBeenCalled();
});
it("automatically reconnects from consumed frames with a new ticket and current pause/gain",async()=>{
  vi.useFakeTimers();const f=setup(async()=>({accepted:true,zone:zone("b")})),original=f.request.getMockImplementation()!;
  f.request.mockImplementation(async(path,init)=>path==="/zones/zone"?{...zone("a","paused",300),muted:true,volume:.04}:original(path,init));
  await f.actor.start();runtime.players[0].emit({sources:[{id:"a",frames:12000,buffered:0}],ended:[],spans:[{id:"a",start:0,end:12000,endFrame:12000}],frames:12000,outputFrames:12000,underrunFrames:0,finished:false},12000);await spin();
  runtime.players[0].fail(new PcmTransportError("connection lost"));await vi.advanceTimersByTimeAsync(1000);await spin();
  expect(runtime.players).toHaveLength(2);expect(f.api.pcmUrl).toHaveBeenCalledWith(tracks[0].id,48000,.25,expect.any(AbortSignal));
  expect(runtime.players[1].pause).toHaveBeenLastCalledWith(true);expect(f.adopt).toHaveBeenLastCalledWith(expect.objectContaining({status:"paused",muted:true,volume:.04}));
  await f.actor.stop();expect(runtime.save.mock.calls.flatMap(([,row])=>row.measured.segments).at(-1)).toMatchObject({start:0,end:.25});
});
it("limits consecutive media failures to five retries and stop cancels a pending retry",async()=>{
  vi.useFakeTimers();const f=setup(async()=>({accepted:true,zone:zone("b")}));await f.actor.start();
  for(const delay of [1000,2000,4000,8000,16000]){runtime.players.at(-1)!.fail(new PcmTransportError("network"));await vi.advanceTimersByTimeAsync(delay);await spin();}
  expect(runtime.players).toHaveLength(6);runtime.players.at(-1)!.fail(new PcmTransportError("network"));await vi.advanceTimersByTimeAsync(60000);expect(runtime.players).toHaveLength(6);
  expect(f.error.mock.calls.at(-1)?.[0].message).toContain("点击播放");
  f.actor.requestRetry();await spin();expect(runtime.players).toHaveLength(7);runtime.players.at(-1)!.fail(new PcmTransportError("network"));await f.actor.stop("stop");await vi.advanceTimersByTimeAsync(60000);expect(runtime.players).toHaveLength(7);
});
it("a permanent PCM response does not automatically retry",async()=>{
  vi.useFakeTimers();const f=setup(async()=>({accepted:true,zone:zone("b")}));await f.actor.start();runtime.players[0].fail(new PcmTransportError("HTTP 403",false));await vi.advanceTimersByTimeAsync(60000);expect(runtime.players).toHaveLength(1);await f.actor.stop();
});
it("waits for the frozen handoff before reconnecting and never appends to the failed player",async()=>{
  vi.useFakeTimers();const held=deferred<unknown>(),f=setup(()=>held.promise),original=f.request.getMockImplementation()!;let prepares=0;
  f.request.mockImplementation(async(path,init)=>{
    if(path==="/zones/zone")return zone("b","playing",300);
    const value=await original(path,init);return path.endsWith("/prepare")&&++prepares===1?{...value as Record<string,unknown>,items:(value as {items:{playback_id:string}[]}).items.filter(item=>item.playback_id==="b")}:value;
  });
  await f.actor.start();runtime.players[0].emit(snapshot("b",12000),108000);await spin();const frozen=structuredClone(f.bodies[0]);
  runtime.players[0].append.mockImplementation(()=>{throw new Error("closed player");});runtime.players[0].fail(new PcmTransportError("network"));await vi.advanceTimersByTimeAsync(1000);expect(runtime.players).toHaveLength(1);
  held.resolve({accepted:true,zone:zone("b","playing",300)});await spin();expect(runtime.players).toHaveLength(2);expect(f.bodies[0]).toEqual(frozen);expect(runtime.players[1].append.mock.calls[0][0][0]).toMatchObject({id:"b",start:.25});await f.actor.stop();
});
it("an explicit new Core playback instance cancels the pending media retry",async()=>{
  vi.useFakeTimers();const f=setup(async()=>({accepted:true,zone:zone("b")})),original=f.request.getMockImplementation()!;
  f.request.mockImplementation(async(path,init)=>path==="/zones/zone"?zone("c","playing",400):original(path,init));
  await f.actor.start();runtime.players[0].fail(new PcmTransportError("network"));await vi.advanceTimersByTimeAsync(1000);await spin();expect(runtime.players).toHaveLength(1);expect(f.actor.active).toBe(false);expect(f.adopt).toHaveBeenLastCalledWith(zone("c","playing",400));
});
it("accepts only a reserved source's explicit queue projection before its HTTP handoff ACK",async()=>{
  const f=setup(async()=>({accepted:true,zone:zone("b")}),false,{...zone(),queue:[tracks[0]]}),original=f.request.getMockImplementation()!;
  f.request.mockImplementation(async(path,init)=>{const value=await original(path,init);if(!path.endsWith("/prepare"))return value;const prepared=value as {items:{playback_id:string}[]};return {...prepared,items:prepared.items.map(item=>item.playback_id==="b"?{...item,queue:tracks,queue_index:1}:item)};});
  await f.actor.start();expect(f.actor.projected(zone("b"))).toBe(true);expect(f.actor.projected(zone("a"))).toBe(false);expect(f.actor.projected({...zone("b"),queue_index:0})).toBe(false);expect(f.actor.projected({...zone("b"),dsp:{...zone().dsp,enabled:true}})).toBe(false);expect(f.actor.projected({...zone("b"),queue:[tracks[1],tracks[0],tracks[2]]})).toBe(false);await f.actor.stop();
});
it("refreshes pending recommendations during the last source without rebuilding its player",async()=>{
  vi.useFakeTimers();const f=setup(async()=>({accepted:true,zone:zone("b")})),original=f.request.getMockImplementation()!;let prepares=0;
  f.request.mockImplementation(async(path,init)=>{const value=await original(path,init);return path.endsWith("/prepare")?{...value as Record<string,unknown>,...(++prepares===1?{items:[],refresh:true}:{refresh:false})}:value;});
  await f.actor.start();runtime.players[0].emit({sources:[{id:"a",frames:4800,buffered:48000}],ended:[],spans:[{id:"a",start:0,end:4800,endFrame:4800}],frames:4800,outputFrames:4800,underrunFrames:0,finished:false},4800);await spin();
  await vi.advanceTimersByTimeAsync(1000);runtime.players[0].emit({sources:[{id:"a",frames:57600,buffered:48000}],ended:[],spans:[{id:"a",start:4800,end:57600,endFrame:57600}],frames:57600,outputFrames:57600,underrunFrames:0,finished:false},57600);await spin();
  expect(prepares).toBe(2);expect(runtime.players).toHaveLength(1);expect(runtime.players[0].append.mock.calls.at(-1)?.[0].map((item:{id:string})=>item.id)).toEqual(["b","c"]);await f.actor.stop();
});
it("Core restart's stale progress 404 preserves the durable cursor with its exact listening evidence",async()=>{
  vi.spyOn(pcmJournal,"begin").mockImplementation(async value=>({...value,id:"cursor",run:"run",tab:"tab",version:1}));
  const checkpoint=vi.spyOn(pcmJournal,"checkpoint").mockResolvedValue(undefined),retire=vi.spyOn(pcmJournal,"retire").mockResolvedValue(undefined);
  const f=setup(async()=>({accepted:true,zone:zone("b")}),true),original=f.request.getMockImplementation()!;
  f.request.mockImplementation(async(path,init)=>{if(path.endsWith("/progress"))throw new ApiError(404,"not_found","instance unavailable");return original(path,init);});
  await f.actor.start();runtime.players[0].emit({sources:[{id:"a",frames:24000,buffered:48000}],ended:[],spans:[{id:"a",start:0,end:24000,endFrame:24000}],frames:24000,outputFrames:24000,underrunFrames:0,finished:false},24000);await spin();
  expect(f.actor.active).toBe(false);expect(retire).not.toHaveBeenCalled();
  const [cursor,snapshots]=checkpoint.mock.calls.at(-1) as [PcmCursor,{measured?:{segments:{start:number;end:number}[]}}[]];
  expect(cursor.heard[0]).toMatchObject({frames:24000,position:.5});expect(snapshots[0].measured?.segments).toMatchObject([{start:0,end:.5}]);
});
it("a failed journal transaction prevents later cursors from advancing beyond durable listening",async()=>{
  vi.spyOn(pcmJournal,"begin").mockImplementation(async value=>({...value,id:"cursor",run:"run",tab:"tab",version:1}));
  const checkpoint=vi.spyOn(pcmJournal,"checkpoint").mockRejectedValue(new Error("storage unavailable"));vi.spyOn(pcmJournal,"retire").mockResolvedValue(undefined);
  const f=setup(async()=>({accepted:true,zone:zone("b")}),true);await f.actor.start();
  runtime.players[0].emit({sources:[{id:"a",frames:24000,buffered:48000}],ended:[],spans:[{id:"a",start:0,end:24000,endFrame:24000}],frames:24000,outputFrames:24000,underrunFrames:0,finished:false},24000);await spin();
  f.actor.update(zone("a","paused",200));runtime.players[0].emit({sources:[{id:"a",frames:48000,buffered:48000}],ended:[],spans:[{id:"a",start:24000,end:48000,endFrame:48000}],frames:48000,outputFrames:48000,underrunFrames:0,finished:false},48000);await spin();
  expect(checkpoint).toHaveBeenCalledTimes(1);expect(f.error).toHaveBeenCalledTimes(1);expect(runtime.save.mock.calls.at(-1)?.[1].measured.segments).toMatchObject([{start:0,end:1}]);await f.actor.stop();
});
it("natural handoff JSON field order does not change the audio recipe",()=>{
  const zone={queue:[],repeat:"off",shuffle:false,autoplay:{enabled:false,source:"mixed",fallback_local:true},dsp:{enabled:false,preamp_db:0,balance:0,normalize:false,target_lufs:-14,bands:[]},transitions:{fade_in_ms:0,fade_out_ms:0}} as unknown as Zone;
  const handoff={...zone,autoplay:{enabled:false,fallback_local:true,source:"mixed"},dsp:{bands:[],target_lufs:-14,normalize:false,balance:0,preamp_db:0,enabled:false}} as Zone;
  expect(pcmSequenceKey(handoff)).toBe(pcmSequenceKey(zone));
});
it("freezes each handoff while rendering proceeds and preserves newer pause and gain",async()=>{
  const first=deferred<unknown>();const f=setup(async body=>(body.from_playback_id==="a"?first.promise:{accepted:true,applied:true,zone:{...zone("c","paused",400),volume:.03,muted:true}}));
  await f.actor.start();const one=runtime.players[0];one.emit(snapshot("b",24000),120000);await spin();expect(f.bodies).toHaveLength(1);const frozen=structuredClone(f.bodies[0]);
  one.emit(snapshot("c",48000),288000);f.actor.update({...zone("a","paused",200),volume:.03,muted:true});first.resolve({accepted:true,applied:true,zone:zone("b","playing",300)});await spin();
  expect(f.bodies).toHaveLength(2);expect(f.bodies[0]).toEqual(frozen);expect(f.bodies[1].from_playback_id).toBe("b");
  expect((f.bodies[1].ended as {playback_id:string}[]).map(value=>value.playback_id)).toEqual(["b"]);
  expect(f.adopt.mock.calls[0][0]).toMatchObject({playback_id:"b",status:"paused",volume:.03,muted:true});expect(one.pause).toHaveBeenLastCalledWith(true);
  expect(runtime.players).toHaveLength(1);await f.actor.stop();expect(runtime.save.mock.calls.every(([owner])=>owner.core_id==="core-a"&&owner.user_id==="user-a")).toBe(true);
});
it("stop ignores the late successful handoff instead of reviving its queue",async()=>{
  const held=deferred<unknown>(),f=setup(()=>held.promise);await f.actor.start();runtime.players[0].emit(snapshot("b",12000),108000);await spin();
  await f.actor.stop("stop");held.resolve({accepted:true,applied:true,zone:zone("b")});await spin();
  expect(f.adopt).not.toHaveBeenCalled();expect(f.actor.active).toBe(false);expect(runtime.players[0].stop).toHaveBeenCalledTimes(1);
});
it("network retries use the identical immutable request even after later frame callbacks",async()=>{
  vi.useFakeTimers();let attempts=0;const f=setup(async()=>{if(++attempts===1)throw new TypeError("network");return {accepted:true,applied:true,zone:zone("b")};});
  await f.actor.start();runtime.players[0].emit(snapshot("b",12000),108000);await spin();
  runtime.players[0].emit({sources:[{id:"b",frames:24000,buffered:48000},{id:"c",frames:0,buffered:48000}],ended:[],spans:[{id:"b",start:12000,end:24000,endFrame:120000}],frames:120000,outputFrames:120000,underrunFrames:0,finished:false},120000);
  await vi.advanceTimersByTimeAsync(1000);await spin();expect(f.bodies).toHaveLength(2);expect(f.bodies[1]).toEqual(f.bodies[0]);await f.actor.stop();
});
it("permanent authorization failure stops output without scheduling network retries",async()=>{
  vi.useFakeTimers();const f=setup(async()=>{throw new ApiError(403,"forbidden","different owner");});await f.actor.start();runtime.players[0].emit(snapshot("b",12000),108000);await spin();
  expect(f.actor.active).toBe(false);expect(f.error).toHaveBeenCalledTimes(1);await vi.advanceTimersByTimeAsync(30000);expect(f.bodies).toHaveLength(1);
});

it("历史部分 DSP 配置不会在连续播放键生成时崩溃",()=>{
 const partial={...zone(),dsp:{enabled:false}} as Zone;
 expect(()=>pcmSequenceKey(partial)).not.toThrow();
 expect(pcmSequenceKey(partial)).toBe(pcmSequenceKey(zone()));
});

it("lossless policy never starts AAC after repeated buffer stalls",async()=>{const f=setup(async()=>({accepted:true,zone:zone("b")}));await f.actor.start();for(const [i,frames]of[24000,48000,72000].entries()){runtime.players[0].emit(buffering(frames,128*(i+1)),frames);await spin();}expect(f.api.aacPcmUrl).not.toHaveBeenCalled();expect(runtime.players).toHaveLength(1);expect(f.error).toHaveBeenCalledWith(expect.objectContaining({message:expect.stringContaining("无损")}));await f.actor.stop();});

it("tail additions retain the browser PCM actor and its playing output",async()=>{const f=setup(async()=>({accepted:true,zone:zone("b")}));await f.actor.start();const next={...zone(),updated_at:200,queue:[...tracks,tracks[0]]};expect(f.actor.appended(next)).toBe(true);f.actor.update(next);expect(runtime.players).toHaveLength(1);expect(runtime.players[0].stop).not.toHaveBeenCalled();expect(f.actor.appended({...next,queue:[tracks[1],...tracks]})).toBe(false);await f.actor.stop();});
