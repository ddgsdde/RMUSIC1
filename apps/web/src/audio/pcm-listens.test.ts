import {expect,it} from "vitest";
import type {Track} from "@r-music/sdk";
import {PcmListen} from "./pcm-listens";
const track={id:"track",title:"节目帧",artist:"qa",duration:120} as Track;
it("credits only rendered source intervals across UTC midnight and excludes pauses and seek offsets",()=>{
  const at=Date.parse("2026-10-14T23:59:50Z"),listen=new PcmListen(track,"source","session",10,5,20);
  expect(listen.flush(true)).toBeUndefined();listen.sample(0,200,at+20000);
  const first=listen.flush(true)!;expect(first.position).toBe(27);expect(first.started_at).toBe(at);
  expect(first.days["2026-10-14"].seconds).toBe(10);expect(first.days["2026-10-15"].seconds).toBe(10);
  expect(first.measured!.segments.map(segment=>[segment.start,segment.end])).toEqual([[7,17],[17,27]]);
  listen.sample(200,210,at+60000);const final=listen.finish("ended",at+60000)!;
  expect(final.days["2026-10-15"].seconds).toBe(11);expect(final.position).toBe(28);
  expect(first.days["2026-10-15"].seconds).toBe(10);expect(final.measured!.segments).toHaveLength(2);
  expect(Number.isInteger(final.started_at)&&Number.isInteger(final.updated_at)).toBe(true);
  listen.sample(210,220,at+61000);expect(listen.finish("stop")).toBeUndefined();
});
it("duplicate and overlapping callbacks cannot add the same programme frames twice",()=>{
  const listen=new PcmListen(track,"source","session",48000,0),at=Date.now();
  listen.sample(0,48000,at+1000);listen.sample(0,48000,at+1500);listen.sample(24000,72000,at+2000);
  const value=listen.finish("interrupted")!;
  expect(Object.values(value.days).reduce((sum,day)=>sum+day.seconds,0)).toBe(1.5);
  expect(value.measured!.segments).toHaveLength(1);expect(value.measured!.segments[0].end).toBe(1.5);
});
