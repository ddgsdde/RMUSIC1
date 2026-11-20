// @vitest-environment jsdom
import {act,type ComponentProps} from "react";
import {createRoot} from "react-dom/client";
import {expect,it,vi} from "vitest";
import {RMusicClient} from "@r-music/sdk";
import {Transport} from "./Transport";
import {nativeBufferedEnd} from "./native";
it("waiting for the remote device stops the playing indicator while keeping pause available",async()=>{
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);const host=document.createElement('div');document.body.append(host);const root=createRoot(host),command=vi.fn(async()=>{});
 const track={id:'song',title:'Song',artist:'Artist',album:'Album',duration:60,format:'flac',source:'local',available:true,bit_depth:16,sample_rate:44100,channels:2,bitrate:0,recording_id:'song'};
 const props={api:new RMusicClient('https://core.test',''),zone:{id:'z',name:'Mac',endpoint_id:'e',status:'playing',track,queue:[track],queue_index:0,position:0,updated_at:10000,volume:1,muted:false,repeat:'off',shuffle:false,dsp:{enabled:false,preamp_db:0,balance:0,normalize:false,target_lufs:-14,bands:[]}},position:0,playing:false,waiting:true,busy:false,command,patchZone:async()=>{},seek:()=>{},showPlayer:()=>{},showQueue:()=>{},notify:()=>{},onError:()=>{},showSignal:()=>{},showDetails:()=>{},resolution:undefined,grade:{level:'unknown',label:'待报告',reason:''}} satisfies ComponentProps<typeof Transport>;
 try{await act(async()=>root.render(<Transport {...props}/>));expect(host.textContent).toContain('等待设备播放');const button=host.querySelector<HTMLButtonElement>('.play-button')!;expect(button.disabled).toBe(false);await vi.waitFor(()=>expect(button.querySelector('.spin')).not.toBeNull());await act(async()=>button.click());expect(command).toHaveBeenCalledWith('pause');}finally{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();}
});

it("shows buffered audio for the current Android playback and clears it on replacement", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const track = {id:"song",title:"Song",artist:"Artist",album:"Album",duration:60,format:"flac",source:"local",available:true,bit_depth:16,sample_rate:44100,channels:2,bitrate:0,recording_id:"song"};
  const zone = {id:"travel-e",name:"Phone",endpoint_id:"e",status:"playing",playback_id:"play-1",track,queue:[track],queue_index:0,position:15,updated_at:10000,volume:1,muted:false,repeat:"off",shuffle:false,dsp:{enabled:false,preamp_db:0,balance:0,normalize:false,target_lufs:-14,bands:[]}} satisfies NonNullable<ComponentProps<typeof Transport>["zone"]>;
  const report = {status:"playing",playback_id:"play-1",track_id:"song",buffered_fraction:0.6,buffered_until:26};
  const props = {api:new RMusicClient("https://core.test",""),zone,position:15,playing:true,busy:false,command:async()=>{},patchZone:async()=>{},seek:()=>{},showPlayer:()=>{},showQueue:()=>{},notify:()=>{},onError:()=>{},showSignal:()=>{},showDetails:()=>{},resolution:undefined,grade:{level:"unknown",label:"待报告",reason:""}} satisfies ComponentProps<typeof Transport>;
  try {
    expect(nativeBufferedEnd({...report,buffered_fraction:null,buffered_until:42},zone)).toBe(42);
    expect(nativeBufferedEnd({...report,buffered_fraction:1,buffered_until:75},zone)).toBe(60);
    await act(async () => root.render(<Transport {...props} bufferedEnd={nativeBufferedEnd(report,zone)}/>));
    expect(host.querySelector<HTMLElement>(".mobile-mini-buffered")?.style.width).toBe("60%");
    const replacement = {...zone,playback_id:"play-2"};
    await act(async () => root.render(<Transport {...props} zone={replacement} bufferedEnd={nativeBufferedEnd(report,replacement)}/>));
    expect(host.querySelector(".mobile-mini-buffered")).toBeNull();
  } finally {
    await act(async () => root.unmount());
    host.remove();
    vi.unstubAllGlobals();
  }
});
