import type {Endpoint,Zone} from "@r-music/sdk";

export function remoteOutputPlaying(zone:Zone|undefined,endpoint:Endpoint|undefined,now:number):boolean {
 if(!zone||zone.status!=="playing"||!endpoint||!endpoint.online||zone.buffering)return false;
 if(endpoint.capabilities.http_pull!==true||endpoint.capabilities.continuous_pcm!==true)return true;
 const report=endpoint.playback;
 return !!report&&report.core_playback_id===zone.playback_id&&report.track_id===zone.track?.id&&report.status==="playing"&&(report.frames||0)>0&&typeof report.reported_at==="number"&&now-report.reported_at<5000;
}
