/* One audible-position clock and a bounded feature timeline, independent of draw rate. */
'use strict';
function rmusicFramePacer(){let deadline=0,rate=0;return {reset(){deadline=0;},due(now,fps){if(rate!==fps){rate=fps;deadline=0;}const period=1000/Math.max(1,fps);if(deadline&&now+0.75<deadline)return false;deadline=deadline?Math.max(deadline+period,now):now+period;return true;}};}
function rmusicPresentationClock(monotonic=()=>performance.now(),wall=()=>Date.now()) {
  let position=0,at=0,playing=false,duration=0;
  return {
    set(value){const transit=Number.isFinite(value.sentAtMS)?Math.min(250,Math.max(0,wall()-value.sentAtMS))/1000:0;position=Math.max(0,Number(value.position)||0)+(value.playing?transit:0);at=monotonic();playing=!!value.playing;duration=Math.max(0,Number(value.duration)||0);},
    read(){const value=position+(playing?Math.min(2,Math.max(0,monotonic()-at)/1000):0);return duration>0?Math.min(duration,value):value;}
  };
}
function rmusicFeatureTimeline() {
  let frames=[],consumed=-1;
  return {
    clear(){frames=[];consumed=-1;},
    push(input,session){const merged=new Map(frames.map(f=>[f.position,f]));for(const f of (Array.isArray(input)?input:[]).slice(0,24)){if(f.playback_id!==session||!Number.isFinite(f.position)||f.position<0||!Array.isArray(f.bands)||f.bands.length!==8||![...f.bands,f.energy,f.peak].every(n=>Number.isFinite(n)&&n>=0&&n<=255))continue;merged.set(f.position,f);}frames=[...merged.values()].sort((a,b)=>a.position-b.position).slice(-48);},
    sample(position){let index=-1;for(let i=0;i<frames.length&&frames[i].position<=position+0.005;i++)index=i;if(index<0)return null;const frame=frames[index];if(index>0)frames.splice(0,index);return position-frame.position<=0.3?frame:null;},
    drain(position){const due=frames.filter(f=>f.position>consumed&&f.position<=position+0.005);if(due.length)consumed=due[due.length-1].position;return due;},
    size(){return frames.length;}
  };
}
