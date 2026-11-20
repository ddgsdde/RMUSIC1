import {useTrackSort} from "./TrackSort";
import {useEffect,useState} from "react";
import {Music2,Play,UserRound,ArrowUpRight} from "lucide-react";
import {ApiError,type RMusicClient,type Track} from "@r-music/sdk";
import {Cover} from "./components";
import {SongMenuButton} from "./SongCollection";
import {isAndroidClient} from "./native";
import {CatalogLink} from "./CatalogLinks";
type Entry={id:string;title?:string;name?:string;source:string;image_url?:string;relevance?:number;track:Track};
type Results={items:Track[];artists:Entry[];albums:Entry[];notice?:string};
const empty:Results={items:[],artists:[],albums:[]};
export function SearchPage({api,query,play,browse,details}:{api:RMusicClient;query:string;play:(track:Track,list?:Track[])=>Promise<void>;browse:(kind:"album"|"artist",id:string,name?:string)=>void;details:(track:Track)=>void}){
 const [scope,setScope]=useState<"all"|"local">("all");
 const [local,setLocal]=useState<Results>(),[combined,setCombined]=useState<Results>();
 const [localBusy,setLocalBusy]=useState(false),[cloudBusy,setCloudBusy]=useState(false);
 const [localError,setLocalError]=useState(""),[cloudError,setCloudError]=useState("");
 const [category,setCategory]=useState("all"),[limit,setLimit]=useState(24),[retry,setRetry]=useState(0);
 const term=query.trim();
 useEffect(()=>{
  setLocal(undefined);setLocalError("");setLimit(24);setLocalBusy(!!term);
  if(!term)return;
  const controller=new AbortController();
  const timer=setTimeout(()=>{
   const q=encodeURIComponent(term),options={signal:controller.signal};
   void api.request<Results>(`/library/search?q=${q}`,options).catch(async error=>{
    if(!(error instanceof ApiError)||error.status!==404||controller.signal.aborted)throw error;
    const {items}=await api.request<{items:Track[]}>(`/library/tracks?source=local&limit=100&q=${q}`,options);
    return {...empty,items};
   })
    .then(value=>{if(!controller.signal.aborted)setLocal({...empty,...value});})
    .catch(error=>{if(!controller.signal.aborted)setLocalError(error.message||"本地搜索失败");})
    .finally(()=>{if(!controller.signal.aborted)setLocalBusy(false);});
  },120);
  return()=>{clearTimeout(timer);controller.abort();};
 },[api,term,retry]);
 useEffect(()=>{
  setCombined(undefined);setCloudError("");setCloudBusy(!!term&&scope==="all");
  if(!term||scope!=="all")return;
  const controller=new AbortController();
  const timer=setTimeout(()=>{
   void api.request<Results>(`/search?q=${encodeURIComponent(term)}`,{signal:controller.signal})
    .then(value=>{if(!controller.signal.aborted)setCombined({...empty,...value});})
    .catch(error=>{if(!controller.signal.aborted)setCloudError(error.message||"云端搜索失败");})
    .finally(()=>{if(!controller.signal.aborted)setCloudBusy(false);});
  },300);
  return()=>{clearTimeout(timer);controller.abort();};
 },[api,term,retry,scope]);
 const results=(scope==="all"?combined??local:local)??empty;
 const busy=scope==="local"?localBusy:localBusy&&!combined||cloudBusy;
 const error=scope==="local"?localError:cloudError||(!cloudBusy&&!combined?localError:"");
 const sorted=useTrackSort(results.items);
 const source=(id:string)=>id==="qqmusic"?"QQ 音乐":id==="local"?"本地音源":id;
 const heading=(label:string,key:string)=><div className="section-heading"><h2>{label}</h2>{category==="all"&&<button onClick={()=>{setCategory(key);setLimit(24);}}>查看全部 <ArrowUpRight size={14}/></button>}</div>;
 const catalog=(key:"albums"|"artists",label:string)=>results[key].length>0&&<section>{heading(label,key)}<div className="search-catalog">{results[key].filter(item=>category!=="artists"||(item.relevance??1)>0).slice(0,category==="all"?6:limit).map(item=><article key={item.id}><button className="search-identity" onClick={()=>browse(key==="albums"?"album":"artist",item.id,item.title||item.name)}>{key==="albums"?<Cover api={api} track={item.track}/>:item.image_url?<Cover api={api} track={{title:item.name||"",album:"",cover_url:item.image_url}} className="search-portrait"/>:<span className="search-portrait"><UserRound size={56}/></span>}<strong>{item.title||item.name}</strong></button>{key==="albums"&&<small><CatalogLink track={item.track}/></small>}<small className="muted">{source(item.source)}</small></article>)}</div></section>;
 const bestArtist=results.artists.find(a=>a.name?.toLocaleLowerCase()===query.trim().toLocaleLowerCase());
 const showBest=category==="all"&&results.items.length>0&&(!isAndroidClient||!!bestArtist||results.items.length>1);
 const count=category==="songs"?results.items.length:category==="albums"?results.albums.length:results.artists.filter(a=>(a.relevance??1)>0).length;
 const songs=(category==="all"||category==="songs")&&results.items.length>0&&<section>
  {heading("歌曲","songs")}{sorted.control}
  <div className="search-songs">{sorted.tracks.slice(0,category==="all"?8:limit).map(track=><article key={track.id}>
   <button aria-label={`播放 ${track.title}`} onClick={()=>void play(track,sorted.tracks)}><Play size={18}/></button>
   <Cover api={api} track={track}/>
   <div><button className="search-song-title" title={track.title} onClick={()=>isAndroidClient?void play(track,sorted.tracks):details(track)}>{track.title}</button><small><CatalogLink track={track}/></small></div>
   <span className="search-song-album"><CatalogLink track={track} album/></span>
   {!isAndroidClient&&<small className="muted">{source(track.source||"")}</small>}
   <SongMenuButton track={track}/>
  </article>)}</div>
 </section>;
 return <div className="search-results" data-category={category}>
  <h1>{query?`“${query}”`:"搜索音乐"}</h1>
  <nav className="source-tabs" aria-label="搜索分类">{[["all","综合"],["songs","歌曲"],["albums","专辑"],["artists","艺术家"]].map(([key,label])=><button key={key} aria-current={category===key?"page":undefined} className={category===key?"selected":""} onClick={()=>{setCategory(key);setLimit(24);}}>{label}</button>)}</nav>
  <nav className="source-tabs" aria-label="搜索范围">{([["all","全部音源"],["local","本地音源"]] as const).map(([key,label])=><button key={key} aria-pressed={scope===key} className={scope===key?"selected":""} onClick={()=>{setScope(key);setLimit(24);}}>{label}</button>)}</nav>
  {busy&&<p role="status">{scope==="all"&&local?"正在补充云端结果…":"正在搜索…"}</p>}
  {error&&<div role="alert">{error} <button onClick={()=>setRetry(x=>x+1)}>重试</button></div>}
  {results.notice&&<p className="muted">{results.notice}</p>}
  {!busy&&!error&&query&&!results.items.length&&<div className="search-empty"><Music2/><h2>没有找到相关音乐</h2><p>换一个歌曲、专辑或歌手名称试试。</p></div>}
  {showBest&&<section className="search-best">
   {bestArtist?<button onClick={()=>browse("artist",bestArtist.id,bestArtist.name)}><Cover api={api} track={{title:bestArtist.name||"",album:"",cover_url:bestArtist.image_url}} className="search-portrait"/><span><small>最相关 · 艺术家</small><strong>{bestArtist.name}</strong></span><ArrowUpRight/></button>:
    <button onClick={()=>isAndroidClient?void play(results.items[0],sorted.tracks):details(results.items[0])}><Cover api={api} track={results.items[0]}/><span><small>最相关 · 歌曲</small><strong>{results.items[0].title}</strong></span>{isAndroidClient?<Play/>:<ArrowUpRight/>}</button>}
  </section>}
  {isAndroidClient&&songs}
  {(category==="all"||category==="artists")&&catalog("artists","艺术家")}
  {(category==="all"||category==="albums")&&catalog("albums","专辑")}
  {!isAndroidClient&&songs}
  {category==="artists"&&!busy&&results.items.length>0&&count===0&&<p className="muted">没有名称匹配的艺术家</p>}
  {category!=="all"&&count>limit&&<button onClick={()=>setLimit(n=>n+24)}>显示更多</button>}
 </div>;
}
