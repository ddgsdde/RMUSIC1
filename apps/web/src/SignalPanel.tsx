import {useState} from "react";
import {SlidersHorizontal,X,Info,ChevronDown} from "lucide-react";
import {SignalLight} from "./SignalLight";
import {normalizeDsp,type AudioInfo,type Endpoint,type Zone} from "@r-music/sdk";
import {IconButton,quality} from "./components";
import {isNative,type NativePlayback} from "./native";
import {signalResolution} from "./signalResolution";
import {signalGrade} from "./signalGrade";
export function SignalPanel({zone,endpoint,info,error,adaptive,nativeState,localOutput,contextRate,close,inline=false,onDsp}:{zone?:Zone;endpoint?:Endpoint;info:AudioInfo|null;error:string;adaptive:number;nativeState:NativePlayback;localOutput:boolean;contextRate:number;close?:()=>void;inline?:boolean;onDsp?:()=>void}){
 const [expanded,setExpanded]=useState(false);
 const track=zone?.track;
 const source=info?.track_id===track?.id?info:null;
 const hqplayer=endpoint?.kind==="hqplayer";
 const report=endpoint?.playback;
 const path=localOutput&&nativeState.track_id===track?.id&&(!zone?.playback_id||!nativeState.core_playback_id||nativeState.core_playback_id===zone.playback_id)?nativeState.signal_path:report?.track_id===track?.id&&(!zone?.playback_id||endpoint?.capabilities?.playback_instance!==true||report?.core_playback_id===zone.playback_id)?report?.signal_path:undefined;
 const remote=path?.processing?.find(stage=>stage.startsWith("Remote FLAC transport:"));
 const coreStream=path?.processing?.find(stage=>stage.startsWith("Core stream conversion:"));
 const nativeCore=path?.processing?.find(stage=>stage.startsWith("Core PCM conversion:"));
 const coreAudio=path?.output?.includes("CoreAudio");
 const pcm=path?.processing?.includes("Core PCM f32le")===true;
 const aacPcm=path?.processing?.includes("WebCodecs AAC decode")===true;
 const asio=endpoint?.kind==="asio"||path?.output?.startsWith("ASIO ")===true;
 const dop=path?.sample_type==="dsd_dop";
 const nativeDsd=path?.sample_type==="dsd_native";
 const decoderStage=path?.processing?.find(stage=>/解码|decode|FFmpeg/i.test(stage));
 const browser=endpoint?.kind==="browser"||localOutput&&!isNative||path?.output?.includes("浏览器");
 const sourceFormat=(source?.format||track?.format||"").toLowerCase();
 const sourceLossy=["mp3","aac","ogg","opus"].includes(sourceFormat)||sourceFormat==="m4a"&&source?.verified===true&&!source.bit_depth;
 const output=path?`${path.output||endpoint?.name} · ${path.sample_rate?nativeDsd?`${path.sample_rate/1000000} MHz 比特时钟`:`${path.sample_rate/1000} kHz${dop?" 载波":""}`:"采样率未报告"} · ${path.bit_depth?`${path.bit_depth} bit ${dop?"DSD/DoP":nativeDsd?"原生 DSD":path.sample_type==="sdm"?"SDM/DSD":path.sample_type==="float"?"浮点":"PCM"}`:"位深未报告"}`:localOutput&&!isNative?"浏览器 → 系统音频设备":`${endpoint?.name||"输出设备"} · 等待播放端报告`;
 const dsp=normalizeDsp(zone?.dsp);
 const resolution=signalResolution(track,source,path,localOutput&&!isNative?contextRate:0);
 const grade=signalGrade(zone,info,endpoint,path,adaptive);
 const sourceGrade=signalGrade(zone?{...zone,dsp:{...dsp,enabled:false},volume:1,muted:false}:undefined,info,undefined,undefined);
 const stages=[
  {name:"音源",level:sourceGrade.level==="high"?"lossless":sourceGrade.level,status:sourceLossy?"有损音源":source?.verified&&["flac","alac","m4a","wav","aiff","dsf","dff"].includes(sourceFormat)?"无损音源":"源文件",text:source?`${source.format&&source.format!=="unknown"?source.format.toUpperCase():"格式未报告"} · ${source.bit_depth?`${source.bit_depth} bit`:"位深未报告"} / ${source.sample_rate?`${source.sample_rate/1000} kHz`:"采样率未报告"}`:quality(track),sub:source?`${source.source==="local"?"本地文件音频属性":`${source.source||track?.source||"在线音源"} · `+(source.quality_label||source.quality||"解析结果")} · ${source.verified?"已校验":"音源提供的属性"}`:"音源未提供完整格式信息"},
  {name:"传输",level:adaptive||aacPcm?"low":pcm||nativeCore?"enhanced":"lossless",status:remote?"FLAC 无损编码":adaptive||aacPcm?"有损转码":nativeCore?"Core 转换":pcm?"格式转换":"原格式",text:remote?remote.replace("Remote FLAC transport:","HTTPS FLAC ·"):nativeCore|| (pcm?"PCM f32le → AudioWorklet":adaptive?`AAC · ${adaptive} kbps · FFmpeg 转码`:hqplayer?"核心 → HQPlayer · 原格式 HTTP 拉流":endpoint?.kind==="airplay"?"核心 → AirPlay 输出适配器":"原格式 HTTP Range · 无转码"),sub:coreStream|| (nativeCore?"Core 已报告转换后的 PCM 格式":pcm?`Core 解码／重采样至 ${contextRate/1000} kHz · 32 位浮点立体声`:aacPcm?`弱网有损转换 · 浏览器连续输出 ${contextRate/1000} kHz`:adaptive?"弱网或浏览器格式兼容转换":endpoint?.kind==="airplay"?"接收格式由 AirPlay 协商":"传输不改变音频采样数据")},
  {name:"DSP",level:hqplayer?"unknown":dsp.enabled?"enhanced":"high",status:hqplayer?"外部管理":dsp.enabled?"已启用":"旁路",text:hqplayer?"由 HQPlayer 管理":dsp?.enabled?`${localOutput?(isNative?"64 位浮点":"32 位浮点"):"由播放端执行"} · 前级 ${dsp.preamp_db} dB`:"已关闭",sub:hqplayer?path?.processing?.filter(stage=>/滤波器|调制器|音量 dB/.test(stage)).join(" · ")||"滤波器与调制器由 HQPlayer 设置":dsp?.enabled?`${dsp.bands.filter(b=>b.enabled).length} 段均衡 · 声道平衡 ${dsp.balance}${contextRate?` · 处理 ${contextRate/1000} kHz`:""}`:"不施加均衡、前级或响度处理"},
  ...(zone?.transitions?.fade_in_ms||zone?.transitions?.fade_out_ms?[{name:"淡入淡出",level:"enhanced",status:path?.processing?.includes("Fade envelope")||localOutput&&!isNative?"已应用":"待生效",text:`曲首 ${(zone.transitions.fade_in_ms||0)/1000} 秒 · 曲尾 ${(zone.transitions.fade_out_ms||0)/1000} 秒`,sub:localOutput&&!isNative?"Web Audio 增益包络":path?.processing?.includes("Fade envelope")?"已应用增益包络":"下次播放生效"}]:[]),
  ...(coreStream?[{name:"Core 格式转换",level:"enhanced",status:["dsf","dff"].includes(sourceFormat)?"DSD → PCM":"格式转换",text:coreStream.replace("Core stream conversion:","音源 → 传输 ·"),sub:"FLAC 编码保持转换后的采样；降采样、降位深不属于原音源逐位无损"}]:[]),
  {name:"解码",level:pcm||aacPcm?"enhanced":decoderStage||nativeDsd||dop||path?"high":"unknown",status:pcm||aacPcm?"格式转换":decoderStage||nativeDsd||dop||path?"已报告":track?browser?"浏览器解码":"设备端解码":"尚未播放",text:nativeDsd?`DSD 原始比特 → ${asio?"ASIO":"ALSA"} 原生 DSD`:dop?"DSD 原始比特 → DoP 封装":pcm?"FFmpeg → PCM f32le → AudioWorklet":aacPcm?"WebCodecs AAC → PCM f32le → AudioWorklet":hqplayer?"HQPlayer 解码":decoderStage|| (path?"原生 PCM 解码":localOutput?(isNative?"原生解码器":"浏览器媒体解码器"):"播放端解码"),sub:pcm||aacPcm?"Web Audio 共享混音，可能重采样；不属于原样输出":hqplayer?"":localOutput&&!isNative?"解码器输出位深未由浏览器公开":path?.processing?.filter(stage=>stage!==decoderStage).join(" · ")||"由目标设备执行解码"},
  {name:"输出",level:path?"high":"unknown",status:browser?"共享混音":path?path.exclusive?"独占输出":"共享输出":"设备端输出",text:output,sub:hqplayer?"DAC／NAA 由 HQPlayer 管理，最终硬件格式待验证":asio?"ASIO 驱动直连 · 不经过 Windows 共享混音器":path?.exclusive?`${coreAudio?"CoreAudio HAL":endpoint?.kind==="alsa"?"ALSA 硬件":"WASAPI"} 独占 · 设备接受的应用格式`:path?.mixer_sample_rate?`共享模式 · 系统混音 ${path.mixer_sample_rate/1000} kHz`:browser?"浏览器无法独占设备或保证硬件逐位原样":"等待设备报告输出格式"},
 ].filter(stage=>stage.name!=="DSP"||hqplayer||dsp.enabled);
 return <section className={`signal-popover ${inline?"inline-signal":""} ${expanded?"signal-expanded":""}`} role={inline?undefined:"dialog"} aria-label="音频链路">
 <header><div className="signal-heading"><SignalLight level={grade.level}/><div><h2>音频链路</h2><span className={`signal-grade ${grade.level}`}>{grade.label}</span></div></div><div className="signal-actions">{onDsp&&endpoint?.capabilities.dsp!==false&&<IconButton label="设置 DSP" onClick={onDsp}><SlidersHorizontal/></IconButton>}{close&&<IconButton label="关闭音频链路" onClick={close}><X/></IconButton>}</div></header>
 {track?<>{resolution&&<p className="signal-note">链路有效规格 · {resolution}</p>}{track.provider_data?.playback_selection && (track.provider_data.playback_selection as {reason?:string}).reason!=="requested"&&<p className="signal-note">{(track.provider_data.playback_selection as {reason?:string}).reason==="preferred"?"已使用你设置的播放首选":"原版本无法读取，已回退到同一录音"}</p>}<ol className="audio-stages">{stages.map(stage=><li key={stage.name}><SignalLight level={stage.level as typeof grade.level}/><div><div className="signal-stage-heading"><strong>{stage.name}</strong><span className={`signal-grade ${stage.level}`}>{stage.status}</span></div><p>{stage.text}</p>{stage.sub&&<small hidden={!expanded}>{stage.sub}</small>}</div></li>)}</ol>
 <footer className="signal-footer"><p className="signal-note"><Info size={13}/>{grade.reason}</p>
 {zone&&endpoint?.output_config?.volume_control!=="fixed"&&endpoint?.output_config?.volume_control!=="device"&&(zone.volume!==1||zone.muted)&&<p className="signal-note">{hqplayer?"HQPlayer 音量":"软件音量"}：{zone.muted?"静音":`${Math.round(zone.volume*100)}% · 采样幅度会改变`}</p>}
 {dsp.enabled&&dsp.normalize&&<p className="signal-note">响度：{track.integrated_lufs!=null?`目标 ${dsp.target_lufs} LUFS · 增益 ${(dsp.target_lufs-track.integrated_lufs).toFixed(1)} dB`:"缺少响度分析，未施加归一化增益"}</p>}
 {error&&<p className="signal-note signal-error">串流参数解析失败：{error}</p>}
 <button className="signal-details" aria-expanded={expanded} onClick={()=>setExpanded(value=>!value)}>{expanded?"收起详情":"处理详情"}<ChevronDown size={13}/></button></footer></>:<p className="muted">开始播放后显示音频链路</p>}
 </section>;
}
