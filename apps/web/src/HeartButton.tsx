import {useState} from "react";
import {Heart} from "lucide-react";
import {IconButton} from "./components";
import "./HeartButton.css";

export function HeartButton({liked,busy,title,toggle}:{liked:boolean;busy:boolean;title:string;toggle:()=>Promise<void>}){
 const [pending,setPending]=useState<boolean|null>(null),[pulse,setPulse]=useState(0);
 const selected=pending??liked;
 return <IconButton className="icon-button heart-button" label={`${selected?"取消喜欢":"喜欢"} ${title}`} aria-pressed={selected} disabled={busy||pending!==null} data-celebrate={pulse>0} onAnimationEnd={event=>{if(event.animationName==="heart-pop")setPulse(0);}} onClick={()=>{setPending(!liked);if(!liked)setPulse(value=>value+1);void toggle().finally(()=>setPending(null));}}>
  <span className="heart-glyph" key={pulse}><Heart size={19} fill={selected?"currentColor":"none"}/></span>
  {pulse>0&&<span className="heart-sparks" aria-hidden="true">{Array.from({length:6},(_,i)=><i key={i} style={{"--spark-angle":`${i*60-15}deg`} as React.CSSProperties}/>)}</span>}
 </IconButton>;
}
