import {useEffect,useState} from "react";
import {Search,LibraryBig,ListMusic,Download,Speaker,Music2} from "lucide-react";
import type {Route} from "./App";
export const mobileQuery="(max-width:700px), (pointer:coarse) and (max-height:600px), (pointer:coarse) and (max-width:1100px)";
export function useMobileLayout(){
 const [mobile,setMobile]=useState(()=>window.matchMedia(mobileQuery).matches);
 useEffect(()=>{const media=window.matchMedia(mobileQuery);const change=()=>setMobile(media.matches);media.addEventListener("change",change);return()=>media.removeEventListener("change",change);},[]);
 useEffect(()=>{document.documentElement.dataset.mobileUi=String(mobile);return()=>{delete document.documentElement.dataset.mobileUi;};},[mobile]);
 return mobile;
}
export function MobileSearchBrowse({navigate,offline}:{navigate:(r:Route)=>void;offline:()=>void}){
 const categories=[{title:'你的歌曲',icon:Music2,route:'library'},{title:'发现新音乐',icon:Search,route:'discover'},{title:'专辑',icon:LibraryBig,route:'albums'},{title:'艺术家',icon:Speaker,route:'artists'},{title:'你的歌单',icon:ListMusic,route:'playlists'},{title:'已下载',icon:Download,route:'offline'}] as const;
 return <section className="mobile-search-browse"><h2>浏览你的音乐</h2><div>{categories.map(({title,icon:Icon,route})=><button key={route} onClick={()=>route==='offline'?offline():navigate(route)}><strong>{title}</strong><Icon aria-hidden="true"/></button>)}</div></section>;
}
