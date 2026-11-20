import {PcmRenderer} from "./pcm-renderer";
declare const sampleRate:number;
declare const currentFrame:number;
declare class AudioWorkletProcessor {readonly port:MessagePort;}
declare function registerProcessor(name:string,processor:typeof AudioWorkletProcessor):void;

class ContinuousPcm extends AudioWorkletProcessor {
  private renderer=new PcmRenderer(sampleRate);
  private reportAt=0;
  private closed=false;
  constructor(){
    super();
    this.port.onmessage=({data})=>{
      if(this.closed)return;
      try {
        switch(data.type){
          case "begin":this.renderer.begin(data.id,data.processing);break;
          case "gain":this.renderer.gain(data.id,data.gain,sampleRate/2);break;
          case "data":this.renderer.enqueue(data.id,data.pcm);break;
          case "end":this.renderer.end(data.id);break;
          case "failed":this.renderer.fail(data.id,data.error);break;
          case "seal":this.renderer.seal();break;
          case "pause":this.report();this.renderer.paused=data.paused;break;
          case "stop":this.report("stopped");this.closed=true;this.renderer.close();break;
        }
        if(this.renderer.failure){this.report("error",this.renderer.failure.error);this.closed=true;this.renderer.close();}
      }catch(error){this.report("error",String(error));this.closed=true;this.renderer.close();}
    };
    this.port.postMessage({type:"ready"});
  }
  private report(type="state",error?:string,frame=currentFrame){
    const state=this.renderer.snapshot();
    this.port.postMessage({type,state,error,frame,sourceId:this.renderer.failure?.id});this.reportAt=currentFrame;
    return state;
  }
  process(_inputs:Float32Array[][],outputs:Float32Array[][]){
    if(this.closed)return false;
    this.renderer.render(outputs[0][0],outputs[0][1]);
    if(this.renderer.failure){this.report("error",this.renderer.failure.error,currentFrame+outputs[0][0].length);this.closed=true;this.renderer.close();return false;}
    // At most ten routine messages per second; boundaries are reported in their render quantum.
    if(this.renderer.hasEnded||this.renderer.finished||currentFrame-this.reportAt>=sampleRate/10)this.report("state",undefined,currentFrame+outputs[0][0].length);
    return !this.renderer.finished;
  }
}
registerProcessor("rmusic-continuous-pcm",ContinuousPcm);
