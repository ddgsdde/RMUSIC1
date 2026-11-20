// @vitest-environment jsdom
import {act} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {useLyricPosition} from "./useLyricPosition";
let host:HTMLDivElement,root:Root,time=0,hidden=false,id=0;
const frames=new Map<number,FrameRequestCallback>();
const owner={};
function Probe({position,playing,remote=true,identity=owner}:{position:number;playing:boolean;remote?:boolean;identity?:unknown}){return <output>{useLyricPosition(position,playing,identity,false,remote)}</output>;}
const render=(position:number,playing=true)=>act(async()=>root.render(<Probe position={position} playing={playing}/>));
async function frame(at:number){time=at;const pending=[...frames.values()];frames.clear();await act(async()=>{for(const callback of pending)callback(at);});}
beforeEach(()=>{
 time=0;hidden=false;frames.clear();vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
 vi.spyOn(performance,"now").mockImplementation(()=>time);
 vi.spyOn(document,"hidden","get").mockImplementation(()=>hidden);
 vi.stubGlobal("requestAnimationFrame",(callback:FrameRequestCallback)=>{frames.set(++id,callback);return id;});
 vi.stubGlobal("cancelAnimationFrame",(key:number)=>frames.delete(key));
 host=document.createElement("div");document.body.append(host);root=createRoot(host);
});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.restoreAllMocks();vi.unstubAllGlobals();});
it("fills between playback reports, caps stale samples and obeys the next authoritative position",async()=>{
 await render(10);await frame(250);expect(Number(host.textContent)).toBeCloseTo(10.25);
 await frame(2600);expect(Number(host.textContent)).toBeCloseTo(11.2);
 await render(12);expect(Number(host.textContent)).toBe(12);
 await frame(2800);expect(Number(host.textContent)).toBeCloseTo(12.2);
});
it("pausing freezes at the actual report and stops the frame loop; hidden readers stop rendering",async()=>{
 await render(5);await frame(400);await render(5.3,false);expect(Number(host.textContent)).toBe(5.3);expect(frames.size).toBe(0);
 await frame(1400);expect(Number(host.textContent)).toBe(5.3);
 await render(7);hidden=true;await act(async()=>document.dispatchEvent(new Event("visibilitychange")));expect(frames.size).toBe(0);
 hidden=false;await act(async()=>document.dispatchEvent(new Event("visibilitychange")));expect(frames.size).toBe(1);
});
it("stops Android presentation when WebView still reports visible in the background",async()=>{
 let visible=true;vi.stubGlobal("RMusicAppearance",{isVisible:()=>visible});
 await render(5);expect(frames.size).toBe(1);
 visible=false;await act(async()=>window.dispatchEvent(new Event("rmusic-activity-visibility")));expect(document.hidden).toBe(false);expect(frames.size).toBe(0);
 visible=true;await act(async()=>window.dispatchEvent(new Event("rmusic-activity-visibility")));expect(frames.size).toBe(1);
});
it("remote reports do not rewind the interpolated reader; a seek still applies immediately",async()=>{
 await render(10);await frame(900);expect(Number(host.textContent)).toBeCloseTo(10.9);
 await render(10.8);expect(Number(host.textContent)).toBeCloseTo(10.9);
 await frame(1100);expect(Number(host.textContent)).toBeCloseTo(11.1);
 await render(4);expect(Number(host.textContent)).toBe(4);
 await render(4,false);expect(Number(host.textContent)).toBe(4);
});
it("local output and a new remote seek timeline accept even a small backwards seek",async()=>{
 await act(async()=>root.render(<Probe position={10} playing remote={false}/>));await frame(300);
 await act(async()=>root.render(<Probe position={10.1} playing remote={false}/>));expect(Number(host.textContent)).toBeCloseTo(10.1);
 await render(20);await frame(500);
 await act(async()=>root.render(<Probe position={20.1} playing identity="seek-instance"/>));expect(Number(host.textContent)).toBeCloseTo(20.1);
});
