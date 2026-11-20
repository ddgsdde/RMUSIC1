type Exports=Record<string,(...args:number[])=>number>&{memory:WebAssembly.Memory};
export interface WasmImpulse {sampleRate:number;channels:number;frames:number;channelMask:number;kernels:Float64Array[]}
const maxFrames=4096;

/** Owns one isolated WASM memory. Native pure Rust modules validate and process every sample. */
export class WasmDspRuntime {
  private api:Exports;
  constructor(module:WebAssembly.Module){this.api=new WebAssembly.Instance(module).exports as unknown as Exports;}
  private checked(value:number){
    if(value<=0){const size=this.api.rm_error_len(),pointer=this.api.rm_error_ptr(),bytes=new Uint8Array(this.api.memory.buffer,pointer,size);throw new Error(decodeURIComponent([...bytes].map(value=>"%"+value.toString(16).padStart(2,"0")).join(""))||"WASM 音频处理失败");}return value;
  }
  private buffer<T>(bytes:Uint8Array,action:(id:number)=>T):T {
    const id=this.checked(this.api.rm_buffer_new(bytes.length));
    try{new Uint8Array(this.api.memory.buffer,this.checked(this.api.rm_buffer_ptr(id)),bytes.length).set(bytes);return action(id);}
    finally{this.api.rm_buffer_free(id);}
  }
  private response<T>(bytes:ArrayBuffer,action:(id:number)=>T):T {
    return this.buffer(new Uint8Array(bytes),buffer=>{const id=this.checked(this.api.rm_response_new(buffer));try{return action(id);}finally{this.api.rm_response_free(id);}});
  }
  parse(bytes:ArrayBuffer):WasmImpulse {
    return this.response(bytes,id=>{
      const sampleRate=this.checked(this.api.rm_response_rate(id)),channels=this.checked(this.api.rm_response_channels(id)),frames=this.checked(this.api.rm_response_frames(id)),channelMask=this.api.rm_response_mask(id);
      const kernels=Array.from({length:channels},(_,channel)=>new Float64Array(this.api.memory.buffer,this.checked(this.api.rm_response_channel_ptr(id,channel)),frames).slice());
      return {sampleRate,channels,frames,channelMask,kernels};
    });
  }
  // Encode DspConfig as UTF-8 on the main thread; AudioWorklet has no text codec globals.
  processor(config:Uint8Array,rate:number,channels:number,channelMask:number,impulse?:ArrayBuffer,measuredLufs?:number){
    const create=(response:number)=>this.buffer(config,buffer=>this.checked(this.api.rm_processor_new(buffer,rate,channels,channelMask,response,measuredLufs??NaN)));
    const id=impulse?this.response(impulse,create):create(0);return new WasmDspProcessor(this.api,id,channels,value=>this.checked(value));
  }
}
export class WasmDspProcessor {
  constructor(private api:Exports,private id:number,readonly channels:number,private checked:(value:number)=>number){}
  private active(){if(!this.id)throw new Error("音频处理器已关闭");return this.id;}
  process(samples:Float64Array,envelope?:Float64Array){
    const id=this.active(),frames=samples.length/this.channels;
    if(!Number.isInteger(frames)||frames>maxFrames||envelope&&envelope.length!==frames)throw new Error("音频块长度无效或超过预算");
    const pointer=this.checked(this.api.rm_processor_samples_ptr(id));new Float64Array(this.api.memory.buffer,pointer,samples.length).set(samples);
    const gains=new Float64Array(this.api.memory.buffer,this.checked(this.api.rm_processor_envelope_ptr(id)),frames);if(envelope)gains.set(envelope);else gains.fill(1);
    this.checked(this.api.rm_processor_process(id,frames));return new Float64Array(this.api.memory.buffer,pointer,samples.length).slice();
  }
  get tailFrames(){const value=this.api.rm_processor_tail(this.active());if(value<0)this.checked(0);return value;}
  drain(frames=maxFrames){
    if(!Number.isInteger(frames)||frames<0||frames>maxFrames)throw new Error("尾音块长度无效或超过预算");
    const id=this.active(),count=this.api.rm_processor_drain(id,frames);if(count<0)this.checked(0);
    return new Float64Array(this.api.memory.buffer,this.checked(this.api.rm_processor_samples_ptr(id)),count*this.channels).slice();
  }
  get meter(){const id=this.active();return {peak:this.api.rm_processor_peak(id),clippedSamples:this.api.rm_processor_clipped(id)};}
  loudness(measuredLufs?:number){this.checked(this.api.rm_processor_loudness(this.active(),measuredLufs??NaN));}
  close(){if(this.id){this.api.rm_processor_free(this.id);this.id=0;}}
}
