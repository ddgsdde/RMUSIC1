import {useEffect,useLayoutEffect,useRef,useState} from "react";
import {ArrowUpRight,Disc3,MoreHorizontal,Play,Plus} from "lucide-react";
import type {RMusicClient,Track} from "@r-music/sdk";
import {Cover,quality} from "../components";
import {AlbumArtworkLink,CatalogLink,hasAlbum} from "../CatalogLinks";
import {useTrackContextMenu,useTrackMenu} from "../trackMenuContext";
import {releaseDate} from "../CatalogControls";
import {formatTime} from "../lyrics";
import {usePresentationVisible} from "../presentationVisibility";

type Props={api:RMusicClient;tracks:Track[];reasons?:Record<string,string>;feedback?:(track:Track)=>{label:string;apply:()=>Promise<void>};coverActions?:boolean;canPlay:boolean;play:(track:Track,list:Track[])=>Promise<void>;details:(t:Track)=>void;setup:()=>void;loading:boolean};
export function ListeningDeck(props:Props){
 const {tracks}=props,count=Math.min(tracks.length,6),visible=usePresentationVisible();
 const [pick,setPick]=useState(0),[interacting,setInteracting]=useState(false),[focused,setFocused]=useState(false);
 const rail=useRef<HTMLDivElement>(null),identity=tracks.slice(0,count).map(t=>t.id).join("\n");
 const previousOrder=useRef<string[]>([]);
 const select=(index:number)=>{
  const reduced=window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
  rail.current?.scrollTo({left:index*rail.current.clientWidth,behavior:reduced?"instant":"smooth"});
  setPick(index);
 };
 useLayoutEffect(()=>{
  const next=identity?identity.split("\n"):[];
  const previous=previousOrder.current;
  previousOrder.current=next;
  const node=rail.current;
  if(!node||!node.clientWidth)return;
  const offset=node.scrollLeft/node.clientWidth;
  const visibleIndex=Math.round(offset);
  const id=previous[visibleIndex];
  const match=id?next.indexOf(id):-1;
  const index=match>=0?match:Math.min(visibleIndex,Math.max(0,count-1));
  node.scrollLeft=Math.max(0,index+(match>=0?offset-visibleIndex:0))*node.clientWidth;
  setPick(index);
 },[identity,count]);
 useEffect(()=>{
  if(count<2||!visible||interacting||focused)return;
  const timer=setTimeout(()=>select((pick+1)%count),6000);
  return()=>clearTimeout(timer);
 },[pick,count,visible,interacting,focused,identity]);
 return <section className="studio-featured" aria-label="本期选曲" aria-roledescription="轮播">
  <div className="studio-deck-rail" ref={rail} onScroll={event=>{const node=event.currentTarget;if(node.clientWidth)setPick(Math.round(node.scrollLeft/node.clientWidth));}}
   onPointerDown={()=>setInteracting(true)} onPointerUp={()=>setInteracting(false)} onPointerCancel={()=>setInteracting(false)}
   onPointerEnter={event=>{if(event.pointerType==="mouse")setInteracting(true);}} onPointerLeave={()=>setInteracting(false)}
   onFocusCapture={()=>setFocused(true)} onBlurCapture={event=>{if(!event.currentTarget.contains(event.relatedTarget))setFocused(false);}}>
   {(count?tracks.slice(0,count):[undefined]).map((track,index)=><FeaturedCard key={track?.id||"empty"} {...props} track={track} index={index} count={count}/>)}
  </div>
  {count>1&&<nav className="studio-deck-dots" aria-label="挑选唱片">{tracks.slice(0,count).map((track,index)=><button key={track.id} onClick={()=>select(index)} aria-pressed={index===pick} aria-label={`选中 ${track.title}`}><i/></button>)}</nav>}
 </section>;
}
function FeaturedCard({api,tracks,reasons,feedback,coverActions,canPlay,play,details,setup,loading,track:t,index,count}:Props&{track?:Track;index:number;count:number}){
 const options=t&&feedback?{recommendationFeedback:feedback(t)}:undefined;
 const menu=useTrackMenu(t,options),openMenu=useTrackContextMenu();
 const album=t?.album&&hasAlbum(t),web=coverActions===false;
 const measured=!web&&t&&t.sample_rate>0&&t.bit_depth>0?quality(t):"";
 const title=t?(web&&album?t.album:t.title):loading?"正在挑选音乐":"从第一张唱片开始";
 const year=web&&t?/^\d{4}/.exec(releaseDate(t))?.[0]:undefined;
 const genre=web&&t?t.provider_data?.genre:undefined;
 const number=(n:number)=>String(n).padStart(2,"0");
 return <article className="studio-deck" aria-label={t?`选曲 ${index+1} / ${count}`:"唱片架"}
  {...(coverActions?{}:web?{onContextMenu:event=>t&&openMenu?.(event,t,undefined,options)}:menu)}>
  <div className="studio-jacket">
   <div className="studio-groove" aria-hidden="true"><i/></div>
   {t?(web?<div className="studio-art record-recommendation-art">
    {album?<AlbumArtworkLink track={t}><Cover api={api} track={t}/></AlbumArtworkLink>:<button className="recommendation-cover" aria-label={`查看 ${t.title}`} onClick={()=>details(t)}><Cover api={api} track={t}/></button>}
    {openMenu&&<button type="button" className="record-cover-menu" aria-label={`精选操作 ${t.title}`} aria-haspopup="menu" onClick={event=>openMenu(event,t,undefined,options)}><MoreHorizontal size={19}/></button>}
   </div>:<button className="studio-art" aria-label={`查看 ${t.title}`} onClick={()=>details(t)} {...(coverActions?menu:{})}><Cover api={api} track={t}/></button>)
   :<div className="studio-art studio-blank" aria-hidden="true"><Disc3/><b>R<span>m</span></b></div>}
  </div>
  <div className="studio-deck-info">
   <span className="studio-rubric"><i/> {t?(web||coverActions?"今日精选":"本期选曲"):"唱片架"} {!coverActions&&count>0&&<small>{number(index+1)} / {number(count)}</small>}</span>
   <h2 title={title}>{title}</h2>
   <p className="studio-deck-byline">{t?<><CatalogLink track={t}/>{year&&<span> · {year}</span>}{typeof genre==="string"&&genre&&<span> · {genre}</span>}</>:loading?"正在读取你的曲库":"连接音乐源或添加本地目录，开始聆听。"}</p>
   {web&&t&&<div className="studio-feature-notes">
    {album&&t.title!==title&&<p className="studio-feature-track" title={t.title}>选曲 · {t.title}</p>}
    {reasons?.[t.id]&&<p className="studio-feature-reason" title={reasons[t.id]}>{reasons[t.id]}</p>}
   </div>}
   {!web&&t&&<div className="studio-facts">{measured&&<span>{measured}</span>}{t.duration>0&&<time>{formatTime(t.duration)}</time>}</div>}
   <div className="studio-deck-actions">{t?<><button className="studio-play" aria-label={`播放精选 ${t.title}`} disabled={!canPlay} onClick={()=>play(t,tracks)}><Play size={15} fill="currentColor"/>播放选曲</button>{!web&&<button className="studio-detail" onClick={()=>details(t)}>歌曲详情<ArrowUpRight size={16}/></button>}</>:!loading&&<button className="studio-play" onClick={setup}><Plus size={17}/>连接音乐源</button>}</div>
  </div>
 </article>;
}
