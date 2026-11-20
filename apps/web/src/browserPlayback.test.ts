import {describe,it,expect} from "vitest";
import {PlaybackGeneration,clearAudio,fadeAutomation,fadeGain} from "./browserPlayback";
describe("source replacement",()=>{
 it("clears the old decoder before a new ticket resolves and rejects older completions",async()=>{
  const events:string[]=[];
  const audio={pause:()=>events.push("pause"),removeAttribute:(name:string)=>events.push(`remove ${name}`),load:()=>events.push("load")};
  const generation=new PlaybackGeneration();
  let finishOld!:(url:string)=>void;
  const old=new Promise<string>(resolve=>{finishOld=resolve;});
  const first=generation.begin();
  const completion=old.then(url=>{if(generation.current(first))events.push(url);});
  const second=generation.begin();clearAudio(audio);
  expect(events).toEqual(["pause","remove src","load"]);
  finishOld("stale source");await completion;
  expect(events).not.toContain("stale source");expect(generation.current(second)).toBe(true);
 });
});
describe("fade envelopes",()=>{
 it("has unity bypass and bounded equal-power boundaries including overlapping short songs",()=>{
  expect(fadeGain(0,10)).toBe(1);expect(fadeGain(0,10,2000,2000)).toBe(0);
  expect(fadeGain(1,10,2000,2000)).toBeCloseTo(Math.SQRT1_2,12);
  expect(fadeGain(5,10,2000,2000)).toBe(1);expect(fadeGain(10,10,2000,2000)).toBe(0);
  expect(fadeGain(.5,1,2000,2000)).toBeCloseTo(.1464466094,9);
 });
 it("schedules continuous automation from the current seek position",()=>{
  const points=fadeAutomation(8.5,10,2000,2000);
  expect(points[0].time).toBe(0);expect(points.at(-1)).toEqual({time:1.5,gain:0});
  expect(points.every((point,index)=>point.time>=0&&point.gain>=0&&point.gain<=1&&(!index||point.time>points[index-1].time))).toBe(true);
  expect(fadeAutomation(5,10,0,0)).toEqual([{time:0,gain:1}]);
 });
});
