import {equalizerNode} from "./audio/parametric-eq";
import {listen} from "@tauri-apps/api/event";
import {usePresentationVisible} from "./presentationVisibility";
import { useEffect, useRef, useState } from "react";
import type { AudioInfo, Endpoint, RMusicClient, Zone } from "@r-music/sdk";
import {ApiError} from "@r-music/sdk";
import {
  invokeNative,
  isNative,
  listenNative,
  nativeOutput,
  nativeProfile,
  type NativePlayback,
} from "./native";
import { browserCache } from "./browserCache";
import { playbackEndBody } from "./playbackEnd";
import {clearAudio,fadeAutomation,PlaybackGeneration} from "./browserPlayback";
import {clientId} from "./clientId";
import {usePcmAudio} from "./audio/usePcmAudio";
export function useAudio(
  api: RMusicClient,
  zone: Zone | undefined,
  endpoints: Endpoint[],
  report: (message: string) => void,
  onRecoveredZone: (zone:Zone)=>void,
  connected = true,
  onRecoveryError?: (error:unknown)=>void,
) {
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const contextRef = useRef<AudioContext | null>(null);
  const sourceRef = useRef<MediaElementAudioSourceNode | null>(null);
  const normalizationGain=useRef<GainNode|null>(null);
  const nodesRef = useRef<AudioNode[]>([]);
  const analyserRef = useRef<AnalyserNode | null>(null);
  const currentRef = useRef(zone);
  const [position, setPosition] = useState(0);
  const positionRef = useRef(0);
  const [playing, setPlaying] = useState(false);
  const [preparing,setPreparing] = useState(false);
  const [adaptive, setAdaptive] = useState(0);
  const adaptiveRef = useRef(0);
  const [adaptiveStart, setAdaptiveStart] = useState(0);
  const offsetRef = useRef(0);
  const loadedPlayback = useRef<
    { zoneId: string; trackId: string; id: string;corePlaybackId?:string|null;position:number } | undefined
  >(undefined);
  const browserGeneration = useRef(0);
  const sourceGeneration=useRef(new PlaybackGeneration());
  const pendingSeek=useRef<number|undefined>(undefined);
  const fadeNode=useRef<GainNode|null>(null);
  const handledEnds = useRef(new Set<string>());
  const offline = useRef(false);
  const [nativeState, setNativeState] = useState<NativePlayback>({});
  const [nativeGroup,setNativeGroup]=useState(false);
  const nativePlayback=useRef<{id:string;trackId:string;zoneId?:string;corePlaybackId?:string|null;started:boolean}|undefined>(undefined);
  const nativeSnapshot=useRef<NativePlayback>({});
  const [sourceInfo,setSourceInfo]=useState<AudioInfo|null>(null);
  const [sourceError,setSourceError]=useState("");
  const [contextSampleRate,setContextSampleRate]=useState(0);
  const lastProgress = useRef(0);
  const stalls = useRef<number[]>([]);
  const lastStallPosition=useRef(-1);
  const playableCore=useRef<string|undefined>(undefined);
  const sourceProbe=useRef<AbortController|undefined>(undefined);
  const ignoreWaitingUntil = useRef(0);
  const visible=usePresentationVisible();
  const endpoint = endpoints.find((item) => item.id === zone?.endpoint_id);
  const local = endpoint?.kind === "browser"||isNative&&endpoint?.capabilities.native_group_v1===true;
  const portableSession=isNative&&endpoints.some(e=>e.capabilities.native_client_v1===true);
  const clientOwned=isNative&&endpoint?.capabilities.native_client_v1===true;
  const continuous=!isNative&&local&&endpoint?.capabilities.continuous_pcm===true;
  const pcm=usePcmAudio(api,zone,continuous,report,onRecoveredZone,endpoint?.capabilities.continuous_recovery===true);
  const localRef=useRef(local);localRef.current=local;
  const connectedRef=useRef(connected);connectedRef.current=connected;
  const recoveredRef=useRef(onRecoveredZone);recoveredRef.current=onRecoveredZone;
  const recoveryErrorRef=useRef(onRecoveryError);recoveryErrorRef.current=onRecoveryError;
  const recoveryApi=useRef(api);recoveryApi.current=api;
  const recovery=useRef<{zoneId:string;trackId:string;coreId:string;position:number;attempts:number;timer?:ReturnType<typeof setTimeout>;controller?:AbortController;exhausted?:boolean}|undefined>(undefined);
  const stalled=useRef<{position:number;timer:ReturnType<typeof setTimeout>}|undefined>(undefined);
  function cancelRecovery(){
    sourceProbe.current?.abort();sourceProbe.current=undefined;
    clearTimeout(stalled.current?.timer);stalled.current=undefined;
    clearTimeout(recovery.current?.timer);recovery.current?.controller?.abort();recovery.current=undefined;
  }
  function recover(position:number){
    const z=currentRef.current;
    if(!localRef.current||offline.current||!z?.track||!z.playback_id||z.status==="stopped"||!Number.isFinite(position)||position<0)return false;
    let pending=recovery.current;
    if(!pending){pending={zoneId:z.id,trackId:z.track.id,coreId:z.playback_id,position,attempts:0};recovery.current=pending;report("音源连接中断，正在尝试续播。");}
    if(pending.timer||pending.controller||pending.exhausted)return true;
    pending.position=Math.max(pending.position,position);setPosition(pending.position);
    clearTimeout(stalled.current?.timer);stalled.current=undefined;
    audioRef.current?.pause();setPlaying(false);setPreparing(true);
    const current=()=>recoveryApi.current===api&&recovery.current===pending&&!offline.current&&localRef.current&&currentRef.current?.id===pending.zoneId&&currentRef.current.track?.id===pending.trackId&&currentRef.current.playback_id===pending.coreId&&currentRef.current.status!=="stopped";
    const schedule=()=>{
      if(!current())return;
      if(pending.attempts>=5){pending.exhausted=true;setPreparing(false);report("音源连接仍未恢复，点击播放重试。");return;}
      pending.timer=setTimeout(async()=>{
        pending.timer=undefined;if(!current())return;pending.attempts++;
        const controller=new AbortController();pending.controller=controller;
        const timeout=setTimeout(()=>controller.abort(),10000);
        try{
          const fresh=await api.request<Zone>(`/zones/${encodeURIComponent(pending.zoneId)}`,{signal:controller.signal});
          if(!current()||controller.signal.aborted)return;
          if(fresh.id!==pending.zoneId||fresh.track?.id!==pending.trackId||fresh.playback_id!==pending.coreId||fresh.status==="stopped"){
            cancelRecovery();setPreparing(false);if(fresh.id===pending.zoneId)recoveredRef.current(fresh);return;
          }
          const position=pendingSeek.current??pending.position;
          const latest=currentRef.current!;
          if(adaptiveRef.current)setAdaptiveStart(position);
          recoveredRef.current({...(latest.updated_at>fresh.updated_at?latest:fresh),position});setOnlineRevision(value=>value+1);
        }catch(error){
          if(!current())return;
          if(error instanceof ApiError&&[401,403,404].includes(error.status)){cancelRecovery();setPreparing(false);if(recoveryErrorRef.current)recoveryErrorRef.current(error);else report(error.message);return;}
          schedule();
        }finally{clearTimeout(timeout);if(recovery.current===pending)pending.controller=undefined;}
      },Math.min(1000*2**pending.attempts,30000));
    };
    schedule();return true;
  }
  function progressResumed(position:number){
    if(stalled.current&&position>stalled.current.position+.05){clearTimeout(stalled.current.timer);stalled.current=undefined;}
    const pending=recovery.current;
    if(pending&&!pending.timer&&!pending.controller&&!pending.exhausted&&position>pending.position+3)cancelRecovery();
  }
  useEffect(()=>{
    if(!isNative)return;
    const changed=()=>{
      const pending=recovery.current;
      if(!pending || pending.controller || currentRef.current?.status!=="playing")return;
      clearTimeout(pending.timer);pending.timer=undefined;pending.exhausted=false;pending.attempts=0;
      recover(pending.position);
    };
    window.addEventListener("rmusic-network-change",changed);
    return()=>window.removeEventListener("rmusic-network-change",changed);
  },[api,local,zone?.id]);
  useEffect(()=>()=>cancelRecovery(),[api,local,zone?.id,zone?.track?.id,zone?.playback_id,zone?.status==="stopped"]);
  const dspKey = JSON.stringify(zone?.dsp);
  const fadeKey=JSON.stringify(zone?.transitions);
  const nativeFadeKey=endpoint?.capabilities.continuous_pcm===true?fadeKey:"";
  function playAudio(audio:HTMLAudioElement){
    const loaded=loadedPlayback.current;
    const failed=(error:Error)=>{
      if(!["AbortError","NotSupportedError"].includes(error.name)&&loaded&&loaded===loadedPlayback.current&&audio===audioRef.current&&!offline.current&&currentRef.current?.status==="playing")
        report(error.name==="NotAllowedError"?"点击播放以允许浏览器输出声音。":`音频播放失败：${error.message}`);
    };
    contextRef.current?.resume().catch(failed);
    audio.play().catch(failed);
  }
  function scheduleFade(){
    const audio=audioRef.current,context=contextRef.current,node=fadeNode.current,z=currentRef.current;
    if(!audio||!context||!node)return;
    const points=fadeAutomation(audio.currentTime+offsetRef.current,z?.track?.duration||audio.duration,z?.transitions?.fade_in_ms,z?.transitions?.fade_out_ms);
    node.gain.cancelScheduledValues(context.currentTime);
    node.gain.setValueAtTime(points[0]?.gain??1,context.currentTime);
    if(!audio.paused)for(const point of points.slice(1))node.gain.linearRampToValueAtTime(point.gain,context.currentTime+point.time);
  }
  const [outputRevision, setOutputRevision] = useState(0);
  const [onlineRevision, setOnlineRevision] = useState(0);
  useEffect(()=>{
    let active=true;setSourceInfo(null);setSourceError("");
    if(local&&zone?.track&&!offline.current)api.get<AudioInfo>(`/library/tracks/${encodeURIComponent(zone.track.id)}/audio-info`).then(info=>{if(active)setSourceInfo(info);}).catch(cause=>{if(active)setSourceError(cause.message);});
    return()=>{active=false;};
  },[api,zone?.track?.id,zone?.playback_id,local,onlineRevision,outputRevision]);
  currentRef.current = zone;
  positionRef.current = position;
  adaptiveRef.current = adaptive;
  useEffect(() => {
    if (isNative||continuous) return;
    const audio = new Audio();
    audio.preload = "metadata";
    audio.crossOrigin = "anonymous";
    audioRef.current = audio;
    const ready=()=>{const z=currentRef.current,loaded=loadedPlayback.current;if(!offline.current&&z?.track&&loaded?.corePlaybackId===z.playback_id&&loaded?.trackId===z.track.id&&audio.readyState>=3)void api.post(`/zones/${z.id}/ready`,{track_id:z.track.id,playback_id:z.playback_id}).catch(()=>{});};
    audio.addEventListener("canplay",ready);
    const time = () => {
      const z = currentRef.current;
      const loaded = loadedPlayback.current;
      if (
        audio !== audioRef.current ||
        !z?.track ||
        loaded?.zoneId !== z.id ||
        loaded.trackId !== z.track.id || loaded.corePlaybackId!==z.playback_id
      ) return;
      const position = audio.currentTime + offsetRef.current;
      loaded.position=position;
      progressResumed(position);
      setPosition(position);
      if (z && !offline.current && Date.now() - lastProgress.current > 5000) {
        lastProgress.current = Date.now();
        api.post(`/zones/${z.id}/progress`, {
          track_id: loaded.trackId,
          ...(loaded.corePlaybackId?{playback_id:loaded.corePlaybackId}:{}),
          position,
        }).catch(() => {});
      }
    };
    const end = () => {
      const z = currentRef.current;
      const loaded = loadedPlayback.current;
      if (audio !== audioRef.current || !audio.ended || !z || offline.current || loaded?.zoneId !== z.id || loaded.corePlaybackId!==z.playback_id) return;
      if(audio.error?.code===2||recovery.current?.timer||recovery.current?.controller||recovery.current?.exhausted)return;
      const body = playbackEndBody(
        handledEnds.current,
        z.track?.id,
        loaded.trackId,
        loaded.id,
        audio.currentTime + offsetRef.current,
      );
      if (body)
        api.post<Zone>(`/zones/${z.id}/ended`, {...body,...(loaded.corePlaybackId?{playback_id:loaded.corePlaybackId}:{})})
          .then((result) => {
            if (loadedPlayback.current===loaded&&!result.playback_id&&result.status === "playing" && result.track?.id === body.track_id)
              setOnlineRevision((value) => value + 1);
          })
          .catch((e) => {if(loadedPlayback.current===loaded)report(e.message);});
    };
    const error = () => {
      if (!audio.src||!audio.error) return;
      if(audio.error.code===1)return;
      const z=currentRef.current,loaded=loadedPlayback.current;
      if(offline.current||!z?.track||loaded?.zoneId!==z.id||loaded.trackId!==z.track.id||loaded.corePlaybackId!==z.playback_id)return;
      const position=Math.max(audio.currentTime+offsetRef.current,loaded.position);
      const compatibility=()=>report(`当前浏览器无法直接解码此格式：${audio.error?.message||"请使用支持 PCM 的浏览器或原生客户端"}。无损策略不会自动转为 AAC。`);
      if(audio.error.code===2||!connectedRef.current||navigator.onLine===false||audio.error.code===4&&loaded.corePlaybackId&&playableCore.current===loaded.corePlaybackId){
        sourceProbe.current?.abort();sourceProbe.current=undefined;
        if(!recover(position))report("音源连接中断，请点击播放重试。");return;
      }
      if(audio.error.code===4){
        sourceProbe.current?.abort();const controller=new AbortController();sourceProbe.current=controller;
        const timeout=setTimeout(()=>controller.abort(),5000);
        const current=()=>loaded===loadedPlayback.current&&!offline.current&&sourceProbe.current===controller;
        fetch(audio.src,{method:"HEAD",signal:controller.signal,credentials:"omit",redirect:"error"}).then(response=>{
          if(!current())return;
          if(!response.ok){if(!recover(position))report("音源连接中断，请点击播放重试。");}
          else if(!recovery.current&&z.playback_id)recover(position);
          else compatibility();
        }).catch(()=>{if(current())recover(position);}).finally(()=>{clearTimeout(timeout);if(sourceProbe.current===controller)sourceProbe.current=undefined;});return;
      }
      compatibility();
    };
    const waiting = () => {
      const context=contextRef.current,node=fadeNode.current,z=currentRef.current;
      const loaded=loadedPlayback.current;
      if(!z||loaded?.zoneId!==z.id||loaded.trackId!==z.track?.id||loaded.corePlaybackId!==z.playback_id)return;
      if(context&&node){node.gain.cancelScheduledValues(context.currentTime);node.gain.setValueAtTime(fadeAutomation(audio.currentTime+offsetRef.current,z?.track?.duration||audio.duration,z?.transitions?.fade_in_ms,z?.transitions?.fade_out_ms)[0]?.gain??1,context.currentTime);}
      const now = Date.now();
      if (audio.seeking || now < ignoreWaitingUntil.current) return;
      if(!stalled.current&&z.status==="playing"){
        const position=audio.currentTime+offsetRef.current;
        stalled.current={position,timer:setTimeout(()=>{stalled.current=undefined;if(loaded===loadedPlayback.current&&currentRef.current?.status==="playing"&&audio.currentTime+offsetRef.current<=position+.05)recover(position);},8000)};
      }
      if(!connectedRef.current||navigator.onLine===false)return;
      const position=audio.currentTime+offsetRef.current;
      if(lastStallPosition.current>=0&&position-lastStallPosition.current<.25)return;
      lastStallPosition.current=position;
      stalls.current = [...stalls.current.filter((t) => now - t < 60000), now];
      if (stalls.current.length >= 3 && adaptiveRef.current !== 128) {
        stalls.current = [];
        report("网络缓冲不足，仍保持原音质；请增加缓冲或使用原生客户端，未转为 AAC。");
      }
    };
    audio.addEventListener("timeupdate", time);
    audio.addEventListener("ended", end);
    audio.addEventListener("error", error);
    audio.addEventListener("waiting", waiting);
    audio.addEventListener("playing",()=>{const loaded=loadedPlayback.current;if(loaded)playableCore.current=loaded.corePlaybackId??loaded.id;scheduleFade();});
    audio.addEventListener("seeked",scheduleFade);
    audio.addEventListener("play", () => {setPlaying(true);scheduleFade();});
    audio.addEventListener("pause", () => {setPlaying(false);scheduleFade();});
    return () => {
      audio.removeEventListener("canplay",ready);
      clearAudio(audio);
      audioRef.current = null;
      contextRef.current?.close();
      contextRef.current = null;
      sourceRef.current = null;
      fadeNode.current=null;
    };
  }, [api, report,continuous]);
  useEffect(() => {
    const mark = (event:Event) => {
      cancelRecovery();
      offline.current = true;
      sourceGeneration.current.begin();
      loadedPlayback.current=undefined;
      const detail=(event as CustomEvent<{playbackId?:string;trackId?:string}>).detail;
      nativePlayback.current=detail?.playbackId&&detail.trackId?{id:detail.playbackId,trackId:detail.trackId,started:true}:undefined;
      nativeSnapshot.current={};setNativeState({});setPosition(0);
      if(audioRef.current)clearAudio(audioRef.current);
      setPlaying(false);
      setPreparing(false);
      const z = currentRef.current;
      if (z?.status === "playing")
        api.post(`/zones/${z.id}/pause`).catch(() => {});
    };
    window.addEventListener("rmusic-offline", mark);
    const stop=()=>{cancelRecovery();offline.current=true;sourceGeneration.current.begin();loadedPlayback.current=undefined;nativePlayback.current=undefined;nativeSnapshot.current={};if(audioRef.current)clearAudio(audioRef.current);setNativeState({});setPlaying(false);setPreparing(false);setPosition(0);};
    window.addEventListener("rmusic-offline-stop",stop);
    return () => {window.removeEventListener("rmusic-offline", mark);window.removeEventListener("rmusic-offline-stop",stop);};
  }, [api]);
  useEffect(() => {
    const change = () => {if(currentRef.current?.status!=="playing"&&currentRef.current?.status!=="paused")setOutputRevision((v) => v + 1);};
    window.addEventListener("rmusic-output", change);
    return () => window.removeEventListener("rmusic-output", change);
  }, []);
  useEffect(() => {
    if (isNative) return;
    const protect = () =>
      browserCache.protect(
        playing && !offline.current ? zone?.track?.id : undefined,
      );
    protect();
    const timer = setInterval(protect, 10000);
    return () => {
      clearInterval(timer);
      browserCache.protect();
    };
  }, [playing, zone?.track?.id]);
  useEffect(() => {
    setAdaptive(0);
    setAdaptiveStart(0);
    stalls.current = [];
  }, [zone?.track?.id]);
  useEffect(() => {
    if (isNative||continuous) return;
    const audio = audioRef.current;
    if (!audio) return;
    if(offline.current)return;
    let active = true;
    const controller=new AbortController();
    const generation=sourceGeneration.current.begin();
    sourceProbe.current?.abort();sourceProbe.current=undefined;
    clearTimeout(stalled.current?.timer);stalled.current=undefined;lastStallPosition.current=-1;
    stalls.current=[];
    pendingSeek.current=undefined;
    loadedPlayback.current=undefined;
    clearAudio(audio);
    setPreparing(false);
    if (!local || !zone?.track || zone.status==="stopped") {
      loadedPlayback.current = undefined;
      setPosition(zone?.position || 0);
      return;
    }
    offline.current = false;
    setPreparing(true);
    window.dispatchEvent(new Event("rmusic-online"));
    browserCache.rememberTrack(zone.track,api.baseUrl).catch(() => {})
      .then(()=>{controller.signal.throwIfAborted();return api.mediaUrl(zone.track!.id,controller.signal);})
      .then((original) => {
        if (!active||!sourceGeneration.current.current(generation)) return;
        const current=currentRef.current;
        if(!current?.track||current.id!==zone.id||current.track.id!==zone.track?.id||current.playback_id!==zone.playback_id||current.status==="stopped")return;
        const target=pendingSeek.current??current.position??0;
        const start = adaptive ? (pendingSeek.current!==undefined||current.position!==zone.position?target:adaptiveStart) : 0;
        let url = original;
        if (adaptive) {
          const parsed = new URL(original);
          parsed.pathname = parsed.pathname.replace("/media/", "/transcode/");
          parsed.searchParams.set("bitrate", String(adaptive));
          parsed.searchParams.set("start", String(start));
          url = parsed.href;
        }
        offsetRef.current = start;
        loadedPlayback.current = {
          zoneId: current.id,
          trackId: current.track.id,
          id: `browser-${++browserGeneration.current}`,
          corePlaybackId:current.playback_id,
          position:target,
        };
        audio.src = url;
        audio.volume=current.volume;
        audio.muted=current.muted;
        if (!adaptive) audio.currentTime = target;
        pendingSeek.current=undefined;
        setPosition(adaptive ? start : target);
        setPreparing(false);
        if (current.status === "playing") playAudio(audio);
      })
      .catch((e) => {if(active&&sourceGeneration.current.current(generation)){
        if((e instanceof ApiError&&e.status>=500||/Failed to fetch|NetworkError|Load failed|error sending request|无法连接核心|MEDIA_SOURCE_UNAVAILABLE/i.test(e.message))&&recover(pendingSeek.current??currentRef.current?.position??0))return;
        setPreparing(false);report(e.message);
      }});
    return () => {
      active = false;
      controller.abort();
    };
  }, [
    api,
    zone?.id,
    zone?.track?.id,
    zone?.playback_id,
    zone?.status==="stopped",
    local,
    adaptive,
    adaptiveStart,
    onlineRevision,
    continuous,
    report,
  ]);
  useEffect(()=>{
    const expected=nativePlayback.current,measured=zone?.track?.integrated_lufs;
    if(!isNative||!local||nativeGroup||offline.current||!expected||!zone?.playback_id||measured==null)return;
    const peak=zone.track?.true_peak_dbfs??(zone.track?.provider_data as {analysis?:{loudness?:{true_peak_dbfs?:number}}}|undefined)?.analysis?.loudness?.true_peak_dbfs;
    void invokeNative("native_loudness",{nativePlaybackId:expected.id,playbackId:zone.playback_id,measuredLufs:measured,truePeakDbfs:peak}).catch(e=>report(e.message));
  },[zone?.track?.integrated_lufs,zone?.track?.true_peak_dbfs,zone?.playback_id,local,nativeGroup,preparing,report]);
  useEffect(() => {
    if (offline.current) return;
    if (isNative) {
      if (local && zone&&!nativeGroup)
        invokeNative("native_volume", {
          volume: zone.volume,
          muted: zone.muted,
        }).catch((e) => report(e.message));
      return;
    }
    if(continuous)return;
    const audio = audioRef.current;
    if (!audio || !local) return;
    audio.volume = zone?.volume ?? 1;
    audio.muted = zone?.muted ?? false;
    if (zone?.status === "playing" && audio.src) {
      playAudio(audio);
    } else audio.pause();
  }, [zone?.status, zone?.volume, zone?.muted, local, continuous,nativeGroup,report]);
  useEffect(() => {
    if (!local || !zone || offline.current) return;
    // A guarded core creates a new playback ID for every seek. Its delayed progress echo is not a seek command.
    if(zone.playback_id||isNative&&endpoint?.capabilities.continuous_pcm===true)return;
    if (isNative) {
      if (Math.abs(positionRef.current - zone.position) > 2.5)
        invokeNative("native_seek", { position: zone.position }).catch((e) =>
          report(e.message),
        );
      return;
    }
    const audio = audioRef.current;
    if (!audio || audio.readyState === 0) return;
    const actual = audio.currentTime + offsetRef.current;
    if (Math.abs(actual - zone.position) > 2.5) {
      if (adaptive) setAdaptiveStart(zone.position);
      else audio.currentTime = zone.position;
      setPosition(zone.position);
    }
  }, [zone?.position, zone?.updated_at, local, adaptive, endpoint?.capabilities.continuous_pcm, report]);
  useEffect(() => {
    if (isNative||continuous) return;
    const audio = audioRef.current;
    const dsp = zone?.dsp;
    if (!audio || !local || !dsp) return;
    const fading=!!(zone?.transitions?.fade_in_ms||zone?.transitions?.fade_out_ms);
    if (!dsp.enabled && !fading && !contextRef.current) return;
    try {
      let context = contextRef.current;
      if (!context) {
        context = new AudioContext();
        contextRef.current = context;
        setContextSampleRate(context.sampleRate);
        sourceRef.current = context.createMediaElementSource(audio);
        analyserRef.current = context.createAnalyser();
        analyserRef.current.fftSize = 256;
      }
      sourceRef.current?.disconnect();
      nodesRef.current.forEach((node) => node.disconnect());
      nodesRef.current = [];
      let tail: AudioNode = sourceRef.current!;
      if (dsp.enabled) {
        const gain = context.createGain();normalizationGain.current=gain;
        const measured = zone?.track?.integrated_lufs;
        const normalization =
          dsp.normalize && measured != null && Number.isFinite(measured)
            ? Math.min(dsp.target_lufs - measured,zone?.track?.true_peak_dbfs==null?Infinity:-zone.track.true_peak_dbfs)
            : 0;
        gain.gain.value = 10 ** ((dsp.preamp_db + normalization) / 20);
        tail.connect(gain);
        tail = gain;
        nodesRef.current.push(gain);
        for (const band of dsp.bands.filter(b => b.enabled && b.frequency < context.sampleRate / 2)) {
          const filter = equalizerNode(context, band);
          tail.connect(filter);
          tail = filter;
          nodesRef.current.push(filter);
        }
        const balance = context.createStereoPanner();
        balance.pan.value = dsp.balance;
        tail.connect(balance);
        tail = balance;
        nodesRef.current.push(balance);
      }
      const fade=context.createGain();
      tail.connect(fade);tail=fade;fadeNode.current=fade;nodesRef.current.push(fade);
      tail.connect(analyserRef.current!);
      analyserRef.current!.connect(context.destination);
      scheduleFade();
    } catch (e) {
      report(e instanceof Error ? e.message : "无法启用浏览器音频处理");
    }
  }, [dspKey,fadeKey,zone?.track?.id,local,continuous,report]);
  useEffect(()=>{const gain=normalizationGain.current,context=contextRef.current,measured=zone?.track?.integrated_lufs,dsp=zone?.dsp;
    if(isNative||continuous||!gain||!context||!dsp?.enabled||!dsp.normalize||measured==null)return;
    const peak=zone?.track?.true_peak_dbfs;
    const target=10**((dsp.preamp_db+Math.min(dsp.target_lufs-measured,peak==null?Infinity:-peak))/20);const now=context.currentTime;
    gain.gain.cancelScheduledValues(now);gain.gain.setValueAtTime(gain.gain.value,now);gain.gain.linearRampToValueAtTime(target,now+.5);
  },[zone?.track?.integrated_lufs,zone?.track?.true_peak_dbfs,local,continuous]);
  useEffect(()=>{
    if(clientOwned)setPreparing(zone?.buffering===true);
  },[clientOwned,zone?.playback_id,zone?.buffering]);
  useEffect(()=>{
    if(!clientOwned||!visible)return;
    let active=true,remove:(()=>void)|undefined;
    const update=(state:NativePlayback)=>{
      if(!active||state.playback_id!==currentRef.current?.playback_id)return;
      nativeSnapshot.current=state;setNativeState(state);
      if(state.position!==undefined)setPosition(state.position);
      setPlaying(state.status==="playing"&&!state.paused);
      setPreparing(state.status==="preparing"||currentRef.current?.buffering===true);
    };
    void listen<NativePlayback>("native-client-state",event=>update(event.payload)).then(clean=>{if(active)remove=clean;else clean();}).catch(()=>{});
    void invokeNative<NativePlayback>("native_status").then(update).catch(()=>{});
    return()=>{active=false;remove?.();};
  },[clientOwned,visible,zone?.playback_id]);
  function consumeNative(event:NativePlayback){
    const expected=nativePlayback.current,z=currentRef.current;
    if(!expected||event.playback_id!==expected.id)return false;
    const checkpoint=event.core_managed&&event.status==="error"?event.recovery?.find(item=>item.playback_id===expected.corePlaybackId&&item.track_id===expected.trackId):undefined;
    if(checkpoint)event={...event,track_id:checkpoint.track_id,core_playback_id:checkpoint.playback_id,position:checkpoint.position};
    if(!expected||event.playback_id!==expected.id||event.track_id!==expected.trackId)return false;
    if(event.core_managed&&event.core_playback_id!==expected.corePlaybackId)return false;
    if(!offline.current&&(!z||expected.zoneId!==z.id||expected.trackId!==z.track?.id||expected.corePlaybackId!==z.playback_id||z.status==="stopped"))return false;
    const previous=nativeSnapshot.current;
    if(previous.playback_id===event.playback_id&&["ended","error","stopped"].includes(previous.status||"")&&event.status!==previous.status)return false;
    nativeSnapshot.current=event;setNativeState(event);
    if(event.position!==undefined){setPosition(event.position);if(event.status==="playing"&&!event.paused)progressResumed(event.position);}
    setPlaying(!event.paused&&(event.status==="playing"||event.status==="preparing"));
    if(["playing","ended","error","stopped"].includes(event.status||""))setPreparing(false);
    if(event.status==="error"&&(previous.playback_id!==event.playback_id||previous.status!==event.status||previous.error!==event.error)){
      if(event.device_failed)report("音频设备暂不可用，已保留播放位置；设备恢复后点击播放。");
      else if(event.http_status&&[401,403].includes(event.http_status)&&recoveryErrorRef.current)recoveryErrorRef.current(new ApiError(event.http_status,"native_http",event.error||"核心凭证失效"));
      else if(!(event.source_failed||event.connection_failed)||!recover(event.position??positionRef.current))report(event.error||"原生音频输出失败");
    }
    if(event.status==="ended"&&!event.core_managed&&!offline.current&&z){
      const body=playbackEndBody(handledEnds.current,z.track?.id,event.track_id,event.playback_id,event.position);
      if(body)api.post<Zone>(`/zones/${z.id}/ended`,{...body,...(expected.corePlaybackId?{playback_id:expected.corePlaybackId}:{})}).then(result=>{
        if(nativePlayback.current===expected&&!result.playback_id&&result.status==="playing"&&result.track?.id===body.track_id)setOnlineRevision(value=>value+1);
      }).catch(e=>{if(nativePlayback.current===expected)report(e.message);});
    }
    return true;
  }
  useEffect(() => {
    if (!isNative||!local||endpoint?.capabilities.continuous_pcm!==true) return;
    const cancel=(event:Event)=>{const detail=(event as CustomEvent<{zone_id?:string;plan_id?:string}>).detail;
      if(!offline.current&&detail?.zone_id===currentRef.current?.id&&detail?.plan_id)invokeNative("native_sequence_cancel",{zoneId:detail.zone_id,planId:detail.plan_id}).catch(error=>report(error.message));};
    window.addEventListener("rmusic-sequence-invalidated",cancel);
    return()=>window.removeEventListener("rmusic-sequence-invalidated",cancel);
  },[report,local,endpoint?.capabilities.continuous_pcm]);
  useEffect(() => {
    if (!isNative) return;
    let active = true;
    let cleanup: (() => void) | undefined;
    listenNative((event) => {
      if(active)consumeNative(event);
    }, report).then((unlisten) => {
      if (active) cleanup = unlisten;
      else unlisten();
    });
    return () => {
      active = false;
      cleanup?.();
    };
  }, [api, report]);
  useEffect(() => {
    if (!isNative||nativeGroup||portableSession) return;
    if(offline.current)return;
    let active = true;
    const generation=sourceGeneration.current.begin();
    let pendingAdmission:ReturnType<typeof setTimeout>|undefined;
    let pendingNativeRequest:string|undefined;
    nativePlayback.current=undefined;nativeSnapshot.current={};setNativeState({});setPlaying(false);
    pendingSeek.current=undefined;
    setPreparing(false);
    if (!local || !zone?.track || zone.status === "stopped" || endpoint?.capabilities.continuous_recovery===true&&!zone.playback_id) {
      invokeNative<NativePlayback>("native_status").then(state=>{
        if(!active||!sourceGeneration.current.current(generation))return;
        if(state.offline&&state.cache_id&&state.playback_id&&state.track_id){
          offline.current=true;nativePlayback.current={id:state.playback_id,trackId:state.track_id,started:true};consumeNative(state);
        }else if(zone) {
          return invokeNative("native_stop",{preserveRecovery:true}).then(async()=>{
            if(!active||!sourceGeneration.current.current(generation)||!local||!zone?.track||endpoint?.capabilities.continuous_recovery!==true)return;
            const profile=await nativeProfile(api.baseUrl);if(!active||!sourceGeneration.current.current(generation))return;
            pendingNativeRequest=clientId();
            const result=await invokeNative<{zone?:Zone;recovering?:boolean}|null>("native_sequence_restore",{requestId:pendingNativeRequest,profileId:profile.id,endpointId:zone.endpoint_id,output:nativeOutput()}).finally(()=>{pendingNativeRequest=undefined;});
            if(!active||!sourceGeneration.current.current(generation))return;
            if(result?.zone)recoveredRef.current(result.zone);
            else if(result?.recovering)pendingAdmission=setTimeout(()=>setOnlineRevision(value=>value+1),250);
          });
        }
      }).catch(e=>{if(active&&sourceGeneration.current.current(generation))report(e.message);});
      return()=>{active=false;clearTimeout(pendingAdmission);if(pendingNativeRequest)invokeNative("native_request_cancel",{requestId:pendingNativeRequest}).catch(()=>{});};
    }
    const expected={id:clientId(),trackId:zone.track.id,zoneId:zone.id,corePlaybackId:zone.playback_id,started:false};
    nativePlayback.current=expected;setPosition(zone.position||0);
    setPreparing(true);
    nativeProfile(api.baseUrl)
      .then(async (profile) => {
        const current=currentRef.current;
        if (!active || !sourceGeneration.current.current(generation) || !current?.track || current.id!==zone.id || current.track.id!==zone.track?.id || current.playback_id!==zone.playback_id || current.status==="stopped") return;
        offline.current = false;
        window.dispatchEvent(new Event("rmusic-online"));
        expected.started=true;
        if(endpoint?.capabilities.native_group_v1===true){
          const group=await invokeNative<NativePlayback>("native_status");
          if(!active||!sourceGeneration.current.current(generation))return;
          if(group.group_managed){
            setNativeGroup(group.profile_id===profile.id);setPreparing(false);
            if(group.profile_id!==profile.id)pendingAdmission=setTimeout(()=>setOnlineRevision(value=>value+1),250);
            return;
          }
        }
        pendingNativeRequest=expected.id;
        const receipt=await invokeNative<{zone?:Zone;recovering?:boolean;superseded?:boolean;group_managed?:boolean}|undefined>("native_play", {
          requestId:expected.id,
          playbackId:expected.id,
          profileId: profile.id,
          track: current.track,
          position: pendingSeek.current??current.position??0,
          dsp: current.dsp,
          output: nativeOutput(),
          volume: current.volume,
          muted: current.muted,
          paused: current.status !== "playing",
          transitions: current.transitions,
          ...(/Android/i.test(navigator.userAgent)&&current.playback_id?{androidContext:{zone_id:current.id,endpoint_id:current.endpoint_id,playback_id:current.playback_id}}:{}),
          ...(endpoint?.capabilities.continuous_pcm===true&&current.playback_id?{sequence:{zone_id:current.id,endpoint_id:current.endpoint_id,playback_id:current.playback_id}}:{}),
        }).finally(()=>{pendingNativeRequest=undefined;});
        if(active&&sourceGeneration.current.current(generation)&&receipt?.group_managed){setNativeGroup(true);setPreparing(false);return;}
        if(active&&sourceGeneration.current.current(generation)&&(receipt?.recovering||receipt?.superseded)){pendingAdmission=setTimeout(()=>setOnlineRevision(value=>value+1),250);return;}
        if(active&&sourceGeneration.current.current(generation)&&receipt?.zone?.id===current.id){recoveredRef.current(receipt.zone);return;}
        if (active&&sourceGeneration.current.current(generation)&&currentRef.current?.track?.id===zone.track?.id) {
          setPreparing(false);
          await invokeNative("native_pause", {
            paused: currentRef.current?.status !== "playing",
          });
        }
      })
      .catch((e) => {if(active&&sourceGeneration.current.current(generation)){
        if((e instanceof ApiError&&e.status>=500||/Failed to fetch|NetworkError|Load failed|error sending request|无法连接核心|MEDIA_SOURCE_UNAVAILABLE/i.test(e.message))&&recover(pendingSeek.current??currentRef.current?.position??0))return;
        setPreparing(false);report(e.message);
      }});
    return () => {
      active = false;
      clearTimeout(pendingAdmission);
      if(pendingNativeRequest)invokeNative("native_request_cancel",{requestId:pendingNativeRequest}).catch(()=>{});
      if(!offline.current)nativePlayback.current=undefined;
    };
  }, [
    api,
    local,
    nativeGroup,
    clientOwned,
    portableSession,
    zone?.id,
    zone?.track?.id,
    zone?.playback_id,
    zone?.status === "stopped",
    dspKey,
    nativeFadeKey,
    endpoint?.capabilities.continuous_pcm,
    endpoint?.capabilities.continuous_recovery,
    endpoint?.capabilities.native_group_v1,
    isNative&&endpoint?.capabilities.continuous_recovery===true&&!zone?.playback_id&&zone?.status==="playing",
    outputRevision,
    onlineRevision,
    report,
  ]);
  useEffect(() => {
    if (!isNative || !local || !zone || nativeGroup||clientOwned||offline.current || endpoint?.capabilities.continuous_recovery===true&&!zone.playback_id) return;
    invokeNative("native_pause", { paused: zone.status !== "playing" }).catch(
      (e) => report(e.message),
    );
  }, [zone?.status, zone?.playback_id, endpoint?.capabilities.continuous_recovery, local,nativeGroup,clientOwned, report]);
  useEffect(() => {
    if (!isNative || !local||clientOwned||!visible) return;
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      const expected=nativePlayback.current;
      try {
        if (!expected) return;
        const state = await invokeNative<NativePlayback>("native_status");
        if (!active||expected!==nativePlayback.current) return;
        const z = currentRef.current;
        if(state.playback_id!==expected.id){
          if(!offline.current&&state.core_managed&&state.zone_id===expected.zoneId&&state.core_playback_id===expected.corePlaybackId&&["preparing","playing"].includes(state.status||""))setOnlineRevision(value=>value+1);
          return;
        }
        if(!consumeNative(state))return;
        if (state.position !== undefined) {
          if (
            z &&
            !offline.current && !state.core_managed&&
            state.status==="playing"&&!state.paused&&
            Date.now() - lastProgress.current > 5000
          ) {
            lastProgress.current = Date.now();
            api
              .post(`/zones/${z.id}/progress`, {
                track_id: state.track_id,
                ...(expected.corePlaybackId?{playback_id:expected.corePlaybackId}:{}),
                position: state.position,
              })
              .catch(() => {});
          }
        }
      } catch (e) {
        if (active&&expected===nativePlayback.current) report(e instanceof Error ? e.message : "设备状态不可用");
      } finally {
        if (active) timer = setTimeout(poll, 500);
      }
    };
    void poll();
    return () => {
      active = false;
      clearInterval(timer);
    };
  }, [api, local,clientOwned,visible, report]);
  useEffect(()=>{
    if(!isNative||endpoint?.capabilities.native_group_v1!==true){setNativeGroup(false);return;}
    let active=true,busy=false,wasGroup=false;
    const poll=async()=>{if(busy)return;busy=true;
      try{const state=await invokeNative<NativePlayback>("native_status");if(!active||offline.current)return;
        const profile=await nativeProfile(api.baseUrl);if(!active||offline.current)return;
        const owned=state.group_managed===true&&state.profile_id===profile.id;
        setNativeGroup(owned);
        if(owned){setNativeState(state);if(state.zone_id===currentRef.current?.id&&state.position!==undefined)setPosition(state.position);setPreparing(state.status==="preparing");}
        else if(wasGroup){setNativeState({});setPreparing(false);setOnlineRevision(value=>value+1);}
        wasGroup=owned;
      }catch(error){if(active)report(error instanceof Error?error.message:"整组状态不可用");}finally{busy=false;}
    };
    poll();const timer=setInterval(poll,500);return()=>{active=false;clearInterval(timer);};
  },[api,endpoint?.capabilities.native_group_v1,report]);
  async function unlock(resume=true) {
    if(recovery.current?.exhausted){const position=recovery.current.position;cancelRecovery();recover(position);return;}
    if (offline.current) {
      offline.current = false;
      window.dispatchEvent(new Event("rmusic-online"));
      setOnlineRevision((value) => value + 1);
    }
    if (isNative) return;
    if(continuous){await pcm.unlock();return;}
    const audio = audioRef.current;
    if (!audio || !local) return;
    await contextRef.current?.resume();
    if (resume&&audio.src) await audio.play();
  }
  async function interrupt(){
    cancelRecovery();
    sourceGeneration.current.begin();loadedPlayback.current=undefined;
    nativePlayback.current=undefined;nativeSnapshot.current={};setNativeState({});
    if(isNative)await invokeNative("native_stop");
    else if(continuous)await pcm.interrupt();
    else if(audioRef.current)clearAudio(audioRef.current);
    setPlaying(false);
    setPreparing(false);
  }
  function finishChange(){if(continuous)pcm.finishChange();else setOnlineRevision(value=>value+1);}
  function seek(position: number) {
    ignoreWaitingUntil.current = Date.now() + 1800;
    stalls.current = [];
    if(continuous)pcm.seek(position);
    else if (isNative && local){
      pendingSeek.current=position;
      const previous=nativePlayback.current;
      if(previous?.started&&!currentRef.current?.playback_id&&endpoint?.capabilities.continuous_pcm!==true){
        const expected={...previous,id:clientId()};nativePlayback.current=expected;nativeSnapshot.current={};setNativeState({});
        invokeNative("native_seek", { position,playbackId:expected.id }).catch(e=>{if(nativePlayback.current===expected)report(e.message);});
      }
    }
    else if (audioRef.current && local) {
      if(audioRef.current.readyState===0)pendingSeek.current=position;
      if (adaptive) setAdaptiveStart(position);
      else audioRef.current.currentTime = position;
    }
    setPosition(position);
  }
  return {
    position: local ? continuous?pcm.position:position : zone?.position || 0,
    outputPosition:()=>!continuous&&audioRef.current?audioRef.current.currentTime+offsetRef.current:continuous?pcm.position:position,
    playing: nativeGroup?zone?.status==="playing":local ? continuous?(pcm.preparing?zone?.status==="playing":pcm.playing):(preparing ? zone?.status==="playing" : playing) : zone?.status === "playing",
    unlock,
    interrupt,finishChange,
    seek,
    analyserRef:continuous?pcm.analyserRef:analyserRef,
    local,
    adaptive:continuous?pcm.adaptive:adaptive,
    nativeState:continuous?pcm.nativeState:nativeState,
    sourceInfo,sourceError,contextSampleRate:continuous?pcm.contextSampleRate:contextSampleRate,
  };
}
