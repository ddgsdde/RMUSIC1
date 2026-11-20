import {it,expect} from "vitest";
import type {AudioInfo,Zone} from "@r-music/sdk";
import {signalGrade} from "./signalGrade";
const zone={track:{id:"track"},dsp:{enabled:false},volume:1,muted:false} as Zone;
const info={track_id:"track",format:"flac",sample_rate:96000,bit_depth:24,verified:true} as AudioInfo;
const path={exclusive:true,sample_rate:96000,bit_perfect_eligible:true,processing:[]};
it("DSD quality compares original bit clocks and requires a raw DSD transport",()=>{
 const dsd={...info,format:"dsf",sample_rate:11289600,bit_depth:1};
 expect(signalGrade(zone,dsd,undefined,{...path,sample_rate:11289600,sample_type:"dsd_native"}).level).toBe("lossless");
 expect(signalGrade(zone,dsd,undefined,{...path,sample_rate:705600,sample_type:"dsd_dop"}).level).toBe("lossless");
 expect(signalGrade(zone,dsd,undefined,{...path,sample_rate:11289600,sample_type:"integer"}).level).toBe("high");
 expect(signalGrade(zone,dsd,undefined,{...path,sample_rate:11289600,sample_type:"dsd_native",bit_perfect_eligible:false}).level).toBe("high");
});
it("distinguishes lossless, user processing, shared output and lossy transmission",()=>{
 expect(signalGrade(zone,info,undefined,path).level).toBe("lossless");
 expect(signalGrade({...zone,dsp:{...zone.dsp,enabled:true}},info,undefined,path).level).toBe("enhanced");
 expect(signalGrade(zone,info,undefined,{...path,processing:["Fade envelope"],bit_perfect_eligible:false}).level).toBe("enhanced");
 expect(signalGrade(zone,info,undefined,{...path,exclusive:false}).level).toBe("high");
 expect(signalGrade(zone,info,undefined,path,192).level).toBe("low");
 expect(signalGrade(zone,{...info,format:"mp3"},undefined,path).level).toBe("low");
 expect(signalGrade(zone,{...info,track_id:"previous"},undefined,path).level).toBe("unknown");
});

it("shared output still identifies processing without claiming hardware losslessness",()=>{
 const grade=signalGrade({...zone,dsp:{...zone.dsp,enabled:true}},info,undefined,{...path,exclusive:false});
 expect(grade.level).toBe("enhanced");expect(grade.label).toBe("已处理");
 expect(signalGrade(zone,info,undefined,path).reason).toContain("未作硬件逐位验证");
 expect(signalGrade(zone,{...info,format:"mp3"},undefined,path).label).toBe("有损");
});

it("浏览器共享输出和显式同步处理不冒充原样，有损编码仍显示有损",()=>{
 const browser={kind:"browser"} as import("@r-music/sdk").Endpoint;
 expect(signalGrade(zone,info,browser,{...path,exclusive:false,processing:["Core PCM f32le","Web Audio shared mixer"]}).label).toBe("浏览器共享输出");
 expect(signalGrade(zone,info,browser,{...path,processing:["WebCodecs AAC decode"]}).level).toBe("low");
 expect(signalGrade(zone,{...info,format:"mp3"},browser,path).label).toBe("有损");
 expect(signalGrade(zone,info,undefined,{...path,bit_perfect_eligible:false,processing:["Adaptive PCM sinc f64; output/source clocks separated"]}).label).toBe("同步校正");
});

it("Android 原生路径按实际传输识别 Opus 与 Core 转换，不按注册类型误标网页",()=>{
 const endpoint={kind:"browser"} as import("@r-music/sdk").Endpoint;
 const android={...path,output:"PJZ110 · AudioTrack",exclusive:false,bit_perfect_eligible:false};
 const opus=signalGrade(zone,info,endpoint,{...android,processing:["Core Opus transport: 160 kbit/s; lossy; decoder clock 48000 Hz"]});
 expect(opus.label).toBe("有损");expect(opus.reason).toContain("Opus");
 expect(signalGrade(zone,info,endpoint,{...android,processing:["Core FLAC transport: 48000 Hz / 24 bit / 2 ch"]}).label).toBe("格式转换");
});

it("CoreAudio 淡入淡出准确显示处理，未核实原样不冒称兼容转换",()=>{
 const hal={...path,output:"CoreAudio HAL",bit_perfect_eligible:false};
 expect(signalGrade(zone,info,undefined,{...hal,processing:["HAL driver: float 32 bit","Shared fade envelope"]}).reason).toBe("已应用淡入淡出");
 expect(signalGrade(zone,info,undefined,hal).label).toBe("原生输出");
});
