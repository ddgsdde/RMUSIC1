import {normalizeDsp,ApiError,type RMusicClient,type SequenceBoundary,type SequencePrepared,type SequenceSlot,type Track,type Zone} from "@r-music/sdk";
import {clientId} from "../clientId";
import {listenJournal,listenCore,type ListenOwner,type ListenSnapshot} from "../offlineStats";
import {PcmPlayer,PcmTransportError,type PcmInput} from "./pcm-player";
import {PcmListen} from "./pcm-listens";
import type {PcmSnapshot} from "./pcm-renderer";
import {pcmJournal,closePcmPage,type PcmCursor} from "./pcm-journal";

interface Entry {id:string;order:number;track:Track;origin:number;base:number;frames:number;ended:boolean;listen?:PcmListen;session?:string;queue?:Track[];queueIndex?:number;codec?:"aac";adaptiveNotice?:boolean}
interface Frozen {action:"handoff"|"progress"|"ended"|"prepare";body:unknown;current:string;revisions:number[]}
export interface SequenceView {track:Track;playbackId:string;position:number;playing:boolean;preparing:boolean;finished:boolean;underrunFrames:number;adaptive:number}
export interface ResumePoint {id:string;origin:number;frames:number}
export function pcmSequenceKey(zone:Zone|undefined){const dsp=normalizeDsp(zone?.dsp);return JSON.stringify([
  zone?.queue.map(track=>[track.id,track.source,track.format,track.sample_rate,track.bit_depth,track.channels,track.duration]),zone?.repeat,zone?.shuffle,
  [zone?.autoplay?.enabled??false,zone?.autoplay?.source??"mixed",zone?.autoplay?.fallback_local??true],
  [dsp.enabled,dsp.preamp_db,dsp.balance,dsp.normalize,dsp.target_lufs,dsp.bands.map(band=>[band.enabled,band.kind,band.frequency,band.gain_db,band.q])],
  [zone?.transitions?.fade_in_ms??0,zone?.transitions?.fade_out_ms??0],
]);}

/** One coordinator commits actual source boundaries independently of audio rendering. */
export class PcmSequence {
  private player?:PcmPlayer;
  private entries=new Map<string,Entry>();
  private confirmed:string;
  private plan="";
  private refresh=false;
  private refreshedAt=0;
  private remote:Zone;
  private revisions=[0,0,0];
  private controller=new AbortController();
  private live=true;
  private prepared=false;
  private terminal=false;
  private current?:Entry;
  private state?:PcmSnapshot;
  private owner?:ListenOwner;
  private writes=Promise.resolve();
  private snapshots:ListenSnapshot[]=[];
  private cursor?:PcmCursor;
  private checkpointAt=0;
  private savingFailed=false;
  private clockOffset:number;
  private reporting=false;
  private frozen?:Frozen;
  private lastReport=0;
  private lastFrames=-1;
  private delay?:ReturnType<typeof setTimeout>;
  private wakeDelay?:()=>void;
  private audioError?:Error;
  private errorPaused=false;
  private retryRequested=false;
  private reconnecting=false;
  private audioAttempts=0;
  private audioBaseline?:{id:string;frames:number};
  private audioRetry?:ReturnType<typeof setTimeout>;
  private aacCapable=false;
  private adapting=false;
  private adaptationAttempted="";
  private stallSource="";
  private stallFrames=-1;
  private stalls:number[]=[];
  private stopped?:Promise<void>;
  constructor(private api:RMusicClient,zone:Zone,private context:AudioContext,private output:(node:AudioNode)=>void,private view:(state:SequenceView)=>void,private adopt:(zone:Zone)=>void,private error:(error:Error)=>void,private allowLossy=false){
    this.remote=zone;this.confirmed=zone.playback_id!;this.clockOffset=Date.now()-context.currentTime*1000;
  }
  get active(){return this.live;}
  get key(){return pcmSequenceKey(this.remote);}
  older(zone:Zone){return zone.updated_at<this.remote.updated_at;}
  owns(zone:Zone){return this.live&&zone.id===this.remote.id&&(!!zone.playback_id&&(zone.playback_id===this.confirmed||this.entries.has(zone.playback_id))||zone.updated_at<this.remote.updated_at);}
  matchesPlan(plan:string){return this.live&&this.plan===plan;}
  appended(zone:Zone){return !this.remote.shuffle&&zone.playback_id===this.remote.playback_id&&zone.queue_index===this.remote.queue_index&&zone.queue.length>this.remote.queue.length&&this.remote.queue.every((track,index)=>track.id===zone.queue[index]?.id)&&pcmSequenceKey({...zone,queue:this.remote.queue})===this.key;}
  projected(zone:Zone){const entry=zone.playback_id&&this.entries.get(zone.playback_id);return !!entry&&!!entry.queue&&zone.queue_index===entry.queueIndex&&pcmSequenceKey({...this.remote,queue:entry.queue})===pcmSequenceKey(zone);}
  resumePoint():ResumePoint|undefined {const entry=this.current;return entry?{id:entry.id,origin:entry.origin,frames:entry.frames}:undefined;}
  private boundary(entry:Entry):SequenceBoundary {return {playback_id:entry.id,track_id:entry.track.id,frames:entry.frames,position:entry.origin+entry.frames/this.context.sampleRate};}
  private processing(entry:Entry){
    const dsp=this.remote.dsp,fade=this.remote.transitions,measured=entry.track.integrated_lufs;
    const peak=entry.track.true_peak_dbfs;
    const normalization=dsp.normalize&&measured!=null&&Number.isFinite(measured)?Math.min(dsp.target_lufs-measured,peak==null?Infinity:-peak):0;
    return {gain:dsp.enabled?10**((dsp.preamp_db+normalization)/20):1,startFrame:Math.round((entry.origin+entry.base/this.context.sampleRate)*this.context.sampleRate),durationFrames:Math.round(entry.track.duration*this.context.sampleRate),fadeInFrames:(fade?.fade_in_ms||0)*this.context.sampleRate/1000,fadeOutFrames:(fade?.fade_out_ms||0)*this.context.sampleRate/1000};
  }
  private source(entry:Entry,url:string):PcmInput {return {id:entry.id,url,start:entry.origin+entry.base/this.context.sampleRate,processing:this.processing(entry),codec:entry.codec};}
  private add(id:string,track:Track,session?:string,origin=0,base=0){
    const entry:Entry={id,order:Math.max(-1,...[...this.entries.values()].map(entry=>entry.order))+1,track,session,origin,base,frames:base,ended:false};
    if(this.owner&&session)entry.listen=new PcmListen(track,id,session,this.context.sampleRate,origin,base);
    this.entries.set(id,entry);return entry;
  }
  private async request<T>(path:string,body?:unknown,method=body===undefined?"GET":"POST"):Promise<T>{
    const controller=new AbortController(),abort=()=>controller.abort();
    this.controller.signal.addEventListener("abort",abort,{once:true});if(this.controller.signal.aborted)controller.abort();
    const timeout=setTimeout(abort,5000);
    try{return await this.api.request<T>(path,{method,...(body===undefined?{}:{body:JSON.stringify(body)}),signal:controller.signal});}
    finally{clearTimeout(timeout);this.controller.signal.removeEventListener("abort",abort);}
  }
  private async sourceUrl(entry:Entry,start:number){
    const controller=new AbortController(),abort=()=>controller.abort();
    this.controller.signal.addEventListener("abort",abort,{once:true});if(this.controller.signal.aborted)controller.abort();
    const timeout=setTimeout(abort,5000);
    try{return await (entry.codec==="aac"?this.api.aacPcmUrl(entry.track.id,this.context.sampleRate,start,controller.signal):this.api.pcmUrl(entry.track.id,this.context.sampleRate,start,controller.signal));}
    finally{clearTimeout(timeout);this.controller.signal.removeEventListener("abort",abort);}
  }
  async start(resume?:ResumePoint){
    const zone=this.remote,id=this.confirmed;
    const [status,user,initial]=await Promise.all([
      this.request<{core_id:string;capabilities?:{browser_pcm?:boolean;browser_aac_pcm?:boolean;measured_listens?:boolean}}>("/status"),
      this.request<{id:string}>("/auth/me"),
      this.request<SequencePrepared>(`/zones/${zone.id}/prepare`,{endpoint_id:zone.endpoint_id,track_id:zone.track!.id,playback_id:id}),
    ]);
    if(!this.live)return;
    if(status.capabilities?.browser_pcm!==true||status.capabilities.measured_listens!==true)throw new Error("核心尚不支持浏览器连续音频");
    this.aacCapable=this.allowLossy&&status.capabilities.browser_aac_pcm===true;
    const expected=listenCore(this.api.baseUrl);if(expected&&status.core_id!==expected)throw new Error("连接的核心身份已改变，请重新连接");
    this.owner={core_id:status.core_id,user_id:user.id};
    if(!initial.accepted){this.adopt(initial.zone);await this.stop();return;}
    this.plan=initial.plan_id;
    const same=resume?.id===id,origin=same?resume.origin:zone.position,base=same?resume.frames:0;
    const entry=this.add(id,zone.track!,initial.listen_session_id,origin,base);this.current=entry;
    if(initial.continuous_recovery){
      this.cursor=await pcmJournal.begin({owner:this.owner,endpoint:zone.endpoint_id,zone:zone.id,plan:this.plan,rate:this.context.sampleRate,heard:[this.boundary(entry)],terminal:false});
      if(!this.live){await pcmJournal.retire(this.cursor);return;}
    }
    this.player=await PcmPlayer.create(this.context,(state,frame)=>this.consume(state,frame),error=>this.failedAudio(error),this.controller.signal);
    if(!this.live){await this.player.stop();return;}
    this.output(this.player.node);this.player.setPaused(this.remote.status!=="playing");
    const url=await this.sourceUrl(entry,entry.origin+entry.base/this.context.sampleRate);
    if(!this.live)return;
    this.player.append([this.source(entry,url)]);this.fill(initial);
    await this.player.ready.catch(async error=>{await this.player?.stop();if(this.live&&!this.audioError)this.failedAudio(error instanceof Error?error:new Error(String(error)));});if(!this.live||this.audioError)return;
    this.prepared=true;this.emit();
  }
  private stage(slots:SequenceSlot[]){
    const sources:PcmInput[]=[];
    for(const slot of slots){
      if(this.entries.has(slot.playback_id))continue;
      const entry=this.add(slot.playback_id,slot.track,slot.listen_session_id),url=new URL(this.api.url('/'+slot.media_path.replace(/^\//,'')),location.href);
      entry.queue=slot.queue;entry.queueIndex=slot.queue_index;
      url.pathname=url.pathname.replace(/\/media\//,"/pcm/");url.searchParams.set("rate",String(this.context.sampleRate));url.searchParams.set("start","0");sources.push(this.source(entry,url.href));
    }
    if(sources.length)this.player!.append(sources);
  }
  private fill(prepared:Extract<SequencePrepared,{accepted:true}>){
    this.refresh=prepared.refresh===true;this.refreshedAt=Date.now();
    if(this.terminal||this.audioError)return;
    this.stage(prepared.items);if(!prepared.has_more)this.player?.seal();
  }
  update(zone:Zone){
    if(!this.live||zone.updated_at<this.remote.updated_at)return;
    const entry=zone.playback_id&&this.entries.get(zone.playback_id),confirmed=this.entries.get(this.confirmed);
    if(entry&&confirmed&&entry.order<confirmed.order)return;
    for(const [i,key] of ["status","volume","muted"].entries())if(zone[key as "status"|"volume"|"muted"]!==this.remote[key as "status"|"volume"|"muted"])this.revisions[i]++;
    const appended=this.appended(zone);if(appended)for(const value of this.entries.values())if(value.queue)value.queue=zone.queue;
    const wasPaused=this.remote.status==="paused";this.remote=zone;this.player?.setPaused(zone.status!=="playing");
    for(const entry of this.entries.values()) {
      const track=zone.track?.id===entry.track.id?zone.track:zone.queue.find(t=>t.id===entry.track.id);
      if(track&&(track.integrated_lufs!==entry.track.integrated_lufs||track.true_peak_dbfs!==entry.track.true_peak_dbfs)){
        entry.track={...entry.track,integrated_lufs:track.integrated_lufs,true_peak_dbfs:track.true_peak_dbfs};
        this.player?.updateGain(entry.id,this.processing(entry).gain);
      }
    }
    if(this.audioError&&wasPaused&&zone.status==="playing")this.retryRequested=true;
    if(this.live){this.emit();this.kick();}
  }
  requestRetry(){if(this.audioError){clearTimeout(this.audioRetry);this.audioAttempts=0;this.retryRequested=true;this.kick();}}
  private save(snapshot:ListenSnapshot|undefined){
    if(snapshot)this.snapshots.push(snapshot);
  }
  private checkpoint(force=false){
    if(!this.owner||!force&&!this.snapshots.length&&Date.now()-this.checkpointAt<1000)return;
    const snapshots=this.snapshots.splice(0),owner=this.owner;this.checkpointAt=Date.now();
    const current=this.current;
    if(this.cursor&&current)this.cursor={...this.cursor,version:this.cursor.version+1,heard:[...this.entries.values()].filter(entry=>entry.order<=current.order&&entry.order>=current.order-8).sort((a,b)=>a.order-b.order).map(entry=>this.boundary(entry)),terminal:current.ended};
    const cursor=this.cursor&&structuredClone(this.cursor);
    this.writes=this.writes.then(async()=>{if(cursor&&!this.savingFailed)await pcmJournal.checkpoint(cursor,snapshots);else for(const snapshot of snapshots)await listenJournal.save(owner,snapshot);}).catch(error=>{if(!this.savingFailed){this.savingFailed=true;this.error(new Error(`本机聆听记录保存失败：${String(error)}`));}});
  }
  pageClosed(){if(this.cursor)closePcmPage(this.cursor);}
  private consume(state:PcmSnapshot,frame:number){
    const moved=this.state?.frames!==state.frames,underrunBefore=this.state?.underrunFrames||0;this.state=state;
    const at=(local:number)=>this.clockOffset+(frame-state.outputFrames+local)/this.context.sampleRate*1000;
    for(const span of state.spans){const entry=this.entries.get(span.id);entry?.listen?.sample(span.start,span.end,at(span.endFrame));}
    for(const source of [...state.sources,...state.ended]){
      const entry=this.entries.get(source.id);if(!entry)continue;const before=entry.frames;entry.frames=entry.base+source.frames;
      this.save(entry.listen?.flush(this.remote.status!=="playing"&&entry.frames!==before));
    }
    for(const source of state.ended){const entry=this.entries.get(source.id);if(entry){entry.ended=true;this.save(entry.listen?.finish("ended",at(source.endFrame!)));}}
    const id=state.sources[0]?.id||state.ended.at(-1)?.id;if(id)this.current=this.entries.get(id);
    if(this.audioBaseline&&this.current&&(this.current.id!==this.audioBaseline.id||this.current.frames-this.audioBaseline.frames>=this.context.sampleRate/4)){this.audioAttempts=0;this.audioBaseline=undefined;}
    if(state.finished&&this.current?.ended)this.terminal=true;
    if(moved||state.ended.length||this.snapshots.length)this.checkpoint(state.ended.length>0||this.remote.status!=="playing");
    if(this.live){
      const entry=this.current;
      if(entry&&this.stallSource!==entry.id){this.stallSource=entry.id;this.stallFrames=-1;this.stalls=[];}
      if(entry&&!entry.codec&&this.prepared&&!this.audioError&&!this.adapting&&this.remote.status==="playing"&&this.context.state==="running"&&state.underrunFrames>underrunBefore&&entry.frames>=this.context.sampleRate/4&&(this.stallFrames<0||entry.frames-this.stallFrames>=this.context.sampleRate/4)){
        this.stallFrames=entry.frames;const now=Date.now();this.stalls=[...this.stalls.filter(at=>now-at<60000),now];
        if(this.stalls.length>=3&&this.adaptationAttempted!==entry.id)void this.adapt(entry);
      }
      this.emit();this.kick();
    }
  }
  private async adapt(entry:Entry){
    this.adapting=true;this.adaptationAttempted=entry.id;
    try{
      const supported=this.aacCapable&&typeof AudioDecoder!=="undefined"&&(await AudioDecoder.isConfigSupported({codec:"mp4a.40.2",sampleRate:48000,numberOfChannels:2,description:new Uint8Array([0x11,0x90])}).catch(()=>({supported:false}))).supported;
      if(!this.live||this.current!==entry||entry.ended)return;
      if(!supported){this.error(new Error(this.aacCapable?"当前浏览器不支持 AAC 流式解码，保留 PCM。":"无损传输缓冲不足，仍保持 PCM；请增加缓冲或使用原生客户端。"));return;}
      if(this.audioError&&!(this.audioError instanceof PcmTransportError&&this.audioError.retryable))return;
      await this.player?.stop();if(!this.live)return;
      if(this.current===entry&&!entry.ended){entry.codec="aac";entry.adaptiveNotice=true;}
      if(!this.audioError)this.failedAudio(new PcmTransportError("网络缓冲不足，正在切换 AAC"));
      clearTimeout(this.audioRetry);this.retryRequested=true;this.kick();
    }finally{this.adapting=false;}
  }
  private emit(){
    const entry=this.entries.get(this.confirmed)||this.current;if(!entry)return;
    this.view({track:entry.track,playbackId:entry.id,position:this.boundary(entry).position,playing:this.context.state==="running"&&this.prepared&&!this.audioError&&!this.terminal&&this.remote.status==="playing",preparing:this.context.state==="running"&&!this.prepared&&(!this.audioError||this.reconnecting),finished:this.terminal,underrunFrames:this.state?.underrunFrames||0,adaptive:entry.codec==="aac"?128:0});
  }
  private merge(zone:Zone,revisions:number[]){
    return {...zone,...(this.revisions[0]!==revisions[0]&&zone.playback_id?{status:this.remote.status}:{}),...(this.revisions[1]!==revisions[1]?{volume:this.remote.volume}:{}),...(this.revisions[2]!==revisions[2]?{muted:this.remote.muted}:{})};
  }
  private failedAudio(error:Error){
    if(!this.live||this.audioError)return;this.audioError=error;this.prepared=false;
    for(const entry of this.entries.values())this.save(entry.listen?.finish("interrupted"));this.checkpoint(true);
    this.reconnecting=error instanceof PcmTransportError&&error.retryable&&this.audioAttempts<5;
    if(this.reconnecting){
      this.audioBaseline??=this.current&&{id:this.current.id,frames:this.current.frames};
      this.audioRetry=setTimeout(()=>{this.retryRequested=true;this.kick();},1000*2**this.audioAttempts);
    }
    if(this.audioAttempts===0||!this.reconnecting){
      const exhausted=error instanceof PcmTransportError&&error.retryable&&this.audioAttempts>=5;
      this.error(this.reconnecting?new Error("音源连接中断，正在从实际播放位置重新连接。"):exhausted?new Error("音源连接重试五次仍失败，请点击播放重试。"):error);
    }
    this.emit();this.kick();
  }
  private next():Frozen|undefined {
    const current=this.current,confirmed=this.entries.get(this.confirmed);if(!current||!confirmed)return;
    if(current.order>confirmed.order){
      const ended=[...this.entries.values()].filter(entry=>entry.order>=confirmed.order&&entry.order<current.order).sort((a,b)=>a.order-b.order);
      if(ended.some(entry=>!entry.ended))return;
      return {action:"handoff",current:current.id,revisions:[...this.revisions],body:{endpoint_id:this.remote.endpoint_id,request_id:clientId(),plan_id:this.plan,from_playback_id:this.confirmed,sample_rate:this.context.sampleRate,ended:ended.map(entry=>this.boundary(entry)),current:this.boundary(current)}};
    }
    if(this.terminal&&this.remote.status==="playing")return {action:"ended",current:current.id,revisions:[...this.revisions],body:{...this.boundary(current),sample_rate:this.context.sampleRate}};
    if(this.audioError&&!this.reconnecting&&!this.errorPaused&&!this.retryRequested)return {action:"progress",current:current.id,revisions:[...this.revisions],body:{track_id:current.track.id,playback_id:current.id,status:"paused"}};
    if(!this.audioError&&current.frames!==this.lastFrames&&Date.now()-this.lastReport>=1000)return {action:"progress",current:current.id,revisions:[...this.revisions],body:{...this.boundary(current),sample_rate:this.context.sampleRate}};
    if(this.refresh&&!this.terminal&&!this.audioError&&Date.now()-this.refreshedAt>=1000)return {action:"prepare",current:this.confirmed,revisions:[...this.revisions],body:{endpoint_id:this.remote.endpoint_id,track_id:confirmed.track.id,playback_id:this.confirmed}};
  }
  private kick(){if(this.reporting||!this.live)return;void this.drain();}
  private async drain(){
    this.reporting=true;let attempts=0;
    try {while(this.live){
      if(this.audioError&&this.retryRequested&&!this.frozen&&this.confirmed===this.current?.id){
        try{await this.reopen();}catch(error){
          if(error instanceof ApiError&&error.status<500&&error.status!==429)throw error;
          if(this.live)this.failedAudio(error instanceof PcmTransportError?error:new PcmTransportError(String(error),error instanceof TypeError||error instanceof DOMException&&error.name==="AbortError"||error instanceof ApiError));
        }continue;
      }
      const request=this.frozen||this.next();if(!request)break;this.frozen=request;
      try{
        const response=await this.request<{accepted?:boolean;zone:Zone}>(`/zones/${this.remote.id}/${request.action}`,request.body);
        if(!this.live)return;this.frozen=undefined;attempts=0;
        if(request.action==="prepare"){
          const prepared=response as unknown as SequencePrepared;
          if(!prepared.accepted||prepared.plan_id!==this.plan){if(!prepared.accepted)this.adopt(prepared.zone);await this.stop();return;}
          this.fill(prepared);
        }else if(request.action==="progress"){
          this.lastReport=Date.now();this.lastFrames=(request.body as SequenceBoundary).frames;
          if(this.audioError){this.errorPaused=true;const zone=await this.request<Zone>(`/zones/${this.remote.id}`);if(this.live){this.remote=zone;this.adopt(zone);}}
        }else if(request.action==="handoff"){
          if(!response.accepted||response.zone.playback_id!==request.current){this.adopt(response.zone);await this.stop();return;}
          this.confirmed=request.current;this.remote=this.merge(response.zone,request.revisions);this.adopt(this.remote);this.emit();
          const current=this.entries.get(this.confirmed)!;
          const prepared=await this.request<SequencePrepared>(`/zones/${this.remote.id}/prepare`,{endpoint_id:this.remote.endpoint_id,track_id:current.track.id,playback_id:this.confirmed});
          if(!this.live)return;
          if(!prepared.accepted||prepared.plan_id!==this.plan){if(!prepared.accepted)this.adopt(prepared.zone);await this.stop();return;}
          this.fill(prepared);
          for(const [id,entry] of this.entries)if(entry.order<current.order-8)this.entries.delete(id);
        }else{
          const zone=this.merge(response as unknown as Zone,request.revisions);
          if(zone.playback_id===request.current&&zone.status==="paused"){this.remote=zone;this.adopt(zone);break;}
          this.adopt(zone);await this.stop();return;
        }
      }catch(error){
        if(!this.live)return;
        if(error instanceof ApiError&&error.status<500&&error.status!==429)throw error;
        const wait=Math.min(1000*2**Math.min(attempts++,5),30000);
        await new Promise<void>(yes=>{this.wakeDelay=yes;this.delay=setTimeout(yes,wait);});this.wakeDelay=undefined;
      }
    }}catch(error){if(this.live){this.error(error instanceof Error?error:new Error(String(error)));await this.stop("interrupted",error instanceof ApiError&&[401,403,404,409].includes(error.status));}}
    finally{this.reporting=false;}
  }
  private async reopen(){
    const current=this.current!;this.retryRequested=false;this.errorPaused=false;this.audioError=undefined;this.audioAttempts++;
    const position=this.boundary(current).position;
    const fresh=await this.request<Zone>(`/zones/${this.remote.id}`);
    if(!this.live)return;
    if(fresh.playback_id!==current.id||fresh.track?.id!==current.track.id||fresh.endpoint_id!==this.remote.endpoint_id||fresh.status==="stopped"){this.adopt(fresh);await this.stop("interrupted",true);return;}
    this.update(fresh);this.adopt(this.remote);
    const prepared=await this.request<SequencePrepared>(`/zones/${this.remote.id}/prepare`,{endpoint_id:this.remote.endpoint_id,track_id:current.track.id,playback_id:current.id});
    if(!this.live)return;
    if(!prepared.accepted||prepared.plan_id!==this.plan){if(!prepared.accepted)this.adopt(prepared.zone);await this.stop("interrupted",true);return;}
    const player=await PcmPlayer.create(this.context,(state,frame)=>this.consume(state,frame),error=>this.failedAudio(error),this.controller.signal);
    if(!this.live){await player.stop();return;}
    this.player=player;this.output(player.node);current.base=current.frames;current.ended=false;
    const sources=[current,...[...this.entries.values()].filter(entry=>entry.order>current.order).sort((a,b)=>a.order-b.order)];
    for(const entry of sources){entry.base=entry===current?entry.frames:0;entry.frames=entry.base;entry.ended=false;entry.listen=this.owner&&entry.session?new PcmListen(entry.track,entry.id,entry.session,this.context.sampleRate,entry.origin,entry.base):undefined;}
    const urls=await Promise.all(sources.map(entry=>this.sourceUrl(entry,entry===current?position:0)));
    if(!this.live)return;
    player.setPaused(this.remote.status!=="playing");player.append(sources.map((entry,i)=>this.source(entry,urls[i])));
    this.fill(prepared);
    await player.ready.catch(async error=>{await player.stop();if(this.live&&!this.audioError)this.failedAudio(error instanceof Error?error:new Error(String(error)));});if(this.live&&!this.audioError){this.prepared=true;this.reconnecting=false;if(current.adaptiveNotice){current.adaptiveNotice=false;this.error(new Error("网络缓冲不足，已切换 AAC 128 kbps；信号链会显示音质转换。"));}this.emit();}
  }
  stop(reason:ListenSnapshot["reason"]="interrupted",preserveCursor=false):Promise<void>{
    if(this.stopped)return this.stopped;
    this.live=false;this.controller.abort();clearTimeout(this.audioRetry);clearTimeout(this.delay);this.wakeDelay?.();
    this.stopped=(async()=>{await this.player?.stop();for(const entry of this.entries.values())this.save(entry.listen?.finish(reason));this.checkpoint(true);await this.writes;if(this.cursor&&!preserveCursor)await pcmJournal.retire(this.cursor);})();return this.stopped;
  }
}
