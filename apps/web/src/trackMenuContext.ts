import {createContext,useContext,useEffect,useRef,type MouseEvent,type PointerEvent} from "react";
import type {Track} from "@r-music/sdk";
export type MenuTrack={id:string;title:string;artist?:string;album?:string}&Partial<Track>;
export type TrackMenuOptions={current?:boolean;recommendationFeedback?:{label:string;apply:()=>Promise<void>}};
export type OpenTrackMenu=(event:MouseEvent,track:MenuTrack,remove?:()=>Promise<void>,options?:TrackMenuOptions)=>void;
export const TrackMenuContext=createContext<OpenTrackMenu|null>(null);
export function useTrackContextMenu(){return useContext(TrackMenuContext);}

export function useTrackMenu(track:MenuTrack|undefined,options?:TrackMenuOptions){
 const open=useTrackContextMenu(),hold=useRef<ReturnType<typeof setTimeout>>(undefined),start=useRef({x:0,y:0}),held=useRef(false);
 const cancel=()=>{clearTimeout(hold.current);hold.current=undefined;};
 useEffect(()=>cancel,[track?.id]);
 return {
  onPointerDown:(event:PointerEvent<HTMLElement>)=>{
   cancel();held.current=false;
   if(!open||!track||event.pointerType==="mouse"||!event.isPrimary)return;
   const target=event.currentTarget,x=event.clientX,y=event.clientY;start.current={x,y};
   hold.current=setTimeout(()=>{held.current=true;target.dispatchEvent(new globalThis.MouseEvent("contextmenu",{bubbles:true,cancelable:true,clientX:x,clientY:y}));},480);
  },
  onPointerMove:(event:PointerEvent)=>{if(Math.hypot(event.clientX-start.current.x,event.clientY-start.current.y)>10)cancel();},
  onPointerUp:cancel,onPointerCancel:cancel,
  onClickCapture:(event:MouseEvent)=>{if(held.current){event.preventDefault();event.stopPropagation();held.current=false;}},
  onContextMenu:(event:MouseEvent)=>{cancel();if(open&&track){held.current=true;open(event,track,undefined,options);}},
 };
}
