import {useEffect,useRef} from "react";
import type {RMusicClient,Zone} from "@r-music/sdk";
import {isNative} from "./native";
/** Share one existing output tap only while a remote viewer has an active Core lease. */
export function useOutputVisualization(api:RMusicClient,zone:Zone|undefined,analyser:React.RefObject<AnalyserNode|null>,position:number|(()=>number)){
 const current=useRef({zone,position});current.current={zone,position};
 useEffect(()=>{
  if(!zone||isNative)return;let until=0,pending=false;const abort=new AbortController();
  const lease=(event:Event)=>{const value=(event as CustomEvent).detail;if(value?.endpoint_id===zone.endpoint_id)until=performance.now()+Math.min(5000,Math.max(0,Number(value.milliseconds)||0));};
  window.addEventListener("rmusic-visualization-lease",lease);
  const timer=setInterval(async()=>{const z=current.current.zone;if(pending||performance.now()>until||!z?.track||!z.playback_id||z.status!=="playing")return;pending=true;try{
   let value:unknown;
   {const node=analyser.current;if(!node)return;const bytes=new Uint8Array(node.frequencyBinCount);node.getByteFrequencyData(bytes);const bands=Array.from({length:8},(_,i)=>{const lo=Math.floor((2**(i/8)-1)*bytes.length),hi=Math.max(lo+1,Math.floor((2**((i+1)/8)-1)*bytes.length));let total=0;for(let j=lo;j<hi;j++)total+=bytes[j];return Math.round(total/(hi-lo));});const energy=Math.round(bands.reduce((a,b)=>a+b,0)/8),at=typeof current.current.position==="function"?current.current.position():current.current.position;value={version:1,core_playback_id:z.playback_id,track_id:z.track.id,position:at,frames:[{playback_id:z.playback_id,position:at,bands,energy,peak:Math.max(...bands)}]};}
   if(!abort.signal.aborted&&value)await api.request(`/zones/${encodeURIComponent(z.id)}/visualization/report`,{method:"POST",body:JSON.stringify(value),signal:abort.signal});
  }catch{/* Visualization failure never interrupts audio. */}finally{pending=false;}},100);
  return()=>{abort.abort();clearInterval(timer);window.removeEventListener("rmusic-visualization-lease",lease);};
 },[api,zone?.id,zone?.endpoint_id,analyser]);
}
