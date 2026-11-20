import {useEffect,useRef,useState} from "react";
import {presentationVisible} from "./presentationVisibility";

/** Interpolate the playback sample, not word timestamps. Stop after 1.2 s
 * without a fresh sample; buffering/paused state never advances the reader. */
export function useLyricPosition(position:number,playing:boolean,owner:unknown,low=false,remote=false) {
 const sample=useRef({reported:position,position,playing,owner,time:performance.now()});
 const [now,setNow]=useState(()=>performance.now());
 if(sample.current.reported!==position||sample.current.playing!==playing||sample.current.owner!==owner){
  const time=performance.now(),previous=sample.current;
  const predicted=previous.position+(previous.playing?Math.max(0,Math.min(1.2,(time-previous.time)/1000)):0);
  const continuous=remote&&playing&&previous.playing&&previous.owner===owner&&Math.abs(position-predicted)<.6;
  sample.current={reported:position,position:continuous?Math.max(position,predicted):position,playing,owner,time};
 }
 useEffect(()=>{
  if(!playing)return;
  let frame=0,last=0;
  const tick=(time:number)=>{if(time-last>=(low?33:15)){last=time;setNow(time);}frame=requestAnimationFrame(tick);};
  const visibility=()=>{cancelAnimationFrame(frame);frame=0;if(presentationVisible())frame=requestAnimationFrame(tick);};
  visibility();document.addEventListener("visibilitychange",visibility);window.addEventListener("rmusic-activity-visibility",visibility);
  return()=>{cancelAnimationFrame(frame);document.removeEventListener("visibilitychange",visibility);window.removeEventListener("rmusic-activity-visibility",visibility);};
 },[playing,low]);
 return playing?sample.current.position+Math.max(0,Math.min(1.2,(now-sample.current.time)/1000)):position;
}
