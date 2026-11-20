import {useEffect,useRef,useState} from "react";
import {Volume2,VolumeX} from "lucide-react";
import {IconButton} from "./components";

export function wheelVolume(value:number,delta:number,mode:number){return Math.max(0,Math.min(1,value-(mode===1?delta*.02:Math.max(-20,Math.min(20,delta))*.001)));}
export function VolumeControl({scope,value,muted,disabled=false,muteDisabled=false,label="音量",change,mute}:{scope:string;value:number;muted:boolean;disabled?:boolean;muteDisabled?:boolean;label?:string;change:(value:number)=>Promise<void>;mute:()=>void}){
 const host=useRef<HTMLDivElement>(null),[draft,setDraft]=useState<number|null>(null),latest=useRef<number|null>(null),writing=useRef(false),generation=useRef(0),current=useRef(value),send=useRef(change);
 current.current=draft??value;send.current=change;
 useEffect(()=>{generation.current++;latest.current=null;writing.current=false;setDraft(null);return()=>{generation.current++;};},[scope]);
 const submit=(next:number)=>{const level=Math.max(0,Math.min(1,next));current.current=level;setDraft(level);latest.current=level;if(writing.current)return;writing.current=true;const owner=generation.current,write=send.current;void(async()=>{try{while(owner===generation.current&&latest.current!==null){const next=latest.current;latest.current=null;await write(next);await new Promise(resolve=>setTimeout(resolve,75));}}finally{if(owner===generation.current){writing.current=false;setDraft(null);}}})().catch(()=>{});};
 const submitRef=useRef(submit);submitRef.current=submit;
 useEffect(()=>{const element=host.current;if(!element)return;const wheel=(event:WheelEvent)=>{if(disabled||event.ctrlKey||!event.deltaY)return;event.preventDefault();submitRef.current(wheelVolume(current.current,event.deltaY,event.deltaMode));};element.addEventListener("wheel",wheel,{passive:false});return()=>element.removeEventListener("wheel",wheel);},[disabled]);
 const shown=draft??value;
 return <div className="volume-control" ref={host} title={`${label} ${Math.round(shown*100)}% · 滚轮调节`}><IconButton label={label} disabled={disabled||muteDisabled} onClick={mute}>{muted?<VolumeX size={20}/>:<Volume2 size={20}/>}</IconButton><input type="range" aria-label="音量" min={0} max={1} step={.01} disabled={disabled} value={shown} onChange={event=>submit(Number(event.target.value))}/>{draft!==null&&<output className="volume-feedback">{Math.round(shown*100)}%</output>}</div>;
}
