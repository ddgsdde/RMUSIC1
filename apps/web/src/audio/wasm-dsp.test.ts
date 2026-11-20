import {readFileSync} from "node:fs";
import {createHash} from "node:crypto";
import {expect,it,vi} from "vitest";
import type {DspConfig} from "@r-music/sdk";
import {WasmDspRuntime} from "./wasm-dsp";

const module=new WebAssembly.Module(readFileSync(new URL("./rmusic-audio.wasm",import.meta.url)));
it("ships the compiled binary matching its source manifest",()=>{
  const manifest=JSON.parse(readFileSync(new URL("./rmusic-audio.wasm.json",import.meta.url),"utf8"));expect(createHash("sha256").update(readFileSync(new URL("./rmusic-audio.wasm",import.meta.url))).digest("hex")).toBe(manifest.sha256);
  for(const [file,hash] of Object.entries(manifest.sources))expect(createHash("sha256").update(readFileSync(new URL("../../../../"+file,import.meta.url),"utf8").replace(/\r\n/g,"\n")).digest("hex"),`Run python scripts/build_audio_wasm.py for ${file}`).toBe(hash);
});
function config(value:Partial<DspConfig>={}){return new TextEncoder().encode(JSON.stringify({enabled:true,preamp_db:0,balance:0,normalize:false,target_lufs:-18,bands:[],...value}));}
function wav(kernels:number[][],rate=48000,mask=0){
  const channels=kernels.length,frames=kernels[0].length,extensible=mask!==0,format=extensible?40:16,bytes=new ArrayBuffer(28+format+frames*channels*8),view=new DataView(bytes),text=(at:number,value:string)=>[...value].forEach((character,index)=>view.setUint8(at+index,character.charCodeAt(0)));
  text(0,"RIFF");view.setUint32(4,bytes.byteLength-8,true);text(8,"WAVEfmt ");view.setUint32(16,format,true);view.setUint16(20,extensible?0xfffe:3,true);view.setUint16(22,channels,true);view.setUint32(24,rate,true);view.setUint32(28,rate*channels*8,true);view.setUint16(32,channels*8,true);view.setUint16(34,64,true);
  if(extensible){view.setUint16(36,22,true);view.setUint16(38,64,true);view.setUint32(40,mask,true);view.setUint32(44,3,true);new Uint8Array(bytes,48,12).set([0,0,16,0,128,0,0,170,0,56,155,113]);}
  text(20+format,"data");view.setUint32(24+format,frames*channels*8,true);for(let frame=0;frame<frames;frame++)for(let channel=0;channel<channels;channel++)view.setFloat64(28+format+(frame*channels+channel)*8,kernels[channel][frame],true);return bytes;
}
it("executes the shipped WASM with no host imports and preserves f64 original coefficient bits",()=>{
  expect(WebAssembly.Module.imports(module)).toEqual([]);const runtime=new WasmDspRuntime(module),values=[2,-3,-0,Number.MIN_VALUE];const result=runtime.parse(wav([values]));expect(result).toMatchObject({sampleRate:48000,channels:1,frames:4,channelMask:0});expect([...new BigUint64Array(result.kernels[0].buffer)]).toEqual([...new BigUint64Array(new Float64Array(values).buffer)]);
  expect(()=>runtime.parse(wav([[NaN]]))).toThrow();expect(()=>runtime.parse(new ArrayBuffer(0))).toThrow();
});
it("keeps raw gain, source envelope and independent stereo history across arbitrary source blocks and the full tail",()=>{
  const kernels=[Array.from({length:8193},(_,i)=>i===0?2:i===64?-.5:i===4096?-.75:i===8192?.125:0),Array.from({length:8193},(_,i)=>i===0?-3:i===127?.5:i===8192?-.25:0)],input=Float64Array.from({length:2050},(_,i)=>Math.sin(i*.11)*.01),envelope=Float64Array.from({length:1025},(_,i)=>i/1025);
  const runtime=new WasmDspRuntime(module),processor=runtime.processor(config({preamp_db:6.020599913279624,balance:.25}),48000,2,3,wav(kernels,48000,3)),actual:number[]=[];let at=0;
  for(const count of [1,63,64,127,128,257,385]){actual.push(...processor.process(input.subarray(at*2,(at+count)*2),envelope.subarray(at,at+count)));at+=count;}
  expect(at).toBe(1025);expect(processor.tailFrames).toBe(8192);while(processor.tailFrames)actual.push(...processor.drain(128));expect(actual).toHaveLength((1025+8192)*2);
  const expected=new Float64Array(actual.length);for(let frame=0;frame<1025;frame++)for(let channel=0;channel<2;channel++)for(let tap=0;tap<kernels[channel].length;tap++)expected[(frame+tap)*2+channel]+=input[frame*2+channel]*2*(channel===0?.75:1)*envelope[frame]*kernels[channel][tap];
  expect(Math.max(...actual.map((value,index)=>Math.abs(value-expected[index])))).toBeLessThan(1e-11);expect(processor.drain()).toHaveLength(0);processor.close();processor.close();expect(()=>processor.process(input)).toThrow("关闭");
});
it("strictly rejects wrong actual rate, channel layout, unloaded configured FIR and oversize render blocks",()=>{
  const runtime=new WasmDspRuntime(module),bytes=wav([[2],[3]],48000,3);expect(()=>runtime.processor(config(),96000,2,3,bytes)).toThrow("sample rate");expect(()=>runtime.processor(config(),48000,2,0,bytes)).toThrow("layout");
  expect(()=>runtime.processor(config({convolution:{response_ids:["0".repeat(64)]}}),48000,2,3)).toThrow("尚未加载");const processor=runtime.processor(config(),48000,2,3,bytes);expect(()=>runtime.processor(config(),48000,2,3,bytes)).toThrow("already exists");expect(()=>processor.process(new Float64Array(8194))).toThrow("预算");expect(()=>processor.process(new Float64Array(3))).toThrow("长度");expect(()=>processor.drain(-1)).toThrow("预算");processor.close();
});
it("disabled DSP retains sample bits and loudness changes preserve earlier FIR history",()=>{
  const runtime=new WasmDspRuntime(module),bypass=runtime.processor(config({enabled:false}),48000,2,3,wav([[2,-3]])),input=new Float64Array([0,-0,Number.MIN_VALUE,-Number.MIN_VALUE]);expect([...new BigUint64Array(bypass.process(input).buffer)]).toEqual([...new BigUint64Array(input.buffer)]);expect(bypass.tailFrames).toBe(0);bypass.close();
  const processor=runtime.processor(config({normalize:true,target_lufs:-18}),48000,1,0,wav([[2,-3]]),-18);expect([...processor.process(new Float64Array([.1]))]).toEqual([.2]);processor.loudness(-24.020599913279625);expect(processor.process(new Float64Array([.1]))[0]).toBeCloseTo(.1,14);expect(processor.drain()[0]).toBeCloseTo(-.6,14);processor.close();
});
it("retains and drains the longest allowed response without coefficient truncation",()=>{
  const kernel=Array.from({length:262144},(_,index)=>index===0?2:index===262143?-3:0),runtime=new WasmDspRuntime(module),processor=runtime.processor(config(),192000,2,3,wav([kernel],192000));expect([...processor.process(new Float64Array([.25,-.125]))]).toEqual([.5,-.25]);let frames=0,last=new Float64Array();while(processor.tailFrames){last=processor.drain();frames+=last.length/2;}expect(frames).toBe(262143);expect(last.at(-2)).toBeCloseTo(-.75,11);expect(last.at(-1)).toBeCloseTo(.375,11);processor.close();
});
it("runs and returns native Chinese errors without AudioWorklet text codec globals",()=>{
  const enabled=config(),unloaded=config({convolution:{response_ids:["0".repeat(64)]}}),runtime=new WasmDspRuntime(module);
  vi.stubGlobal("TextEncoder",undefined);vi.stubGlobal("TextDecoder",undefined);
  try{const processor=runtime.processor(enabled,48000,2,3,wav([[2,-3]]));expect([...processor.process(new Float64Array([.25,-.125]))]).toEqual([.5,-.25]);processor.close();expect(()=>runtime.processor(unloaded,48000,2,3)).toThrow("卷积响应尚未加载");}finally{vi.unstubAllGlobals();}
});
it("reports unclamped programme and tail peaks and clipping through the ABI",()=>{
  const processor=new WasmDspRuntime(module).processor(config(),48000,2,3,wav([[2,-3]]));expect([...processor.process(new Float64Array([1,-1]))]).toEqual([2,-2]);expect(processor.meter).toEqual({peak:2,clippedSamples:2});expect([...processor.drain()]).toEqual([-3,3]);expect(processor.meter).toEqual({peak:3,clippedSamples:2});processor.close();
});
