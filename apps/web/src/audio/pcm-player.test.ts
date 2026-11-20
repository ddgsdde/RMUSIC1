import {afterEach,expect,it,vi} from "vitest";
import {PcmPlayer} from "./pcm-player";
vi.mock("./pcm-worklet.ts?worker&url",()=>({default:"qa-worklet.js"}));
afterEach(()=>{vi.useRealTimers();vi.unstubAllGlobals();vi.restoreAllMocks();});
it("times out an open PCM connection only while playback is intended and the context runs",async()=>{
  vi.useFakeTimers();
  class Node {
    port={onmessage:null as null|((event:{data:unknown})=>void),close:vi.fn(),postMessage:(data:{type:string})=>{
      if(data.type==="stop")queueMicrotask(()=>this.port.onmessage?.({data:{type:"stopped",frame:0}}));
    }};disconnect=vi.fn();
  }
  vi.stubGlobal("AudioWorkletNode",Node);
  let source!:ReadableStreamDefaultController<Uint8Array>;
  const body=new ReadableStream<Uint8Array>({start(controller){source=controller;}});
  vi.stubGlobal("fetch",vi.fn(async()=>new Response(body,{headers:{"X-RMusic-PCM":"f32le; rate=48000; channels=2","X-RMusic-Start":"0"}})));
  const context={sampleRate:48000,state:"running",audioWorklet:{addModule:vi.fn(async()=>{})}} as unknown as BaseAudioContext,error=vi.fn();
  const player=await PcmPlayer.create(context,()=>{},error);player.append([{id:"a",url:"http://qa/a"}]);
  await vi.advanceTimersByTimeAsync(20000);expect(error).not.toHaveBeenCalled();
  player.setPaused(false);await vi.advanceTimersByTimeAsync(7900);expect(error).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(1100);expect(error).toHaveBeenCalledWith(expect.objectContaining({retryable:true}));
  expect(()=>source.enqueue(new Uint8Array(8))).toThrow();await player.stop();
});
it("does not start an empty current source using only a later source's buffer",async()=>{
  class Node {
    port={onmessage:null as null|((event:{data:unknown})=>void),close:vi.fn(),postMessage:(data:{type:string})=>{
      if(data.type==="stop")queueMicrotask(()=>this.port.onmessage?.({data:{type:"stopped",frame:0}}));
    }};
    disconnect=vi.fn();
  }
  vi.stubGlobal("AudioWorkletNode",Node);
  let source!:ReadableStreamDefaultController<Uint8Array>;
  const first=new ReadableStream<Uint8Array>({start(controller){source=controller;}}),pcm=new Uint8Array(new Float32Array(48000*.3*2).buffer);
  vi.stubGlobal("fetch",vi.fn(async(url:string)=>new Response(url.endsWith("/a")?first:pcm,{headers:{"X-RMusic-PCM":"f32le; rate=48000; channels=2","X-RMusic-Start":"0"}})));
  const player=await PcmPlayer.create({sampleRate:48000,audioWorklet:{addModule:vi.fn(async()=>{})}} as unknown as BaseAudioContext,()=>{},()=>{});
  let ready=false;void player.ready.then(()=>{ready=true;});player.setPaused(false);player.append([{id:"a",url:"http://qa/a"},{id:"b",url:"http://qa/b"}]);
  for(let i=0;i<60;i++)await Promise.resolve();
  expect(ready).toBe(false);
  source.enqueue(pcm);source.close();await player.ready;await player.stop();
});
it("starts a fully decoded short reservation window without waiting for final queue EOF",async()=>{
  class Node {
    port={onmessage:null as null|((event:{data:unknown})=>void),close:vi.fn(),postMessage:(data:{type:string})=>{
      if(data.type==="stop")queueMicrotask(()=>this.port.onmessage?.({data:{type:"stopped",frame:0}}));
    }};
    disconnect=vi.fn();
  }
  vi.stubGlobal("AudioWorkletNode",Node);
  vi.stubGlobal("fetch",vi.fn(async()=>new Response(new Uint8Array(new Float32Array([.25,-.25]).buffer),{headers:{"X-RMusic-PCM":"f32le; rate=48000; channels=2","X-RMusic-Start":"0"}})));
  const context={sampleRate:48000,audioWorklet:{addModule:vi.fn(async()=>{})}} as unknown as BaseAudioContext;
  const error=vi.fn(),player=await PcmPlayer.create(context,()=>{},error);
  player.setPaused(false);player.append([{id:"a",url:"http://qa/a"},{id:"b",url:"http://qa/b"}]);
  // The Core can have further reservations. These tiny sources cannot fill 250 ms,
  // and waiting for seal here would prevent ever playing/committing the first window.
  await player.ready;expect(error).not.toHaveBeenCalled();await player.stop();
},1000);
it("network lookahead never overfills the two-second render queue or preloads a third track",async()=>{
 class Node {
  static last:Node;port={onmessage:null as null|((event:{data:unknown})=>void),close:vi.fn(),postMessage:(data:{type:string;id?:string;pcm?:Float32Array})=>{
   if(data.type==="data")sent.push(data.pcm!.length/2);
   if(data.type==="stop")queueMicrotask(()=>this.port.onmessage?.({data:{type:"stopped",frame:0}}));
  }};disconnect=vi.fn();constructor(){Node.last=this;}
 }
 const sent:number[]=[];vi.stubGlobal("AudioWorkletNode",Node);
 const urls:string[]=[];vi.stubGlobal("fetch",vi.fn(async(url:string)=>{urls.push(url);return new Response(new Uint8Array(new Float32Array(48000*5*2).buffer),{headers:{"X-RMusic-PCM":"f32le; rate=48000; channels=2","X-RMusic-Start":"0"}});}));
 const player=await PcmPlayer.create({sampleRate:48000,audioWorklet:{addModule:vi.fn(async()=>{})}} as unknown as BaseAudioContext,()=>{},()=>{});
 player.append(["a","b","c"].map(id=>({id,url:`http://qa/${id}`})));await player.ready;
 for(let i=0;i<20;i++)await Promise.resolve();expect(urls).toEqual(["http://qa/a"]);expect(sent.reduce((a,b)=>a+b,0)).toBeLessThanOrEqual(96000);
 Node.last.port.onmessage?.({data:{type:"state",frame:48000,state:{sources:[{id:"a",frames:48000,buffered:48000}],ended:[],spans:[],frames:48000,outputFrames:48000,underrunFrames:0,finished:false}}});
 for(let i=0;i<40;i++)await Promise.resolve();expect(urls).toEqual(["http://qa/a","http://qa/b"]);expect(sent.reduce((a,b)=>a+b,0)).toBeLessThanOrEqual(48000+2*96000);await player.stop();
});
