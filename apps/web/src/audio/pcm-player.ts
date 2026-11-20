import workletUrl from "./pcm-worklet.ts?worker&url";
import {PcmFramer,type PcmSnapshot,type PcmProcessing} from "./pcm-renderer";

export interface PcmInput {id:string;url:string;start?:number;processing?:PcmProcessing;codec?:"aac"}
interface Flow extends PcmInput {sent:number;rendered:number;eof:boolean;inputEof:boolean;queued:Float32Array[];queuedFrames:number;failurePosted?:boolean;failed?:boolean;error?:Error}
interface WorkletEvent {type:string;state?:PcmSnapshot;frame:number;error?:string;sourceId?:string}
const modules=new WeakMap<BaseAudioContext,Promise<void>>();
export class PcmTransportError extends Error {constructor(message:string,readonly retryable=true){super(message);}}

// Transport and buffering only. Core queue commits, listening identity and DSP belong to the caller.
export class PcmPlayer {
  readonly node:AudioWorkletNode;
  readonly ready:Promise<void>;
  private resolveReady!:()=>void;
  private rejectReady!:(error:Error)=>void;
  private flows=new Map<string,Flow>();
  private pending:Flow[]=[];
  private waiting=new Set<()=>void>();
  private controller=new AbortController();
  private running=0;
  private closed=false;
  private prepared=false;
  private wantedPaused=true;
  private last?:PcmSnapshot;
  private stopped?:Promise<PcmSnapshot|undefined>;
  private finishStop?:()=>void;
  private readers=new Set<ReadableStreamDefaultReader<Uint8Array>>();
  private advancedAt=Date.now();
  private watchdog:ReturnType<typeof setInterval>;
  static async create(context:BaseAudioContext,onState:(state:PcmSnapshot,frame:number)=>void,onError:(error:Error)=>void,signal?:AbortSignal){
    signal?.throwIfAborted();
    let loaded=modules.get(context);
    if(!loaded){loaded=context.audioWorklet.addModule(workletUrl);modules.set(context,loaded);loaded.catch(()=>modules.delete(context));}
    await loaded;signal?.throwIfAborted();
    return new PcmPlayer(context,onState,onError);
  }
  private constructor(private context:BaseAudioContext,private onState:(state:PcmSnapshot,frame:number)=>void,private onError:(error:Error)=>void){
    this.ready=new Promise<void>((yes,no)=>{this.resolveReady=yes;this.rejectReady=no;});
    this.ready.catch(()=>{});
    this.node=new AudioWorkletNode(context,"rmusic-continuous-pcm",{numberOfInputs:0,numberOfOutputs:1,outputChannelCount:[2]});
    this.node.port.onmessage=({data}:{data:WorkletEvent})=>{
      if(data.state){
        if(data.state.frames!==this.last?.frames)this.advancedAt=Date.now();
        this.last=data.state;
        for(const source of [...data.state.sources,...data.state.ended]){
          const flow=this.flows.get(source.id);if(flow)flow.rendered=source.frames;
        }
        for(const source of data.state.ended)this.flows.delete(source.id);
        for(const flow of this.flows.values())this.drain(flow);
        this.wake();this.pump();this.onState(data.state,data.frame);
      }
      if(data.type==="stopped")this.finishStop?.();
      if(data.type==="error")this.fail(Object.assign(data.sourceId&&this.flows.get(data.sourceId)?.error||new Error(data.error||"音频输出失败"),{sourceId:data.sourceId}));
    };
    this.node.onprocessorerror=()=>this.fail(new Error("浏览器音频输出中断"));
    this.watchdog=setInterval(()=>{
      if(this.wantedPaused||this.context.state!=="running"){this.advancedAt=Date.now();return;}
      if(this.flows.size&&!this.last?.finished&&Date.now()-this.advancedAt>=8000)this.fail(new PcmTransportError("音频连接八秒没有节目进度，正在重新连接"));
    },1000);
  }
  append(inputs:PcmInput[]){
    if(this.closed||this.last?.finished||this.flows.size+inputs.length>9||new Set(inputs.map(input=>input.id)).size!==inputs.length||inputs.some(input=>this.flows.has(input.id)))throw new Error("连续预载队列已关闭、重复或超过窗口");
    for(const input of inputs){
      const flow:Flow={...input,sent:0,rendered:0,eof:false,inputEof:false,queued:[],queuedFrames:0};
      this.flows.set(input.id,flow);this.pending.push(flow);this.node.port.postMessage({type:"begin",id:input.id,processing:input.processing});
    }
    this.pump();
  }
  updateGain(id:string,gain:number){this.node.port.postMessage({type:"gain",id,gain});}
  seal(){this.node.port.postMessage({type:"seal"});this.maybeReady();}
  setPaused(paused:boolean){if(paused!==this.wantedPaused)this.advancedAt=Date.now();this.wantedPaused=paused;this.node.port.postMessage({type:"pause",paused:paused||!this.prepared});}
  private wake(){for(const notify of this.waiting)notify();this.waiting.clear();}
  private maybeReady(){
    if(this.prepared||this.closed)return;
    const flows=[...this.flows.values()].slice(0,2);let frames=0;
    for(const flow of flows){frames+=flow.sent-flow.rendered;if(!flow.eof)break;}
    if(frames>=this.context.sampleRate/4||frames>0&&flows.every(flow=>flow.eof||flow.failed)){
      this.prepared=true;this.setPaused(this.wantedPaused);this.resolveReady();
    }
  }
  private pump(){
    while(!this.closed&&this.running<2&&this.pending.length){
      const window=[...this.flows.values()].slice(0,2),next=this.pending[0];
      // Only the current track and one successor may consume network/RAM.
      // Wait for current output before preloading a successor, except short EOF.
      if(!window.includes(next)||next!==window[0]&&!window[0].eof&&window[0].rendered===0)return;
      const flow=this.pending.shift()!;this.running++;
      this.read(flow).catch(error=>{if(!this.closed){flow.failed=true;flow.error=error instanceof Error?error:new Error(String(error));this.drain(flow);}}).finally(()=>{this.running--;this.pump();});
    }
  }
  private async read(flow:Flow){
    const response=await fetch(flow.url,{signal:this.controller.signal,credentials:"omit",redirect:"error",cache:"no-store"}).catch(error=>{throw new PcmTransportError(String(error));});
    if(!response.ok)throw new PcmTransportError(`音频加载失败（HTTP ${response.status}）`,response.status===429||response.status>=500);
    const format=flow.codec==="aac"?response.headers.get("X-RMusic-Continuous")===`aac-mp4-v1; output_rate=${this.context.sampleRate}`&&response.headers.get("Content-Type")?.startsWith("audio/mp4"):response.headers.get("X-RMusic-PCM")===`f32le; rate=${this.context.sampleRate}; channels=2`;
    if(!format||Number(response.headers.get("X-RMusic-Start"))!==(flow.start??0))throw new Error("音频转换格式或起始位置与输出不匹配");
    if(!response.body)throw new Error("音频响应没有可读取的内容");
    const reader=response.body.getReader();
    this.readers.add(reader);
    try {
      const read=()=>reader.read().catch(error=>{throw new PcmTransportError(String(error));});
      if(flow.codec==="aac"){
        const {decodeAac}=await import("./aac-stream");await decodeAac(read,this.context.sampleRate,this.controller.signal,pcm=>this.send(flow,pcm));
      }else{
        const framer=new PcmFramer();
        while(true){const {value,done}=await read();if(done)break;for(const pcm of framer.push(value))await this.send(flow,pcm);}
        framer.finish();
      }
      this.controller.signal.throwIfAborted();
      if(flow.sent===0&&flow.queuedFrames===0)throw new Error("音源没有可播放的节目帧");
      flow.inputEof=true;this.drain(flow);
    }finally {this.readers.delete(reader);await reader.cancel().catch(()=>{});reader.releaseLock();}
  }
  private async send(flow:Flow,pcm:Float32Array){
    const frames=pcm.length/2;
    const mobile=typeof navigator!=="undefined"&&/Android|iPhone|iPad/i.test(navigator.userAgent);
    const current=[...this.flows.values()][0]===flow;
    const bytes=(current?(mobile?8:32):(mobile?2:8))*1024**2;
    const bufferedFrames=Math.max(frames,Math.min(this.context.sampleRate*30,bytes/8));
    while(!this.closed&&flow.queuedFrames+flow.sent-flow.rendered+frames>bufferedFrames)await new Promise<void>(yes=>this.waiting.add(yes));
    this.controller.signal.throwIfAborted();flow.queued.push(pcm);flow.queuedFrames+=frames;this.drain(flow);
  }
  private drain(flow:Flow){
    // The render thread keeps its original two-second queue. Network lookahead
    // stays on this thread and moves across only as actual output frees space.
    while(flow.queued.length){
      const pcm=flow.queued[0],frames=pcm.length/2;
      if(flow.sent-flow.rendered+frames>this.context.sampleRate*2)break;
      flow.queued.shift();flow.queuedFrames-=frames;flow.sent+=frames;
      this.node.port.postMessage({type:"data",id:flow.id,pcm},[pcm.buffer as ArrayBuffer]);
    }
    if(flow.failed&&!flow.queued.length&&!flow.failurePosted){flow.failurePosted=true;this.node.port.postMessage({type:"failed",id:flow.id,error:flow.error?.message});}
    if(flow.inputEof&&!flow.failed&&!flow.queued.length&&!flow.eof){flow.eof=true;this.node.port.postMessage({type:"end",id:flow.id});this.pump();}
    this.maybeReady();
  }
  private fail(error:Error){if(this.closed)return;this.rejectReady(error);void this.stop().then(()=>this.onError(error));}
  stop():Promise<PcmSnapshot|undefined>{
    if(this.stopped)return this.stopped;
    this.closed=true;clearInterval(this.watchdog);this.controller.abort();for(const reader of this.readers)void reader.cancel().catch(()=>{});this.pending=[];this.wake();this.rejectReady(new DOMException("播放已取消","AbortError"));
    this.stopped=new Promise<void>(yes=>{this.finishStop=yes;this.node.port.postMessage({type:"stop"});const timeout=setTimeout(yes,1000);this.finishStop=()=>{clearTimeout(timeout);yes();};}).then(()=>{
      this.node.disconnect();this.node.port.onmessage=null;this.node.onprocessorerror=null;this.node.port.close();this.flows.clear();return this.last;
    });
    return this.stopped;
  }
}
