import {clientId} from "../clientId";
import type {ListenSnapshot} from "../offlineStats";
import type {Track} from "@r-music/sdk";

/** Source intervals come only from contiguous audio-thread runs, never wall-clock estimates. */
export class PcmListen {
  private value?:ListenSnapshot;
  private saved=0;
  private lastFrame=0;
  constructor(private track:Track,private playbackId:string,private session:string,private rate:number,private origin:number,private baseFrames=0){}
  sample(start:number,end:number,at:number){
    at=Math.round(at);
    start=Math.max(start,this.lastFrame);
    if(end<=start||this.value&&this.value.reason!=="active")return;
    const seconds=(end-start)/this.rate,begin=at-seconds*1000;
    if(!this.value)this.value={id:clientId(),version:0,track_id:this.track.id,title:this.track.title,artist:this.track.artist,duration:this.track.duration,started_at:Math.floor(begin),updated_at:at,position:this.origin,days:{},reason:"active",measured:{playback_id:this.playbackId,listen_session_id:this.session,segments:[]}};
    const value=this.value;
    // A backwards wall clock is not evidence for a different listening date.
    if(at<value.updated_at-1)return;
    this.lastFrame=end;
    let time=begin;
    while(time<at){
      const date=new Date(time).toISOString().slice(0,10),midnight=Date.parse(date+"T00:00:00Z")+86400000,stop=Math.min(at,midnight),lastAt=stop===midnight?stop-1:stop;
      const from=this.origin+(this.baseFrames+start+(end-start)*(time-begin)/(at-begin))/this.rate;
      const to=this.origin+(this.baseFrames+start+(end-start)*(stop-begin)/(at-begin))/this.rate;
      const segments=value.measured!.segments,last=segments.at(-1);
      if(last&&Math.abs(last.end-from)<1e-7&&new Date(last.last_at).toISOString().slice(0,10)===date){last.end=to;last.last_at=lastAt;}
      else segments.push({start:from,end:to,last_at:lastAt});
      const day=value.days[date]||{seconds:0,last_at:lastAt};day.seconds+=seconds*(stop-time)/(at-begin);day.last_at=Math.max(day.last_at,lastAt);value.days[date]=day;time=stop;
    }
    value.position=this.origin+(this.baseFrames+end)/this.rate;value.updated_at=at;
  }
  flush(force=false,at=Date.now()):ListenSnapshot|undefined {
    if(!this.value||!force&&at-this.saved<1000)return;
    this.saved=at;this.value.version++;return structuredClone(this.value);
  }
  finish(reason:ListenSnapshot["reason"],at=Date.now()){
    at=Math.round(at);
    if(!this.value||this.value.reason!=="active")return;
    this.value.reason=reason;this.value.updated_at=Math.max(this.value.updated_at,at);return this.flush(true,at);
  }
}
