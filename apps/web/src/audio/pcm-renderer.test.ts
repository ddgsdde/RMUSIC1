import {describe,expect,it} from "vitest";
import {PcmFramer,PcmRenderer} from "./pcm-renderer";
const stereo=(values:number[])=>Float32Array.from(values.flatMap(value=>[value,-value]));
function block(renderer:PcmRenderer,count:number){const left=new Float32Array(count),right=new Float32Array(count);renderer.render(left,right);return {left:[...left],right:[...right]};}

describe("actual browser PCM render clock",()=>{
  it("extends a complete queue before its real EOF without reopening finished output",()=>{
    const renderer=new PcmRenderer(48000);renderer.begin("a");renderer.enqueue("a",stereo([.5,.25]));renderer.end("a");renderer.seal();renderer.paused=false;
    expect(block(renderer,1).left).toEqual([.5]);renderer.begin("b");renderer.enqueue("b",stereo([.75]));renderer.end("b");renderer.seal();
    expect(block(renderer,2).left).toEqual([.25,.75]);expect(renderer.snapshot().underrunFrames).toBe(0);expect(renderer.finished).toBe(true);expect(()=>renderer.begin("late")).toThrow();
  });
  it("retains contiguous audio-time runs separately across pauses and underflow",()=>{
    const renderer=new PcmRenderer(48000);renderer.begin("a");renderer.enqueue("a",stereo([.5,.5,.5]));renderer.paused=false;block(renderer,2);
    renderer.paused=true;block(renderer,100);renderer.paused=false;block(renderer,3);
    expect(renderer.snapshot().spans).toEqual([{id:"a",start:0,end:2,endFrame:2},{id:"a",start:2,end:3,endFrame:103}]);
  });
  it("applies each source's gain and seek-aware envelope within the boundary block",()=>{
    const renderer=new PcmRenderer(48000);renderer.begin("a",{gain:.5,startFrame:0,durationFrames:2,fadeInFrames:0,fadeOutFrames:0});renderer.enqueue("a",stereo([.5,.5]));renderer.end("a");
    renderer.begin("b",{gain:.25,startFrame:1,durationFrames:3,fadeInFrames:2,fadeOutFrames:0});renderer.enqueue("b",stereo([.5]));renderer.end("b");renderer.seal();renderer.paused=false;
    const output=block(renderer,3);expect(output.left.slice(0,2)).toEqual([.25,.25]);expect(output.left[2]).toBeCloseTo(.125*Math.sin(Math.PI/4),7);
  });
  it("a failed preloaded source waits for the preceding source and never reports its own EOF",()=>{
    const renderer=new PcmRenderer(48000);renderer.begin("a");renderer.enqueue("a",stereo([.5,.25]));renderer.end("a");renderer.begin("b");renderer.fail("b","decode failure");
    expect(renderer.failure).toBeUndefined();renderer.paused=false;expect(block(renderer,4).left).toEqual([.5,.25,0,0]);
    expect(renderer.failure).toEqual({id:"b",error:"decode failure"});const state=renderer.snapshot();expect(state.frames).toBe(2);expect(state.ended.map(source=>source.id)).toEqual(["a"]);expect(state.finished).toBe(false);
  });
  it("crosses two source boundaries inside one audio block without silence or duplicate frames",()=>{
    const renderer=new PcmRenderer(48000);renderer.begin("a");renderer.enqueue("a",stereo([.125,.25,.5]));renderer.end("a");
    renderer.begin("b");renderer.enqueue("b",stereo([.75]));renderer.end("b");renderer.begin("c");renderer.enqueue("c",stereo([.875,.9375]));renderer.end("c");renderer.seal();renderer.paused=false;
    expect(block(renderer,8)).toEqual({left:[.125,.25,.5,.75,.875,.9375,0,0],right:[-.125,-.25,-.5,-.75,-.875,-.9375,0,0]});
    const state=renderer.snapshot();expect(state.frames).toBe(6);expect(state.underrunFrames).toBe(0);expect(state.finished).toBe(true);
    expect(state.ended.map(source=>[source.id,source.frames,source.endFrame])).toEqual([["a",3,3],["b",1,4],["c",2,6]]);expect(renderer.snapshot().ended).toEqual([]);
  });
  it("pause produces no source credit, including a pause before the first decoded buffer",()=>{
    const renderer=new PcmRenderer(48000);renderer.begin("a");expect(block(renderer,128).left.every(value=>value===0)).toBe(true);
    renderer.enqueue("a",stereo([.5,.25,.125]));expect(block(renderer,32).left.every(value=>value===0)).toBe(true);expect(renderer.snapshot().frames).toBe(0);
    renderer.paused=false;expect(block(renderer,1).left).toEqual([.5]);renderer.paused=true;block(renderer,256);expect(renderer.snapshot().sources[0].frames).toBe(1);
    renderer.paused=false;expect(block(renderer,2).left).toEqual([.25,.125]);expect(renderer.snapshot().frames).toBe(3);
  });
  it("buffer exhaustion is not EOF and missing frames never advance the source clock",()=>{
    const renderer=new PcmRenderer(48000);renderer.begin("a");renderer.enqueue("a",stereo([.5]));renderer.paused=false;
    expect(block(renderer,4).left).toEqual([.5,0,0,0]);expect(renderer.snapshot().underrunFrames).toBe(3);expect(renderer.snapshot().ended).toEqual([]);
    renderer.enqueue("a",stereo([.25]));renderer.end("a");renderer.seal();expect(block(renderer,4).left).toEqual([.25,0,0,0]);
    const state=renderer.snapshot();expect(state.frames).toBe(2);expect(state.underrunFrames).toBe(3);expect(state.ended[0].endFrame).toBe(5);
  });
  it("accounts a source consumed before its late EOF without fabricating the following source",()=>{
    const renderer=new PcmRenderer(48000);renderer.begin("a");renderer.enqueue("a",stereo([.5]));renderer.paused=false;block(renderer,1);
    renderer.paused=true;renderer.end("a");renderer.begin("b");renderer.enqueue("b",stereo([.25]));block(renderer,8);
    const state=renderer.snapshot();expect(state.ended).toEqual([{id:"a",frames:1,buffered:0,endFrame:1}]);expect(state.sources[0].frames).toBe(0);
  });
  it("bounds source count and PCM buffering even while output is paused",()=>{
    const renderer=new PcmRenderer(8000);for(let i=0;i<9;i++)renderer.begin(String(i));expect(()=>renderer.begin("overflow")).toThrow();
    renderer.enqueue("0",new Float32Array(32000));expect(()=>renderer.enqueue("0",stereo([1]))).toThrow();expect(()=>renderer.enqueue("1",new Float32Array(1))).toThrow();
    expect(()=>renderer.begin("0")).toThrow();renderer.end("0");expect(()=>renderer.enqueue("0",stereo([1]))).toThrow();
  });
  it("stop discards queued audio without reporting unplayed sources as ended",()=>{
    const renderer=new PcmRenderer(48000);renderer.begin("a");renderer.enqueue("a",stereo([.5,.25]));renderer.paused=false;block(renderer,1);renderer.close();
    expect(block(renderer,3).left).toEqual([0,0,0]);const state=renderer.snapshot();expect(state.frames).toBe(1);expect(state.ended).toEqual([]);expect(state.finished).toBe(true);
  });
});
describe("PCM HTTP framing",()=>{
  it("reassembles every byte split, including mid-float and mid-stereo-frame splits",()=>{
    const expected=stereo([.125,.25,.5,.75]);const raw=new Uint8Array(expected.buffer);
    for(let split=0;split<=raw.length;split++){
      const framer=new PcmFramer();const chunks=[...framer.push(raw.slice(0,split)),...framer.push(raw.slice(split))];framer.finish();
      expect(chunks.flatMap(chunk=>[...chunk])).toEqual([...expected]);
    }
  });
  it("limits transferred frame blocks and rejects a truncated last stereo frame",()=>{
    const framer=new PcmFramer();const chunks=[...framer.push(new Uint8Array(4096*8+8))];expect(chunks.map(chunk=>chunk.length/2)).toEqual([4096,1]);framer.finish();
    const truncated=new PcmFramer();[...truncated.push(new Uint8Array(9))];expect(()=>truncated.finish()).toThrow(/节目帧/);
  });
});

it("ramps live normalization over half a second without replacing buffered frames",()=>{
 const renderer=new PcmRenderer(48000);renderer.begin("current");renderer.enqueue("current",stereo(Array(24010).fill(.5)));renderer.paused=false;
 expect(block(renderer,10).left).toEqual(Array(10).fill(.5));renderer.gain("current",.5,24000);
 const result=block(renderer,24000);expect(result.left[0]).toBeCloseTo(.5,4);expect(result.left.at(-1)).toBe(.25);expect(result.right.at(-1)).toBe(-.25);
 expect(renderer.snapshot().frames).toBe(24010);expect(renderer.snapshot().underrunFrames).toBe(0);
});

it("uses a late measurement from the first frame of an unheard successor",()=>{
  const renderer=new PcmRenderer(48000);
  renderer.begin("current");renderer.enqueue("current",stereo([.5]));renderer.end("current");
  renderer.begin("next",{gain:1,startFrame:0,durationFrames:2,fadeInFrames:0,fadeOutFrames:0});
  renderer.enqueue("next",stereo([.5,.5]));renderer.end("next");
  renderer.gain("next",.5,24000);renderer.seal();renderer.paused=false;
  expect(block(renderer,3).left).toEqual([.5,.25,.25]);
  expect(renderer.snapshot().underrunFrames).toBe(0);
});
