import {useEffect,useState} from "react";
import {LoaderCircle,Pause,Play} from "lucide-react";

export function PlaybackButton({playing,waiting,busy,disabled,command}:{
 playing:boolean;waiting:boolean;busy:boolean;disabled:boolean;
 command:(action:"play"|"pause")=>Promise<void>;
}) {
 const [showWaiting,setShowWaiting]=useState(false);
 useEffect(()=>{
  if(!waiting){setShowWaiting(false);return;}
  const timer=setTimeout(()=>setShowWaiting(true),200);
  return()=>clearTimeout(timer);
 },[waiting]);

 return <button type="button" className="play-button"
  aria-label={waiting?"正在等待设备播放，点击暂停":playing?"暂停":"播放"}
  aria-busy={busy||waiting}
  data-command-pending={busy&&!disabled||undefined}
  disabled={disabled||busy}
  onClick={()=>void command(playing||waiting?"pause":"play")}>
  {waiting&&showWaiting?<LoaderCircle className="spin"/>:playing?<Pause fill="currentColor"/>:<Play fill="currentColor"/>}
 </button>;
}
