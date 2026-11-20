import {expect,it} from "vitest";
import type {Endpoint,Zone} from "@r-music/sdk";
import {remoteOutputPlaying} from "./remotePlayback";
const zone={status:"playing",playback_id:"current",track:{id:"song"}} as Zone;
const endpoint={id:"device",name:"Mac",kind:"browser",verified:false,online:true,capabilities:{http_pull:true,continuous_pcm:true},playback:{core_playback_id:"current",track_id:"song",status:"playing",frames:48000,reported_at:10000}} satisfies Endpoint;
it("shows playing only for fresh measured output from the current track instance",()=>{
 expect(remoteOutputPlaying(zone,endpoint,11000)).toBe(true);
 for(const patch of [{core_playback_id:"previous"},{track_id:"previous"},{status:"paused"},{frames:0},{reported_at:5000}])expect(remoteOutputPlaying(zone,{...endpoint,playback:{...endpoint.playback,...patch}},11000)).toBe(false);
 expect(remoteOutputPlaying({...zone,status:"paused"},endpoint,11000)).toBe(false);
 expect(remoteOutputPlaying(zone,{...endpoint,online:false},11000)).toBe(false);
});
it("keeps browser playback on its own zone reports",()=>{
 expect(remoteOutputPlaying(zone,{...endpoint,capabilities:{continuous_pcm:true},playback:undefined},11000)).toBe(true);
});
