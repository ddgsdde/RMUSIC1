import {useEffect,useState} from "react";

/** Android WebView may keep document.hidden=false after its Activity stops. */
export function presentationVisible() {
 if(document.hidden)return false;
 const host=window as Window&{RMusicAppearance?:{isVisible?:()=>boolean}};
 try{return host.RMusicAppearance?.isVisible?.()!==false;}catch{return true;}
}
export function usePresentationVisible() {
 const [visible,setVisible]=useState(presentationVisible);
 useEffect(()=>{
  const changed=()=>setVisible(presentationVisible());
  document.addEventListener("visibilitychange",changed);window.addEventListener("rmusic-activity-visibility",changed);
  changed();return()=>{document.removeEventListener("visibilitychange",changed);window.removeEventListener("rmusic-activity-visibility",changed);};
 },[]);
 return visible;
}
