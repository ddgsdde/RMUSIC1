import {memo,useCallback,useEffect,useLayoutEffect,useMemo,useRef,useState} from "react";
import {LocateFixed} from "lucide-react";
import type {LyricDocument,RMusicClient,Track} from "@r-music/sdk";
import {Empty} from "./components";
import {AudioLines} from "lucide-react";
import {useLyricPosition} from "./useLyricPosition";
import {cachedLyrics,loadLyrics} from "./lyricResource";
import {activeLyric,parseLyrics,translatedLyrics,timedLyrics,lyricWordProgress,type Word,type LyricLine} from "./lyrics";
const TimedWords=memo(function TimedWords({words,position}:{words:Word[];position:number}){
 return <>{words.map((word,index)=>{const progress=position===Infinity?1:position===-Infinity?0:lyricWordProgress(word,position);return <span key={index} className={`timed-word ${progress>0?"word-active":""}`} style={{"--word-progress":`${progress*100}%`} as React.CSSProperties}>{word.text}</span>;})}</>;
});
export function Lyrics({
  document,
  position,
  offset,
  font,
  mode,
  language,romanization,
  seek,
  loading=false,failed=false,retry,
  playing=false,lowPerformance=false,playbackId,remote=false,
}: {
  document: LyricDocument | null;
  position: number;
  offset: number;
  font: number;
  mode: "original"|"bilingual"|"translation";
  language?:string;romanization?:boolean;
  seek: (position: number) => void;
  loading?:boolean;failed?:boolean;retry?:()=>void;
  playing?:boolean;lowPerformance?:boolean;playbackId?:string;remote?:boolean;
}) {
  const translated=document?.translations?.find(item=>item.id===language)?.text||document?.translations?.[0]?.text||document?.translation;
  const lines = useMemo(()=>timedLyrics(document || {text:""},translated),[document,translated]);
  const hasTiming=useMemo(()=>lines.some(line=>line.time>=0),[lines]);
  const roman=useMemo(()=>romanization&&document?.romanization?translatedLyrics(document.text,document.romanization):[],[document?.text,document?.romanization,romanization]);
  const actualPosition = useLyricPosition(position,playing||false,playbackId??document,lowPerformance,remote) - offset - (document?.offset_ms || 0) / 1000;
  const active = activeLyric(lines, actualPosition);
  const scroll = useRef<HTMLDivElement>(null);
  const [following,setFollowing]=useState(true);
  const followingRef=useRef(following);followingRef.current=following;
  const frame=useRef<number | null>(null);
  const motion=useRef({position:0,target:0,velocity:0,time:0});
  const stopScroll=useCallback(()=>{
    if(frame.current!==null) cancelAnimationFrame(frame.current);
    frame.current=null;
    motion.current.velocity=0;
  },[]);
  const center=useCallback((smooth:boolean)=>{
    const parent=scroll.current, line=parent?.querySelector<HTMLElement>('[data-active="true"]');
    if(!parent||!line) return;
    const start=parent.scrollTop;
    const offset=line.getBoundingClientRect().top-parent.getBoundingClientRect().top;
    const top=Math.max(0,Math.min(parent.scrollHeight-parent.clientHeight,start+offset-(parent.clientHeight-line.offsetHeight)*.4));
    if(!smooth||lowPerformance||matchMedia("(prefers-reduced-motion: reduce)").matches){
      stopScroll();
      parent.scrollTo({top,behavior:"instant"});
      return;
    }
    const state=motion.current;
    state.target=top;
    if(frame.current!==null)return;
    if(Math.abs(top-start)<.5){parent.scrollTo({top,behavior:"instant"});return;}
    state.position=start;
    state.time=performance.now();
    const step=(now:number)=>{
      const dt=Math.max(0,(now-state.time)/1000);
      state.time=now;
      // Critical damping keeps position and velocity continuous when a second line arrives mid-scroll.
      const distance=state.position-state.target, rate=11;
      const decay=Math.exp(-rate*dt), slope=state.velocity+rate*distance;
      state.position=state.target+(distance+slope*dt)*decay;
      state.velocity=(state.velocity-rate*slope*dt)*decay;
      const settled=Math.abs(state.position-state.target)<.35&&Math.abs(state.velocity)<5;
      parent.scrollTo({top:settled?state.target:state.position,behavior:"instant"});
      if(settled){state.velocity=0;frame.current=null;}
      else frame.current=requestAnimationFrame(step);
    };
    frame.current=requestAnimationFrame(step);
  },[stopScroll,lowPerformance]);
  const browse=()=>{stopScroll();followingRef.current=false;setFollowing(false);};
  const seekRef=useRef(seek);seekRef.current=seek;
  const seekLine=useCallback((time:number)=>{setFollowing(true);seekRef.current(Math.max(0,time+offset+(document?.offset_ms||0)/1000));},[offset,document?.offset_ms]);
  useLayoutEffect(()=>setFollowing(true),[document?.text]);
  useLayoutEffect(()=>{
    const parent=scroll.current;
    if(!parent)return;
    let width=parent.clientWidth,height=parent.clientHeight;
    if(followingRef.current)center(false);
    const observer=new ResizeObserver(()=>{
      if(width===parent.clientWidth&&height===parent.clientHeight)return;
      width=parent.clientWidth;height=parent.clientHeight;
      if(followingRef.current)center(true);
    });
    observer.observe(parent);
    return ()=>{observer.disconnect();stopScroll();};
  },[center,stopScroll,font,mode,romanization,language,document?.text,loading,failed]);
  useLayoutEffect(()=>{if(following)center(true);},[active,center,following,loading,failed]);
  if(loading||failed)return <div className="lyrics-state" role="status"><AudioLines size={32}/><p>{loading?"正在读取歌词…":"歌词暂时未能加载"}</p>{failed&&<button onClick={retry}>重新加载</button>}</div>;
  if (!document?.text)
    return (
      <Empty
        icon={<AudioLines />}
        headline="此刻，让旋律说话"
        description="这首歌还没有歌词。支持本地 LRC、内嵌歌词与音源插件歌词。"
      />
    );
  return (
    <div className="lyrics-reader" data-following={following} data-timed={hasTiming}>
    <div
      className="lyrics-scroll"
      aria-label="歌词"
      ref={scroll}
      onTouchMove={browse}
      onWheel={browse}
      onKeyDown={event=>{if(["ArrowDown","ArrowUp","PageDown","PageUp","Home","End"].includes(event.key))browse();}}
      style={{ "--lyric-font": `${font}px` } as React.CSSProperties}
    >
      {lines.map((line,index)=><LyricRow key={`${index}-${line.time}`} line={line} active={active===index} past={active>index} position={index===active?actualPosition:index<active?Infinity:-Infinity} mode={mode} pronunciation={roman[index]?.translation} onSeek={seekLine}/>)}
    </div>
    {!following&&hasTiming&&<button className="lyrics-resume" onClick={()=>setFollowing(true)}><LocateFixed size={15}/>回到当前歌词</button>}
    </div>
  );
}

const LyricRow=memo(function LyricRow({line,active,past,position,mode,pronunciation,onSeek}:{line:LyricLine;active:boolean;past:boolean;position:number;mode:"original"|"bilingual"|"translation";pronunciation?:string;onSeek:(time:number)=>void}){
 return <button data-active={active} aria-current={active?"true":undefined} className={`lyric-line ${active?"active":""} ${past?"past":""}`} disabled={line.time<0} onClick={()=>onSeek(line.time)}>
  {mode==="translation"&&line.translation?line.translation:line.words?<TimedWords words={line.words} position={position}/>:line.text}
  {mode==="bilingual"&&line.translation&&<small>{line.translation}</small>}{pronunciation&&<small className="lyric-pronunciation">{pronunciation}</small>}
 </button>;
});

export type MiniLyricsOptions = {
 mode:"translation"|"original";
 language?:string;
 follow:boolean;
 offset:number;
 remote:boolean;
};

export function MiniLyricTitle({api,track,position,playbackId,playing,options}:{
 api:RMusicClient;track:Track;position:number;playbackId?:string;playing:boolean;options:MiniLyricsOptions;
}) {
 const [loaded,setLoaded]=useState(()=>({api,document:cachedLyrics(api,track.id)}));
 useEffect(()=>{
  let active=true;
  setLoaded({api,document:cachedLyrics(api,track.id)});
  void loadLyrics(api,track.id).then(document=>{if(active)setLoaded({api,document});}).catch(()=>{});
  return()=>{active=false;};
 },[api,track.id]);
 const document=loaded.api===api?loaded.document:undefined;
 const translation=document?.translations?.find(line=>line.id===options.language)?.text||document?.translations?.[0]?.text||document?.translation;
 const lines=useMemo(()=>document?timedLyrics(document,translation):[],[document,translation]);
 const translationLines=useMemo(()=>translation?parseLyrics(translation).filter(line=>line.words?.length):[],[translation]);
 const offset=options.offset+(document?.offset_ms||0)/1000;
 const chooseWords=(line:LyricLine|undefined,translated:boolean)=>{
  if(!translated)return line?.words;
  return translationLines.find(candidate=>line&&Math.abs(candidate.time-line.time)<.12&&candidate.text===line.translation)?.words;
 };
 const sampled=lines[activeLyric(lines,position-offset)];
 const [overflow,setOverflow]=useState(0);
 const follow=options.follow&&!!sampled;
 const motion=follow&&(overflow>0||!!chooseWords(sampled,options.mode==="translation"&&!!sampled.translation));
 const actual=useLyricPosition(position,playing&&motion,playbackId??track.id,true,options.remote)-offset;
 const index=activeLyric(lines,actual),line=lines[index],next=lines[index+1];
 const translated=options.mode==="translation"&&!!line?.translation;
 const text=(translated?line?.translation:line?.text)||track.title;
 const words=follow?chooseWords(line,translated):undefined;
 const clip=useRef<HTMLElement>(null),row=useRef<HTMLSpanElement>(null);
 useLayoutEffect(()=>{
  const box=clip.current,content=row.current;
  if(!box||!content||!follow){setOverflow(0);return;}
  const measure=()=>setOverflow(Math.max(0,Math.ceil(content.scrollWidth-box.clientWidth)));
  measure();
  const resize=new ResizeObserver(measure);resize.observe(box);resize.observe(content);
  return()=>resize.disconnect();
 },[text,follow]);
 const end=next?.time??track.duration;
 const fraction=line&&end>line.time?Math.max(0,Math.min(1,(actual-line.time)/(end-line.time))):0;
 return <strong ref={clip} className="mini-lyric-title" data-scroll={follow} aria-label={text}
  style={{"--mini-lyric-shift":`${-overflow*fraction}px`} as React.CSSProperties}>
  <span ref={row} className="mini-lyric-text">{words?<TimedWords words={words} position={actual}/>:text}</span>
 </strong>;
}
