import {readFileSync} from "node:fs";
import {afterEach,expect,it,vi} from "vitest";
import {AacResampler,decodeAac,Mp4Boxes} from "./aac-stream";
afterEach(()=>vi.unstubAllGlobals());
it("retains resampling phase and exact duration across arbitrarily split codec packets",()=>{
  const input=new Float32Array([0,0,1,1,0,0,-1,-1]);
  const whole=new AacResampler(48000,192000),expected=[...whole.push(input),...whole.finish()];
  const split=new AacResampler(48000,192000),actual:number[]=[];for(let i=0;i<input.length;i+=2)actual.push(...split.push(input.subarray(i,i+2)));actual.push(...split.finish());
  expect(actual).toEqual(expected);expect(actual).toHaveLength(4*4*2);expect(actual.slice(0,8)).toEqual([0,0,.25,.25,.5,.5,.75,.75]);
});
it("rejects truncated and oversized MP4 boxes without treating them as EOF",()=>{
  const parser=new Mp4Boxes();expect([...parser.push(new Uint8Array([0,0,0,20,109,100,97,116]))]).toEqual([]);expect(()=>parser.finish()).toThrow("不完整");
  expect(()=>[...new Mp4Boxes().push(new Uint8Array([1,0,0,0,109,100,97,116]))]).toThrow("限额");
});
function decoderMock(){
  vi.stubGlobal("EncodedAudioChunk",class {constructor(public value:{timestamp:number;data:Uint8Array}){}});
  vi.stubGlobal("AudioDecoder",class {
    static async isConfigSupported(){return {supported:true};}
    state="configured";configure(){}close(){this.state="closed";}async flush(){}
    constructor(private init:{output:(data:AudioData)=>void}){}
    decode(chunk:{value:{timestamp:number}}){const timestamp=chunk.value.timestamp;this.init.output({timestamp,sampleRate:48000,numberOfChannels:2,numberOfFrames:1024,copyTo:(target:Float32Array)=>target.fill(.25),close:()=>{}} as unknown as AudioData);}
  });
}
it("keeps priming on a nonnegative coding clock instead of normalizing it onto programme zero",async()=>{
  decoderMock();const timestamps:number[]=[];const Decoder=AudioDecoder;
  vi.stubGlobal("AudioDecoder",class extends Decoder {decode(chunk:EncodedAudioChunk){timestamps.push((chunk as unknown as {value:{timestamp:number}}).value.timestamp);super.decode(chunk);}});
  const bytes=new Uint8Array(readFileSync(new URL('./fixtures/aac-113.mp4',import.meta.url)));let done=false;
  await decodeAac(async()=>done?{done:true,value:undefined}:(done=true,{done:false,value:bytes}),192000,new AbortController().signal,async()=>{});
  expect(timestamps).toEqual([0,21333]);
});
for(const frames of [113,96000])it(`clips priming and final padding using the real ${frames}-frame MP4 timeline`,async()=>{
  decoderMock();const bytes=new Uint8Array(readFileSync(new URL(`./fixtures/aac-${frames}.mp4`,import.meta.url)));let offset=0;const output:Float32Array[]=[];
  await decodeAac(async()=>{if(offset===bytes.length)return {done:true,value:undefined};const value=bytes.slice(offset,offset+97);offset+=value.length;return {done:false,value};},192000,new AbortController().signal,async pcm=>{output.push(pcm);});
  expect(output.reduce((n,p)=>n+p.length/2,0)).toBe(frames*4);expect(output.flatMap(p=>[...p]).every(x=>x===.25)).toBe(true);
});
