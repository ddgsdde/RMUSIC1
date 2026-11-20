// @vitest-environment jsdom
import {act,useState} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import type {RMusicClient,Zone} from "@r-music/sdk";
import {usePcmAudio} from "./usePcmAudio";
const runtime=vi.hoisted(()=>({actors:[] as {frames:number;start:ReturnType<typeof vi.fn>;stop:ReturnType<typeof vi.fn>}[],contexts:[] as {close:ReturnType<typeof vi.fn>}[],stopGate:undefined as Promise<void>|undefined,recover:vi.fn(),rejectStart:false,startLimit:Infinity}));
vi.mock("./pcm-recovery",()=>({recoverPcm:(...args:unknown[])=>runtime.recover(...args)}));
vi.mock("./pcm-sequence",async original=>{
  const actual=await original<typeof import("./pcm-sequence")>();
  return {...actual,PcmSequence:class {
    frames=100;key:string;start=vi.fn(async(_resume?:unknown)=>{if(runtime.rejectStart&&runtime.actors.length<=runtime.startLimit)throw new TypeError("network");});stopped?:Promise<void>;
    stop=vi.fn(()=>{if(!this.stopped){const gate=runtime.stopGate;runtime.stopGate=undefined;this.stopped=(gate||Promise.resolve()).then(()=>{this.frames=250;});}return this.stopped;});
    constructor(_api:unknown,private zone:Zone){this.key=actual.pcmSequenceKey(zone);runtime.actors.push(this);}
    owns(zone:Zone){return !this.stopped&&this.zone.playback_id===zone.playback_id;}
    appended(){return false;}
    projected(){return false;}
    matchesPlan(plan:string){return !this.stopped&&plan==="plan";}
    older(){return false;}
    update(){}
    resumePoint(){return {id:this.zone.playback_id,origin:0,frames:this.frames};}
    requestRetry(){}
  }};
});
vi.mock("./pcm-graph",()=>({PcmGraph:class {analyser={};configure=vi.fn();gain=vi.fn();attach=vi.fn();close=vi.fn();}}));
const api={} as RMusicClient;
function zone():Zone{return {id:"zone",playback_id:"source",track:{id:"track"},queue:[],status:"playing",volume:1,muted:false,dsp:{enabled:false,preamp_db:0,balance:0,normalize:false,target_lufs:-14,bands:[]}} as unknown as Zone;}
let root:Root,host:HTMLDivElement,audio:ReturnType<typeof usePcmAudio>;
function Harness({value,enabled=true,durable=false,client=api}:{value:Zone;enabled?:boolean;durable?:boolean;client?:RMusicClient}){audio=usePcmAudio(client,value,enabled,()=>{},()=>{},durable);return null;}
async function settle(){for(let i=0;i<40;i++)await Promise.resolve();}
beforeEach(()=>{
  runtime.actors=[];runtime.contexts=[];runtime.stopGate=undefined;runtime.rejectStart=false;runtime.startLimit=Infinity;
  runtime.recover.mockReset();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
  vi.stubGlobal("AudioContext",class {state="running";currentTime=0;sampleRate=48000;close=vi.fn(async()=>{this.state="closed";});resume=vi.fn(async()=>{});constructor(){runtime.contexts.push(this);}});
  host=document.createElement("div");document.body.append(host);root=createRoot(host);
});
it("manual play waits for the closed page's persisted recovery before issuing playback",async()=>{
  const value={...zone(),status:"paused" as const};let release!:(value:Zone)=>void;
  runtime.recover.mockResolvedValueOnce(undefined).mockImplementationOnce(()=>new Promise<Zone>(yes=>{release=yes;}));
  await act(async()=>{root.render(<Harness value={value} durable/>);await settle();});expect(runtime.actors).toHaveLength(0);
  let unlocked=false;await act(async()=>{void audio.unlock().then(()=>{unlocked=true;});await settle();});
  expect(runtime.recover.mock.calls.at(-1)?.[3]).toBe(true);expect(unlocked).toBe(false);
  await act(async()=>{release({...value,playback_id:"restored"});await settle();});
  expect(unlocked).toBe(true);expect(runtime.actors).toHaveLength(1);
  runtime.recover.mockResolvedValue(undefined);const otherApi={} as RMusicClient;
  await act(async()=>{root.render(<Harness client={otherApi} value={value} durable/>);await settle();});
  expect(runtime.recover.mock.calls.at(-1)?.[3]).toBe(false);expect(runtime.actors).toHaveLength(1);
});
afterEach(async()=>{await act(async()=>{root.unmount();await settle();});host.remove();vi.useRealTimers();vi.unstubAllGlobals();});
it("bounds failed initial admission to five retries and manual play starts a fresh budget",async()=>{
  vi.useFakeTimers();runtime.rejectStart=true;
  await act(async()=>{root.render(<Harness value={zone()}/>);await settle();});
  for(const delay of [1000,2000,4000,8000,16000])await act(async()=>{await vi.advanceTimersByTimeAsync(delay);await settle();});
  expect(runtime.actors).toHaveLength(6);await act(async()=>{await vi.advanceTimersByTimeAsync(60000);window.dispatchEvent(new CustomEvent("rmusic-browser-registered",{detail:zone().endpoint_id}));await settle();});expect(runtime.actors).toHaveLength(6);
  runtime.rejectStart=false;await act(async()=>{await audio.unlock();await settle();});expect(runtime.actors).toHaveLength(7);
});
it("automatic durable rebindings cannot reset the admission retry budget",async()=>{
  vi.useFakeTimers();runtime.rejectStart=true;runtime.startLimit=8;let ids=0;
  runtime.recover.mockImplementation(async(_api,value:Zone)=>({...value,playback_id:`restored-${++ids}`}));
  function Restored(){const [value,setValue]=useState(zone());audio=usePcmAudio(api,value,true,()=>{},setValue,true);return null;}
  await act(async()=>{root.render(<Restored/>);await settle();});
  for(const delay of [1000,2000,4000,8000,16000])await act(async()=>{await vi.advanceTimersByTimeAsync(delay);await settle();});
  expect(runtime.actors).toHaveLength(6);await act(async()=>{await vi.advanceTimersByTimeAsync(60000);await settle();});expect(runtime.actors).toHaveLength(6);
});
it("DSP rebuild resumes after the final stopped audio frames, not an earlier UI checkpoint",async()=>{
  const value=zone();await act(async()=>{root.render(<Harness value={value}/>);await settle();});
  let release!:()=>void;runtime.stopGate=new Promise<void>(yes=>{release=yes;});
  await act(async()=>{root.render(<Harness value={{...value,dsp:{...value.dsp,enabled:true}}}/>);await settle();});
  expect(runtime.actors).toHaveLength(1);expect(runtime.actors[0].stop).toHaveBeenCalledTimes(1);
  await act(async()=>{release();await settle();});
  expect(runtime.actors).toHaveLength(2);expect(runtime.actors[1].start).toHaveBeenCalledWith({id:"source",origin:0,frames:250});
  expect(runtime.contexts).toHaveLength(1);
});
it("a matching invalidated plan stops old output and retains its final frame position",async()=>{
  const value=zone();await act(async()=>{root.render(<Harness value={value}/>);await settle();});
  let release!:()=>void;runtime.stopGate=new Promise<void>(yes=>{release=yes;});
  await act(async()=>{window.dispatchEvent(new CustomEvent("rmusic-sequence-invalidated",{detail:{zone_id:"zone",plan_id:"other"}}));await settle();});expect(runtime.actors[0].stop).not.toHaveBeenCalled();
  await act(async()=>{window.dispatchEvent(new CustomEvent("rmusic-sequence-invalidated",{detail:{zone_id:"zone",plan_id:"plan"}}));await settle();});expect(runtime.actors[0].stop).toHaveBeenCalledWith("interrupted",true);expect(runtime.actors).toHaveLength(1);
  await act(async()=>{release();await settle();});expect(runtime.actors[1].start).toHaveBeenCalledWith({id:"source",origin:0,frames:250});
});
it("late teardown closes the old context while a newly unlocked output remains open",async()=>{
  const value=zone();await act(async()=>{root.render(<Harness value={value}/>);await settle();});
  let release!:()=>void;runtime.stopGate=new Promise<void>(yes=>{release=yes;});
  await act(async()=>{root.render(<Harness value={value} enabled={false}/>);await settle();});
  await act(async()=>{root.render(<Harness value={value}/>);await settle();});
  await act(async()=>{await audio.unlock();});
  expect(runtime.contexts).toHaveLength(2);
  await act(async()=>{release();await settle();});
  expect(runtime.contexts[0].close).toHaveBeenCalledTimes(1);expect(runtime.contexts[1].close).not.toHaveBeenCalled();
  expect(runtime.actors).toHaveLength(2);
});
