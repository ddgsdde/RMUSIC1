import type {AudioInfo,Endpoint,Zone} from "@r-music/sdk";
import type {NativePlayback} from "./native";
export type SignalGrade={level:"lossless"|"enhanced"|"high"|"low"|"unknown";label:string;reason:string};
export function signalGrade(zone:Zone|undefined,info:AudioInfo|null,endpoint:Endpoint|undefined,path:NativePlayback["signal_path"]|NonNullable<Endpoint["playback"]>["signal_path"]|undefined,adaptive=0):SignalGrade{
 if(!zone?.track)return {level:"unknown",label:"未播放",reason:""};
 const source=info?.track_id===zone.track.id?info:null;
 const format=(source?.format||zone.track.format)?.toLowerCase();
 const android=path?.output?.includes("AudioTrack");
 const browser=(!android&&endpoint?.kind==="browser")||path?.output?.includes("浏览器")||path?.processing?.includes("Web Audio shared mixer");
 const opus=path?.processing?.some(stage=>/Core Opus transport:/i.test(stage));
 const encoded=opus||path?.processing?.some(stage=>/WebCodecs AAC|AAC.*kbps/i.test(stage));
 const lossy=["mp3","aac","ogg","opus"].includes(format||"")||(format==="m4a"&&source?.verified===true&&!source.bit_depth);
 if(adaptive||encoded||lossy)return {level:"low",label:"有损",reason:opus?"Opus 有损转码":adaptive||encoded?"AAC 有损转码":"有损音源"};
 if(!browser&&path?.processing?.some(stage=>/underrun|continuity lost/i.test(stage)))return {level:"enhanced",label:"输出曾中断",reason:"输出发生过欠载，未通过变速或丢帧追赶；可重新起播恢复连续输出"};
 if(browser&&!lossy)return {level:"enhanced",label:"浏览器共享输出",reason:"PCM 浮点输出经系统混音；不能保证原样，重采样与增益处理会改变采样"};
 if(!source?.verified||!["flac","wav","aiff","alac","m4a","dsf","dff"].includes(format||""))return {level:"unknown",label:"格式信息不足",reason:"当前音源没有提供可核实的完整格式信息"};
 if(endpoint?.kind==="hqplayer")return {level:"high",label:"外部处理",reason:"HQPlayer 管理处理与输出，DAC／NAA 由外部引擎管理"};
 const dsd=["dsf","dff"].includes(format||"");
 const rate=dsd&&path?.sample_type==="dsd_dop"&&path.sample_rate!=null?path.sample_rate*16:path?.sample_rate;
 if(path?.processing?.some(stage=>/Adaptive PCM|sinc f64/.test(stage)))return {level:"enhanced",label:"同步校正",reason:"已显式开启 PCM 变速校正，音频采样会改变"};
 if(path?.processing?.some(stage=>/Core PCM conversion:|Core FLAC transport:|FFmpeg rate conversion|有效位深限制|Device route precision/.test(stage)))return {level:"enhanced",label:"格式转换",reason:"已执行采样率、声道或有效精度转换；以链路实际格式为准"};
 const softwareVolume=endpoint?.output_config?.volume_control!=="device"&&endpoint?.output_config?.volume_control!=="fixed";
 if(zone.dsp?.enabled||path?.processing?.some(stage=>/^(Shared )?fade envelope$/i.test(stage)||stage==="Software volume"||stage.startsWith("Shared DSP"))||softwareVolume&&(zone.volume!==1||zone.muted))return {level:"enhanced",label:"已处理",reason:zone.dsp?.enabled?"已启用 DSP · 以播放端实际反馈为准":path?.processing?.includes("Software volume")||softwareVolume&&(zone.volume!==1||zone.muted)?"软件音量处理":"已应用淡入淡出"};
 if(!path?.exclusive||endpoint?.kind==="airplay"||rate!==source.sample_rate||dsd&&!["dsd_dop","dsd_native"].includes(path.sample_type||""))
  return {level:"high",label:"无损音源",reason:endpoint?.kind==="airplay"?"AirPlay 协商输出 · 接收端控制硬件格式":"无损音源 · 当前输出未满足原样条件"};
 if(path.bit_perfect_eligible)return {level:"lossless",label:"无损",reason:"软件处理链未改变采样；未作硬件逐位验证"};
 return {level:"high",label:"原生输出",reason:"无损音源经原生输出；尚无软件原样判定，未作硬件逐位验证"};
}
