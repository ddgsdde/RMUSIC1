export interface PcmSourceState {id:string;frames:number;buffered:number;endFrame?:number}
export interface PcmSpan {id:string;start:number;end:number;endFrame:number}
export interface PcmProcessing {gain:number;startFrame:number;durationFrames:number;fadeInFrames:number;fadeOutFrames:number}
export interface PcmSnapshot {sources:PcmSourceState[];ended:PcmSourceState[];spans:PcmSpan[];frames:number;outputFrames:number;underrunFrames:number;finished:boolean}
interface Source extends PcmSourceState {chunks:Float32Array[];offset:number;eof:boolean;lastFrame:number;processing?:PcmProcessing;ramp?:{gain:number;target:number;remaining:number};error?:string}

// One bounded queue on the audio thread; a source boundary may occur inside a render block.
export class PcmRenderer {
  readonly capacity:number;
  private sources:Source[]=[];
  private ended:PcmSourceState[]=[];
  private spans:PcmSpan[]=[];
  private frames=0;
  private outputFrames=0;
  private underrunFrames=0;
  private sealed=false;
  private closed=false;
  failure?:{id:string;error:string};
  paused=true;
  constructor(rate:number){this.capacity=rate*2;}
  begin(id:string,processing?:PcmProcessing){
    if(this.closed||this.finished||this.sources.length>=9||this.sources.some(source=>source.id===id))throw new Error("播放队列已关闭、重复或超过预载窗口");
    this.sealed=false;
    this.sources.push({id,frames:0,buffered:0,chunks:[],offset:0,eof:false,lastFrame:this.outputFrames,processing});
  }
  enqueue(id:string,pcm:Float32Array){
    const source=this.sources.find(source=>source.id===id);
    if(!source||source.eof||pcm.length===0||pcm.length%2||source.buffered+pcm.length/2>this.capacity)throw new Error("音频帧超出当前预载范围");
    source.chunks.push(pcm);source.buffered+=pcm.length/2;
  }
  gain(id:string,target:number,frames:number){
    const source=this.sources.find(source=>source.id===id);if(!source||!Number.isFinite(target)||target<0)return;
    if(source.frames===0&&source.processing){source.processing={...source.processing,gain:target};source.ramp=undefined;return;}
    source.ramp={gain:source.ramp?.gain??source.processing?.gain??1,target,remaining:Math.max(1,Math.round(frames))};
  }
  end(id:string){
    const source=this.sources.find(source=>source.id===id);
    if(!source)throw new Error("音源已经退出播放队列");
    source.eof=true;
  }
  fail(id:string,error:string){
    const source=this.sources.find(source=>source.id===id);if(!source)return;
    source.error=error;
    if(source===this.sources[0]&&source.buffered===0)this.failure={id,error};
  }
  seal(){this.sealed=true;}
  close(){this.closed=true;this.sources=[];}
  private advance(){
    while(this.sources[0]?.eof&&this.sources[0].buffered===0){
      const source=this.sources.shift()!;
      this.ended.push({id:source.id,frames:source.frames,buffered:0,endFrame:source.lastFrame});
    }
  }
  render(left:Float32Array,right:Float32Array){
    left.fill(0);right.fill(0);this.outputFrames+=left.length;
    if(this.closed)return;
    this.advance();
    if(this.sources[0]?.error&&this.sources[0].buffered===0)this.failure={id:this.sources[0].id,error:this.sources[0].error};
    if(this.paused||this.failure)return;
    let position=0;
    while(position<left.length){
      this.advance();const source=this.sources[0];
      if(!source?.buffered){
        if(source?.error){this.failure={id:source.id,error:source.error};break;}
        if(source||!this.sealed)this.underrunFrames+=left.length-position;
        break;
      }
      const chunk=source.chunks[0],count=Math.min(left.length-position,chunk.length/2-source.offset);
      for(let i=0;i<count;i++){
        let gain=source.processing?.gain??1;
        if(source.ramp){const ramp=source.ramp;if(ramp.remaining>0){ramp.gain+=(ramp.target-ramp.gain)/ramp.remaining;ramp.remaining--;}gain=ramp.gain;}
        if(source.processing){
          const p=source.processing,time=p.startFrame+source.frames+i;
          if(p.fadeInFrames)gain*=Math.sin(Math.min(1,time/p.fadeInFrames)*Math.PI/2);
          if(p.fadeOutFrames&&p.durationFrames>0)gain*=Math.sin(Math.min(1,Math.max(0,(p.durationFrames-time)/p.fadeOutFrames))*Math.PI/2);
        }
        left[position+i]=chunk[(source.offset+i)*2]*gain;right[position+i]=chunk[(source.offset+i)*2+1]*gain;
      }
      const endFrame=this.outputFrames-left.length+position+count,span=this.spans.at(-1);
      if(span?.id===source.id&&span.end===source.frames&&span.endFrame===endFrame-count){span.end+=count;span.endFrame=endFrame;}
      else this.spans.push({id:source.id,start:source.frames,end:source.frames+count,endFrame});
      source.offset+=count;source.frames+=count;source.buffered-=count;this.frames+=count;position+=count;
      source.lastFrame=this.outputFrames-left.length+position;
      if(source.offset===chunk.length/2){source.chunks.shift();source.offset=0;}
    }
    this.advance();
  }
  get finished(){return this.closed||this.sealed&&this.sources.length===0;}
  get hasEnded(){return this.ended.length>0;}
  snapshot():PcmSnapshot {
    const ended=this.ended,spans=this.spans;this.ended=[];this.spans=[];
    return {sources:this.sources.map(({id,frames,buffered})=>({id,frames,buffered})),ended,spans,frames:this.frames,outputFrames:this.outputFrames,underrunFrames:this.underrunFrames,finished:this.finished};
  }
}

// HTTP chunks need not end on a float or stereo-frame boundary.
export class PcmFramer {
  private tail=new Uint8Array(0);
  *push(bytes:Uint8Array):Generator<Float32Array> {
    const joined=new Uint8Array(this.tail.length+bytes.length);joined.set(this.tail);joined.set(bytes,this.tail.length);
    const complete=joined.length-joined.length%8;
    this.tail=joined.slice(complete);
    const view=new DataView(joined.buffer);
    for(let offset=0;offset<complete;){
      const size=Math.min(4096*8,complete-offset),pcm=new Float32Array(size/4);
      for(let i=0;i<pcm.length;i++)pcm[i]=view.getFloat32(offset+i*4,true);
      offset+=size;yield pcm;
    }
  }
  finish(){if(this.tail.length)throw new Error("音频流结束时缺少完整节目帧");}
}
