import {describe,it,expect} from "vitest";
import {renderToStaticMarkup} from "react-dom/server";
import {SignalPanel} from "./SignalPanel";
import type {Endpoint,Zone} from "@r-music/sdk";
describe("音频链路真实状态",()=>{
 it("separates remote FLAC encoding from actual DSD or precision conversion",()=>{
  const zone={track:{id:"dsd",source:"local",format:"dsf",sample_rate:2822400,bit_depth:1},playback_id:"current",volume:1,muted:false,dsp:{enabled:false}} as Zone;
  const endpoint={id:"mac",name:"Mac",kind:"wasapi",online:true,verified:false,capabilities:{playback_instance:true},playback:{track_id:"dsd",core_playback_id:"current",signal_path:{output:"CoreAudio",sample_rate:44100,channels:2,bit_depth:24,sample_type:"int",exclusive:false,bit_perfect_eligible:false,processing:["Remote FLAC transport: 44100 Hz / 16 bit / 2 ch; buffer up to 15 s / 32 MiB","Core stream conversion: 2822400 Hz / 1 bit / 2 ch → 44100 Hz / 16 bit / 2 ch"]}}} as Endpoint;
  const html=renderToStaticMarkup(<SignalPanel zone={zone} endpoint={endpoint} info={null} error="" adaptive={0} nativeState={{}} localOutput={false} contextRate={0}/>);
  expect(html).toContain("HTTPS FLAC");expect(html).toContain("DSD → PCM");expect(html).not.toContain("原格式 HTTP Range · 无转码");
 });
 it("历史部分 DSP 配置开启后仍能展示链路",()=>{
  const zone={track:{id:"test",format:"wav"},volume:1,muted:false,dsp:{enabled:true}} as Zone;
  expect(()=>renderToStaticMarkup(<SignalPanel zone={zone} info={null} error="" adaptive={0} nativeState={{}} localOutput={false} contextRate={0}/>)).not.toThrow();
 });
 it("shows the reported decoder precision as the decoding stage, not the PCM container depth",()=>{
  const zone={track:{id:"aac",format:"aac"},playback_id:"current",volume:1,muted:false,dsp:{enabled:false}} as Zone;
  const endpoint={id:"android",name:"Android",kind:"browser",online:true,verified:false,capabilities:{playback_instance:true},playback:{track_id:"aac",core_playback_id:"current",signal_path:{output:"AudioTrack",sample_rate:48000,channels:2,bit_depth:32,sample_type:"int",exclusive:false,bit_perfect_eligible:false,processing:["Android 系统输出","Android AAC 解码：PCM16 → PCM32 容器"]}}} as Endpoint;
  const html=renderToStaticMarkup(<SignalPanel zone={zone} endpoint={endpoint} info={null} error="" adaptive={0} nativeState={{}} localOutput={false} contextRate={0}/>);
  expect(html).toContain("<p>Android AAC 解码：PCM16 → PCM32 容器</p>");expect(html).toContain("32 bit PCM");
 });
 it("ALSA native DSD identifies the actual bridge transport",()=>{
  const zone={track:{id:"dsd",source:"local",format:"dsf"},playback_id:"current",volume:1,muted:false,dsp:{enabled:false}} as Zone;
  const endpoint={id:"alsa",name:"ALSA QA",kind:"alsa",online:true,verified:false,capabilities:{},playback:{track_id:"dsd",core_playback_id:"current",signal_path:{output:"ALSA hw:QA",sample_rate:22579200,channels:2,exclusive:true,bit_perfect_eligible:true,bit_depth:1,sample_type:"dsd_native",processing:[]}}} as Endpoint;
  const html=renderToStaticMarkup(<SignalPanel zone={zone} endpoint={endpoint} info={null} error="" adaptive={0} nativeState={{}} localOutput={false} contextRate={0}/>);
  expect(html).toContain("DSD 原始比特 → ALSA 原生 DSD");expect(html).toContain("22.5792 MHz 比特时钟");expect(html).not.toContain("ASIO 原生 DSD");expect(html).not.toContain("DoP");
 });
 it("native DSD reports its bit clock and ASIO transport without PCM or DoP",()=>{
  const zone={track:{id:"dsd",source:"local",format:"dsf"},playback_id:"current",volume:1,muted:false,dsp:{enabled:false}} as Zone;
  const html=renderToStaticMarkup(<SignalPanel zone={zone} info={null} error="" adaptive={0} nativeState={{track_id:"dsd",core_playback_id:"current",signal_path:{output:"ASIO QA",sample_rate:22579200,channels:2,exclusive:true,bit_perfect_eligible:true,bit_depth:1,sample_type:"dsd_native",processing:[]}}} localOutput contextRate={2822400}/>);
  expect(html).toContain("22.5792 MHz 比特时钟");expect(html).toContain("1 bit 原生 DSD");expect(html).toContain("DSD 原始比特 → ASIO 原生 DSD");expect(html).not.toContain("FFmpeg → 32 bit PCM");expect(html).not.toContain("DoP");
 });
 it("DoP reports the carrier and original DSD payload without a PCM decoding claim",()=>{
  const zone={track:{id:"dsd",source:"local",format:"dsf"},playback_id:"current",volume:1,muted:false,dsp:{enabled:false}} as Zone;
  const html=renderToStaticMarkup(<SignalPanel zone={zone} info={null} error="" adaptive={0} nativeState={{track_id:"dsd",core_playback_id:"current",signal_path:{output:"USB DAC",sample_rate:705600,channels:2,exclusive:true,bit_perfect_eligible:true,bit_depth:24,sample_type:"dsd_dop",processing:["DSD 11289600 Hz → DoP 705600 Hz; no PCM conversion"]}}} localOutput contextRate={705600}/>);
  expect(html).toContain("705.6 kHz 载波");expect(html).toContain("DSD/DoP");expect(html).toContain("DSD 原始比特");expect(html).not.toContain("FFmpeg → 32 bit PCM");
 });
 it("ASIO driver output is identified without a WASAPI claim",()=>{
  const zone={track:{id:"test",source:"local",format:"wav"},playback_id:"current",volume:1,muted:false,dsp:{enabled:false}} as Zone;
  const html=renderToStaticMarkup(<SignalPanel zone={zone} info={null} error="" adaptive={0} nativeState={{track_id:"test",core_playback_id:"current",signal_path:{output:"ASIO test driver",sample_rate:768000,channels:2,exclusive:true,bit_perfect_eligible:true,bit_depth:32,sample_type:"integer",processing:[]}}} localOutput contextRate={768000}/>);
  expect(html).toContain("ASIO 驱动直连");expect(html).toContain("768 kHz");expect(html).not.toContain("WASAPI");
 });
 it("continuous AAC clearly reports lossy bandwidth conversion and actual floating-point output",()=>{
  const zone={track:{id:"test",source:"local",format:"wav"},playback_id:"current",volume:1,muted:false,dsp:{enabled:false}} as Zone;
  const html=renderToStaticMarkup(<SignalPanel zone={zone} info={null} error="" adaptive={128} nativeState={{track_id:"test",core_playback_id:"current",signal_path:{output:"浏览器 Web Audio",sample_rate:192000,channels:2,exclusive:false,bit_perfect_eligible:false,bit_depth:32,sample_type:"float",processing:["AAC 128 kbps (network)","WebCodecs AAC decode"]}}} localOutput contextRate={192000}/>);
  expect(html).toContain("AAC · 128 kbps");expect(html).toContain("WebCodecs AAC → PCM f32le");expect(html).toContain("弱网有损转换");expect(html).toContain("32 bit 浮点");expect(html).not.toContain("原格式 HTTP Range · 无转码");expect(html).not.toContain("输出位深未由浏览器公开");
 });
 it("browser continuous PCM reports the actual conversion and AudioContext format",()=>{
  const zone={track:{id:"test",source:"local",format:"wav"},playback_id:"current",volume:1,muted:false,dsp:{enabled:false}} as Zone;
  const html=renderToStaticMarkup(<SignalPanel zone={zone} info={null} error="" adaptive={0} nativeState={{track_id:"test",core_playback_id:"current",signal_path:{output:"浏览器 Web Audio",sample_rate:192000,channels:2,exclusive:false,bit_perfect_eligible:false,bit_depth:32,sample_type:"float",processing:["Core PCM f32le","Web Audio shared mixer"]}}} localOutput contextRate={192000}/>);
  expect(html).toContain("192 kHz");expect(html).toContain("32 bit 浮点");expect(html).toContain("PCM f32le → AudioWorklet");expect(html).not.toContain("原格式 HTTP Range · 无转码");expect(html).not.toContain("浏览器媒体解码器");expect(html).toContain("不属于原样输出");
 });
 it("未选歌曲或设备时正常显示空态",()=>{expect(renderToStaticMarkup(<SignalPanel info={null} error="" adaptive={0} nativeState={{}} localOutput={false} contextRate={0}/>)).toContain("开始播放后");});
 it("显示源采样率位深，浏览器最终输出明确为未知",()=>{
  const zone={track:{id:"test",source:"qqmusic",format:"unknown"},volume:1,muted:false,dsp:{enabled:false}} as Zone;
  const html=renderToStaticMarkup(<SignalPanel zone={zone} info={{track_id:"test",source:"qqmusic",format:"flac",sample_rate:96000,bit_depth:24,verified:true}} error="" adaptive={192} nativeState={{}} localOutput contextRate={0}/>);
  expect(html).toContain("24 bit / 96 kHz");expect(html).toContain("192 kbps");expect(html).toContain("共享混音");expect(html).not.toContain("<strong>DSP</strong>");expect(html).not.toContain("待确认");
 });
 it("HQPlayer reports SDM output without claiming WASAPI or enabling duplicate DSP",()=>{
  const zone={track:{id:"test",source:"local",format:"flac"},volume:1,muted:false,dsp:{enabled:false}} as Zone;
  const endpoint:Endpoint={id:"hq",name:"HQPlayer",online:true,verified:false,kind:"hqplayer",capabilities:{dsp:false},playback:{track_id:"test",signal_path:{output:"HQPlayer → DAC / NAA",sample_rate:11289600,bit_depth:1,sample_type:"sdm",exclusive:false,processing:["HQPlayer 滤波器: poly-sinc"]}}};
  const html=renderToStaticMarkup(<SignalPanel zone={zone} endpoint={endpoint} info={{track_id:"test",source:"local",format:"flac",sample_rate:96000,bit_depth:24,verified:true}} error="" adaptive={0} nativeState={{}} localOutput={false} contextRate={0} onDsp={()=>{}}/>);
  expect(html).toContain("1 bit SDM/DSD");expect(html).toContain("由 HQPlayer 管理");expect(html).not.toContain("设置 DSP");expect(html).not.toContain("WASAPI");
 });
 it("does not show the same track's old output parameters after a new core playback instance",()=>{
  const zone={track:{id:"test"},playback_id:"new",volume:1,muted:false,dsp:{enabled:false}} as Zone;
  const endpoint:Endpoint={id:"hq",name:"HQPlayer",kind:"hqplayer",online:true,verified:false,capabilities:{playback_instance:true,dsp:false},playback:{track_id:"test",core_playback_id:"old",signal_path:{sample_rate:192000,bit_depth:24}}};
  const html=renderToStaticMarkup(<SignalPanel zone={zone} endpoint={endpoint} info={null} error="" adaptive={0} nativeState={{}} localOutput={false} contextRate={0}/>);
  expect(html).not.toContain("192 kHz");expect(html).toContain("等待播放端报告");
 });
});
