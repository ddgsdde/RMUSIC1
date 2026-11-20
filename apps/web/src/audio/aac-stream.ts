import {createFile,type Sample,type Track} from "mp4box";
interface Descriptor {data?:Uint8Array;findDescriptor:(tag:number)=>Descriptor|undefined}

// Core emits small default-base-is-moof fragments. Keep only one fragment's metadata.
export class Mp4Boxes {
  private tail=new Uint8Array(0);
  *push(value:Uint8Array){
    const bytes=new Uint8Array(this.tail.length+value.length);bytes.set(this.tail);bytes.set(value,this.tail.length);let offset=0;
    while(bytes.length-offset>=8){
      const size=new DataView(bytes.buffer,offset).getUint32(0);
      if(size<8||size>262144)throw new Error("AAC 分片大小无效或超过限额");
      if(bytes.length-offset<size)break;
      yield bytes.slice(offset,offset+size);offset+=size;
    }
    this.tail=bytes.slice(offset);
  }
  finish(){if(this.tail.length)throw new Error("AAC 分片响应不完整");}
}
function join(parts:Uint8Array[]){const bytes=new Uint8Array(parts.reduce((n,p)=>n+p.length,0));let offset=0;for(const part of parts){bytes.set(part,offset);offset+=part.length;}return bytes;}
function kind(box:Uint8Array){return String.fromCharCode(...box.subarray(4,8));}

/** Continuous interpolation retains its phase and final programme length across codec packets. */
export class AacResampler {
  inputFrames=0;
  private outputFrames=0;
  private last?:[number,number];
  constructor(private sourceRate:number,private outputRate:number){}
  push(pcm:Float32Array){
    const start=this.inputFrames,end=start+pcm.length/2,out:number[]=[];
    const sample=(frame:number,channel:number)=>frame<start?this.last![channel]:pcm[(frame-start)*2+channel];
    while(this.outputFrames*this.sourceRate/this.outputRate<end-1){
      const position=this.outputFrames*this.sourceRate/this.outputRate,frame=Math.floor(position),fraction=position-frame;
      for(let channel=0;channel<2;channel++)out.push(sample(frame,channel)*(1-fraction)+sample(frame+1,channel)*fraction);
      this.outputFrames++;
    }
    if(pcm.length)this.last=[pcm[pcm.length-2],pcm[pcm.length-1]];
    this.inputFrames=end;return new Float32Array(out);
  }
  finish(){
    const frames=Math.round(this.inputFrames*this.outputRate/this.sourceRate)-this.outputFrames,pcm=new Float32Array(frames*2);
    for(let i=0;i<frames;i++){pcm[i*2]=this.last![0];pcm[i*2+1]=this.last![1];}
    this.outputFrames+=frames;return pcm;
  }
}
function extract(init:Uint8Array[],fragment:Uint8Array[]){
  const parser=createFile(),samples:Sample[]=[];let track:Track|undefined,error:unknown;
  parser.onError=value=>{error=value;};
  parser.onReady=info=>{if(info.tracks.length!==1||info.audioTracks.length!==1)throw new Error("AAC 响应必须只有一个音频轨道");track=info.audioTracks[0];parser.setExtractionOptions(track.id,null,{nbSamples:1});parser.start();};
  parser.onSamples=(_id,_user,values)=>samples.push(...values);
  const bytes=join([...init,...fragment]),buffer=bytes.buffer as ArrayBuffer&{fileStart:number};buffer.fileStart=0;parser.appendBuffer(buffer);parser.flush();
  if(error||!track||samples.length!==parser.getTrackById(track.id).samples.length)throw new Error("AAC 轨道或节目包不完整");
  return {track,samples};
}

export async function decodeAac(
  read:()=>Promise<ReadableStreamReadResult<Uint8Array>>,outputRate:number,signal:AbortSignal,write:(pcm:Float32Array)=>Promise<void>,
){
  const boxes=new Mp4Boxes(),init:Uint8Array[]=[];let moof:Uint8Array|undefined,decoder:AudioDecoder|undefined,resampler:AacResampler|undefined,recipe="",expectedEnd=0,failure:Error|undefined;
  const decoded:AudioData[]=[];
  const abort=()=>{if(decoder&&decoder.state!=="closed")decoder.close();};signal.addEventListener("abort",abort,{once:true});
  try{
    while(true){
      signal.throwIfAborted();const {value,done}=await read();if(done)break;
      // Bound compressed input even when the browser returns a large network chunk.
      for(let offset=0;offset<value.length;offset+=16384)for(const box of boxes.push(value.subarray(offset,offset+16384))){
        const type=kind(box);
        if(type==="ftyp"||type==="moov"){if(moof||decoder||init.some(p=>kind(p)===type))throw new Error("AAC 初始化信息重复或迟到");init.push(box);continue;}
        if(type==="moof"){if(moof)throw new Error("AAC 分片缺少节目数据");moof=box;continue;}
        if(type!=="mdat")continue;
        if(!moof||init.length!==2)throw new Error("AAC 节目数据缺少初始化或时间戳");
        const {track,samples}=extract(init,[moof,box]);moof=undefined;
        const edit=track.edits?.[0],audio=track.audio;
        if(track.codec!=="mp4a.40.2"||!audio||audio.channel_count!==2||!edit||track.edits!.length!==1||edit.media_rate_integer!==1||edit.media_rate_fraction!==0||edit.media_time<0||edit.media_time>4096)throw new Error("AAC 编码配置或首部裁切信息无效");
        const description=(samples[0]?.description as unknown as {esds?:{esd:Descriptor}})?.esds?.esd.findDescriptor(4)?.findDescriptor(5)?.data;
        if(!description||description.length<2||description.length>64)throw new Error("AAC 解码配置缺失");
        const nextRecipe=JSON.stringify([track.codec,audio.sample_rate,audio.channel_count,edit.media_time,...description]);
        if(recipe&&recipe!==nextRecipe)throw new Error("AAC 流中途改变编码配置");
        if(!decoder){
          const config:AudioDecoderConfig={codec:track.codec,sampleRate:audio.sample_rate,numberOfChannels:2,description:new Uint8Array(description)};
          if(!(await AudioDecoder.isConfigSupported(config)).supported)throw new Error("浏览器不支持 AAC 流式解码");signal.throwIfAborted();
          decoder=new AudioDecoder({output:data=>decoded.push(data),error:error=>{failure=error;}});decoder.configure(config);
          resampler=new AacResampler(audio.sample_rate,outputRate);recipe=nextRecipe;
        }
        for(const sample of samples){
          if(!sample.data||sample.data.length!==sample.size||sample.size>16384||sample.timescale!==audio.sample_rate||!Number.isSafeInteger(sample.cts)||sample.cts!==sample.dts||sample.duration<=0||sample.duration>1024)throw new Error("AAC 节目包或时钟无效");
          // Rounding up these fractional microseconds drops AAC frames in Chromium's decoder.
          // Decode on the nonnegative coding timeline; trim the edit separately. Some decoders
          // normalize negative timestamps to zero and would misattribute the priming packet.
          const timestamp=Math.floor(sample.cts*1e6/sample.timescale);
          decoder.decode(new EncodedAudioChunk({type:"key",timestamp,data:sample.data}));await decoder.flush();signal.throwIfAborted();if(failure)throw failure;
          expectedEnd=Math.max(expectedEnd,sample.cts-edit.media_time+sample.duration);
          while(decoded.length){
            const data=decoded.shift()!;
            try{
              if(data.timestamp!==timestamp||data.sampleRate!==audio.sample_rate||data.numberOfChannels!==2)throw new Error("AAC 解码输出与节目时间戳不匹配");
              const from=Math.max(0,edit.media_time-sample.cts),count=Math.max(0,Math.min(data.numberOfFrames,sample.duration)-from);
              if(!count)continue;
              if(sample.cts-edit.media_time+from!==resampler!.inputFrames)throw new Error("AAC 节目帧不连续");
              const pcm=new Float32Array(data.numberOfFrames*2);data.copyTo(pcm,{planeIndex:0,format:"f32"});
              const output=resampler!.push(pcm.subarray(from*2,(from+count)*2));if(output.length)await write(output);
            }finally{data.close();}
          }
          if(resampler!.inputFrames!==Math.max(0,expectedEnd))throw new Error("AAC 解码缺少节目帧");
        }
      }
    }
    boxes.finish();signal.throwIfAborted();if(moof||!decoder||!resampler||resampler.inputFrames===0)throw new Error("AAC 节目响应未完整结束");
    const tail=resampler.finish();if(tail.length)await write(tail);
  }finally{signal.removeEventListener("abort",abort);abort();for(const data of decoded)data.close();}
}
