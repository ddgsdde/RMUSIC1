import {equalizerNode} from "./parametric-eq";
import type {Zone} from "@r-music/sdk";

/** Source gain/fades run in the worklet. Filters, balance, volume and spectrum share one graph. */
export class PcmGraph {
  readonly analyser:AnalyserNode;
  private input:GainNode;
  private volume:GainNode;
  private nodes:AudioNode[]=[];
  constructor(private context:AudioContext){
    this.input=context.createGain();this.volume=context.createGain();this.analyser=context.createAnalyser();this.analyser.fftSize=256;
    this.volume.connect(this.analyser);this.analyser.connect(context.destination);
  }
  attach(node:AudioNode){node.connect(this.input);}
  configure(zone:Zone){
    this.input.disconnect();for(const node of this.nodes)node.disconnect();this.nodes=[];
    let tail:AudioNode=this.input;
    if(zone.dsp.enabled){
      for(const band of zone.dsp.bands.filter(band=>band.enabled&&band.frequency<this.context.sampleRate/2)){
        const filter=equalizerNode(this.context,band);
        tail.connect(filter);tail=filter;this.nodes.push(filter);
      }
      const pan=this.context.createStereoPanner();pan.pan.value=zone.dsp.balance;tail.connect(pan);tail=pan;this.nodes.push(pan);
    }
    tail.connect(this.volume);this.gain(zone);
  }
  gain(zone:Zone){this.volume.gain.value=zone.muted?0:zone.volume;}
  close(){this.input.disconnect();for(const node of this.nodes)node.disconnect();this.volume.disconnect();this.analyser.disconnect();}
}
