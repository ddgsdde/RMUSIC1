export function clearAudio(audio:Pick<HTMLMediaElement,"pause"|"removeAttribute"|"load">){
 audio.pause();audio.removeAttribute("src");audio.load();
}
/** A source request may finish after a skip, reconnect or another request. */
export class PlaybackGeneration {
 private generation=0;
 begin(){return ++this.generation;}
 current(value:number){return value===this.generation;}
}
export function fadeGain(position:number,duration:number,fadeInMs=0,fadeOutMs=0){
 const ramp=(time:number,ms:number)=>ms?Math.sin(Math.min(1,Math.max(0,time*1000/ms))*Math.PI/2):1;
 return ramp(position,fadeInMs)*(Number.isFinite(duration)&&duration>0?ramp(duration-position,fadeOutMs):1);
}
export function fadeAutomation(position:number,duration:number,fadeInMs=0,fadeOutMs=0){
 const points=new Set([position]);
 if(fadeInMs)for(let i=0;i<=64;i++)points.add(fadeInMs/1000*i/64);
 if(fadeOutMs&&Number.isFinite(duration)&&duration>0)for(let i=0;i<=64;i++)points.add(duration-fadeOutMs/1000*i/64);
 return [...points].filter(time=>time>=position&&time>=0).sort((a,b)=>a-b).map(time=>({time:time-position,gain:fadeGain(time,duration,fadeInMs,fadeOutMs)}));
}
