import {useEffect,useRef,useState} from "react";
import type {RMusicClient,Zone} from "@r-music/sdk";
import type {NativePlayback} from "../native";
import {PcmSequence,pcmSequenceKey,type SequenceView,type ResumePoint} from "./pcm-sequence";
import {PcmGraph} from "./pcm-graph";
import {recoverPcm} from "./pcm-recovery";
import {ApiError} from "@r-music/sdk";

export function browserPcmAvailable(){return window.isSecureContext&&typeof AudioContext!=="undefined"&&typeof AudioWorkletNode!=="undefined";}
export function usePcmAudio(api:RMusicClient,zone:Zone|undefined,enabled:boolean,report:(message:string)=>void,adopt:(zone:Zone)=>void,durable=false){
  const context=useRef<AudioContext|null>(null),graph=useRef<PcmGraph|null>(null),actor=useRef<PcmSequence|null>(null),generation=useRef(0),pending=useRef<Promise<void>>(Promise.resolve());
  const latest=useRef(zone);latest.current=zone;
  const callbacks=useRef({report,adopt});callbacks.current={report,adopt};
  const analyserRef=useRef<AnalyserNode|null>(null),[rate,setRate]=useState(0),[state,setState]=useState<SequenceView>(),[offline,setOffline]=useState(false),[revision,setRevision]=useState(0);
  const semantic=pcmSequenceKey(zone);
  const admission=useRef<AbortController|null>(null),retry=useRef<ReturnType<typeof setTimeout>|undefined>(undefined),failures=useRef(0);
  const restoredIdentity=useRef<string|null|undefined>(undefined);
  const manual=useRef(false),blocked=useRef(false),unlocked=useRef<(()=>void)[]>([]);
  function finishUnlock(){for(const done of unlocked.current.splice(0))done();}
  function audioContext(){
    if(!context.current||context.current.state==="closed"){
      context.current=new AudioContext();graph.current=new PcmGraph(context.current);analyserRef.current=graph.current.analyser;setRate(context.current.sampleRate);
    }
    return context.current;
  }
  function halt(reason:"interrupted"|"stop"="interrupted",preserve=true){
    generation.current++;admission.current?.abort();clearTimeout(retry.current);retry.current=undefined;finishUnlock();const old=actor.current;actor.current=null;
    const stopped=old?.stop(reason,preserve)||Promise.resolve();pending.current=Promise.allSettled([pending.current,stopped]).then(()=>{});return pending.current;
  }
  useEffect(()=>{
    const mark=()=>{setOffline(true);void halt();setState(undefined);};
    const online=()=>{setOffline(false);setRevision(value=>value+1);};
    const closing=()=>{manual.current=false;blocked.current=false;actor.current?.pageClosed();void halt();};
    window.addEventListener("rmusic-offline",mark);window.addEventListener("rmusic-offline-stop",mark);window.addEventListener("rmusic-online",online);
    window.addEventListener("pagehide",closing);
    return()=>{window.removeEventListener("rmusic-offline",mark);window.removeEventListener("rmusic-offline-stop",mark);window.removeEventListener("rmusic-online",online);window.removeEventListener("pagehide",closing);};
  },[]);
  useEffect(()=>{const registered=(event:Event)=>{if((event as CustomEvent).detail===zone?.endpoint_id)setRevision(value=>value+1);};window.addEventListener("rmusic-browser-registered",registered);return()=>window.removeEventListener("rmusic-browser-registered",registered);},[zone?.endpoint_id]);
  useEffect(()=>{const invalidated=(event:Event)=>{const data=(event as CustomEvent).detail as {zone_id?:string;plan_id?:string};const old=actor.current;if(old&&data.zone_id===latest.current?.id&&data.plan_id&&old.matchesPlan(data.plan_id)){generation.current++;admission.current?.abort();void old.stop("interrupted",true);setRevision(value=>value+1);}};window.addEventListener("rmusic-sequence-invalidated",invalidated);return()=>window.removeEventListener("rmusic-sequence-invalidated",invalidated);},[]);
  useEffect(()=>()=>{manual.current=false;blocked.current=false;const oldGraph=graph.current,oldContext=context.current;graph.current=null;context.current=null;analyserRef.current=null;void halt().finally(()=>{oldGraph?.close();void oldContext?.close();});},[api,enabled,zone?.id]);
  useEffect(()=>{failures.current=0;restoredIdentity.current=undefined;clearTimeout(retry.current);retry.current=undefined;},[api,enabled,zone?.id]);
  useEffect(()=>{if(restoredIdentity.current!==zone?.playback_id){failures.current=0;clearTimeout(retry.current);retry.current=undefined;}restoredIdentity.current=undefined;},[zone?.playback_id]);
  useEffect(()=>{
    if(!enabled||offline||!zone||!durable&&(!zone.track||!zone.playback_id||zone.status==="stopped")){void halt();setState(undefined);return;}
    const old=actor.current;
    if(old?.owns(zone)&&(old.key===semantic||old.older(zone)||old.projected(zone)||old.appended(zone))){const older=old.older(zone);old.update(zone);if(!older)graph.current?.gain(zone);return;}
    if(!old&&(failures.current>5||retry.current))return;
    const attempt=++generation.current;
    admission.current?.abort();clearTimeout(retry.current);const controller=new AbortController();admission.current=controller;
    actor.current=null;const stopping=old?.stop("interrupted",!zone.playback_id)||Promise.resolve();
    pending.current=pending.current.catch(()=>{}).then(async()=>{
      await stopping;if(generation.current!==attempt)return;
      const resume:ResumePoint|undefined=old?.resumePoint();
      let current=latest.current;if(!current)return;
      try{
      if(durable){const restored=await recoverPcm(api,current,controller.signal,manual.current);if(!restored){blocked.current=true;setState(undefined);return;}current=restored;blocked.current=false;}
      if(generation.current!==attempt)return;
      if(!current.playback_id||!current.track||current.status==="stopped"){setState(undefined);callbacks.current.adopt(current);return;}
      const ctx=audioContext();graph.current!.configure(current);
      const next=new PcmSequence(api,current,ctx,node=>graph.current?.attach(node),view=>{if(generation.current===attempt)setState(view);},value=>{if(generation.current===attempt)callbacks.current.adopt(value);},error=>{if(generation.current===attempt)callbacks.current.report(error.message);});
      actor.current=next;
      if(durable){if(current.playback_id!==latest.current?.playback_id)restoredIdentity.current=current.playback_id;callbacks.current.adopt(current);}
      await next.start(resume);
      if(generation.current===attempt)failures.current=0;
      }catch(error){if(generation.current===attempt&&!controller.signal.aborted){
        await actor.current?.stop("interrupted",true);actor.current=null;
        if(failures.current++===0)callbacks.current.report(error instanceof Error?error.message:String(error));
        if(!(error instanceof ApiError&&error.status<500&&error.status!==429)){
          if(failures.current<=5)retry.current=setTimeout(()=>{retry.current=undefined;setRevision(value=>value+1);},1000*2**(failures.current-1));
          else callbacks.current.report("音频连接重试五次仍失败，请点击播放重试。");
        }
      }}finally{if(generation.current===attempt)finishUnlock();}
    });
  },[api,enabled,durable,offline,zone?.id,zone?.playback_id,zone?.status,zone?.volume,zone?.muted,zone?.track?.integrated_lufs,semantic,revision]);
  async function unlock(){if(!enabled)return;actor.current?.requestRetry();await audioContext().resume();if(!actor.current&&failures.current){failures.current=0;clearTimeout(retry.current);retry.current=undefined;setRevision(value=>value+1);}if(blocked.current){manual.current=true;blocked.current=false;const ready=new Promise<void>(done=>unlocked.current.push(done));setRevision(value=>value+1);await ready;}}
  async function interrupt(){await halt("stop",false);setState(undefined);}
  const nativeState:NativePlayback=state?{status:state.finished?"ended":state.preparing?"preparing":"playing",core_managed:true,track_id:state.track.id,core_playback_id:state.playbackId,position:state.position,paused:!state.playing,signal_path:{output:"浏览器 Web Audio",sample_rate:rate,channels:2,exclusive:false,bit_depth:32,sample_type:"float",bit_perfect_eligible:false,processing:[...(state.adaptive?["AAC 128 kbps (network)","WebCodecs AAC decode"]:["Core PCM f32le"]),"Web Audio shared mixer",...(zone?.dsp.enabled?["PEQ / balance / source normalization"]:[]),...(zone?.transitions?.fade_in_ms||zone?.transitions?.fade_out_ms?["Fade envelope"]:[])]}}:{};
  return {position:state?.position??zone?.position??0,playing:state?.playing??false,preparing:state?.preparing??false,adaptive:state?.adaptive??0,analyserRef,contextSampleRate:rate,nativeState,unlock,interrupt,seek:(position:number)=>setState(state=>state?{...state,position}:state),finishChange:()=>setRevision(value=>value+1)};
}
