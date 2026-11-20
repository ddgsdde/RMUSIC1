import {CatalogSources} from "./CatalogSources";
import {CatalogSort,NewMusic,releaseDate,useReleaseClock} from "./CatalogControls";
import {ArtistAlbums} from "./ArtistAlbums";
import {MediaProfileEditor} from "./MediaProfileEditor";
import {useEffect,useState,useRef} from "react";
import type {RMusicClient,Track} from "@r-music/sdk";
import {Cover,Modal} from "./components";
import {ArrowLeft,Play,Ellipsis,Info,Radio} from "lucide-react";
import {SongMenuButton} from "./SongCollection";
import {useTrackContextMenu} from "./trackMenuContext";
import {CatalogLink,hasAlbum} from "./CatalogLinks";
import {isAndroidClient} from "./native";
import {formatTime} from "./lyrics";
type Profile=Record<string,string|undefined>;
type Platform={source:string;name?:string;items:Track[];page?:number;total?:number;next_page?:number;notice?:string;profile?:Profile;sorts?:string[]};
type Detail={name:string;kind:string;anchor:Track;profile:Profile;provenance:Record<string,string>;field_sources?:Record<string,{source:string;value:string;truncated?:boolean}[]>;local:{items:Track[];total:number;page:number;next_page?:number};platforms:Platform[]};
const facts:Record<string,string>={release_date:"发行",country:"地区",genre:"风格",language:"语言",label:"唱片公司",birthday:"生日",birthplace:"出生地",alias:"别名",company:"公司",foreign_name:"外文名",identity:"身份",instrument:"乐器",enter:"出道",tag:"标签",area:"地区"};
export function CatalogDetail({api,id,kind,initialTracks=[],title,back,play,startRadio,enqueue,details,playingId,canEdit=false}:{api:RMusicClient;id:string;kind:"artist"|"album";initialTracks?:Track[];title?:string;playingId?:string;back?:()=>void;canEdit?:boolean;startRadio?:(track:Track)=>Promise<void>;play:(track:Track,list?:Track[])=>Promise<void>;enqueue:(track:Track)=>Promise<void>;details:(track:Track)=>void}){
 const songMenu=useTrackContextMenu();
 const now=useReleaseClock();const[sort,setSort]=useState("default"),[platformSort,setPlatformSort]=useState("default");
 const[page,setPage]=useState(1),[localPage,setLocalPage]=useState(1),[provider,setProvider]=useState(""),[section,setSection]=useState("all"),[about,setAbout]=useState(false),[revision,setRevision]=useState(0);
 const force=useRef(false);const[editing,setEditing]=useState(false);
 const[data,setData]=useState<Detail>(),[error,setError]=useState(""),[loading,setLoading]=useState(true),[actionError,setActionError]=useState("");
 const path=`/library/catalog/${kind}/${encodeURIComponent(id)}?page=${page}&local_page=${localPage}&provider=${encodeURIComponent(section==="library"?"":provider)}&remote=${section==="library"?"false":"true"}&sort=${sort}&platform_sort=${platformSort}`;
 useEffect(()=>{const abort=new AbortController();let current=true;setLoading(true);setError("");
  const refresh=force.current;force.current=false;
  const load=async()=>{try{
   if(!data||(section==="library"&&!refresh)){const local=await api.request<Detail>(path.replace("remote=true","remote=false"),{signal:abort.signal});if(current)setData(local);}
   if(abort.signal.aborted||(section==="library"&&!refresh))return;
   const target=refresh?path.replace("remote=false","remote=true"):path;
   const full=await api.request<Detail>(target+(refresh?"&refresh=true":""),{signal:abort.signal});if(current)setData(full);
  }catch(e){if(current)setError((e as Error).message);}finally{if(current)setLoading(false);}};
  void load();return()=>{current=false;abort.abort();};
 // Data is a snapshot, not a fetch dependency: tabs and biography expansion reuse it.
 // eslint-disable-next-line react-hooks/exhaustive-deps
 },[api,path,revision]);
 const tracks=data?.local.items??initialTracks,profile=data?.profile??{},anchor=data?.anchor??initialTracks[0];
 const platformIDs=new Set(data?.platforms.flatMap(p=>p.items.map(t=>t.id))||[]);
 const visibleLocal=section==="all"?tracks.filter(t=>!platformIDs.has(t.id)):tracks;
 const playable=Array.from(new Map([...(data?.platforms.flatMap(p=>p.items)||[]),...visibleLocal].filter(t=>t.available).map(t=>[t.id,t])).values());
 const remoteSorting=section!=="library"&&!!data?.platforms.length;
 const sortOptions=remoteSorting?(data?.platforms[0].sorts?.length?data.platforms[0].sorts:["default"]):["default","title","title_desc","release_new","release_old","duration","duration_desc","plays"];
 const changeSort=(value:string)=>{if(remoteSorting){setPlatformSort(value);setPage(1);}if(value!=="hot"){setSort(value);setLocalPage(1);}};
 const act=async(fn:()=>Promise<void>)=>{try{setActionError("");await fn();}catch(e){setActionError((e as Error).message);}};
 const rows=(items:Track[])=>items.map((track,index)=><article className="entity-track" aria-current={playingId===track.id?"true":undefined} key={track.id}>
   <button className="entity-play" aria-label={`播放 ${track.title}`} onClick={()=>void act(()=>play(track,items))}>{isAndroidClient?index+1:playingId===track.id?<i className="record-levels" aria-hidden="true"><b/><b/><b/></i>:String(index+1).padStart(2,"0")}<span>▶</span></button>
   <Cover api={api} track={track}/><div className="entity-track-title"><button title={track.title} onClick={()=>isAndroidClient&&songMenu?void act(()=>play(track,items)):details(track)}>{track.title}</button> <NewMusic date={releaseDate(track)} now={now}/><small><CatalogLink track={track}/>{kind==="artist"&&<> · <CatalogLink track={track} album/></>}</small></div>
   <span className="muted">{track.source==="local"?"本地":"平台"}</span><time title={releaseDate(track)}>{formatTime(track.duration)}{releaseDate(track)&&<small>{releaseDate(track)}</small>}</time>{songMenu?<SongMenuButton track={track} current={playingId===track.id}/>:<button aria-label={`将 ${track.title} 加入队列`} onClick={()=>void act(()=>enqueue(track))}>＋</button>}
 </article>);
 const paging=(current:number,next:number|undefined,change:(n:number)=>void)=>(current>1||next)&&<div className="entity-paging"><button disabled={current===1||loading} onClick={()=>change(current-1)}>上一页</button><small>第 {current} 页</small><button disabled={!next||loading} onClick={()=>change(next!)}>下一页</button></div>;
 const heroActions=<>
    <button className="entity-start" disabled={!playable.length} onClick={()=>{if(playable[0])void act(()=>play(playable[0],playable));}}><Play size={15} fill="currentColor"/>播放</button>
    {startRadio&&<button className="entity-radio" disabled={!playable.length} onClick={()=>{if(playable[0])void act(()=>startRadio(playable[0]));}}><Radio size={15}/>启动电台</button>}
 </>;
 return <section className="catalog-entity" data-entity={kind} aria-label={kind==="artist"?"艺术家详情":"专辑详情"}>
  <div className="entity-topline">{back&&<button className="entity-back" aria-label="返回曲库" onClick={back}>{isAndroidClient?"‹ 返回":<><ArrowLeft size={20}/><span>返回</span></>}</button>}{!isAndroidClient&&<span>{kind==="album"?"专辑":"艺术家"}</span>}<button className="entity-about-button" aria-label="查看资料与来源" onClick={()=>setAbout(true)}>{isAndroidClient?<><Info size={17}/>资料</>:<Ellipsis size={20}/>}</button></div>
  <header className="entity-header">
   {anchor&&(profile.image_url||kind==="album")?<Cover api={api} track={{...anchor,cover_url:profile.image_url||anchor.cover_url}}/>:<div className="entity-portrait">♫</div>}
   <div><small className="entity-eyebrow">{kind==="artist"?"艺术家":anchor?.source==="local"?"本地曲库":"专辑"}</small><h1>{kind==="album"&&anchor&&!hasAlbum(anchor)?"未归属专辑":data?.name||title||(loading?"正在读取资料…":"音乐资料")}</h1>{kind==="album"&&anchor&&<CatalogLink track={anchor}/>}
    <p className="entity-summary">{kind==="album"&&profile.release_date&&<span>{profile.release_date.slice(0,4)} · </span>}{data?`${section==="library"?data.local.total:(data.platforms[0]?.total??data.local.total)} 首歌曲`:loading?"正在读取曲目":""}</p>
    {isAndroidClient&&<div className="entity-android-actions">{heroActions}</div>}
   </div>
  </header>
  {!isAndroidClient&&<div className="entity-hero-actions">{heroActions}</div>}
  <nav className="entity-tabs" aria-label="曲目范围">{[["all","全部曲目"],...(kind==="artist"?[["albums","专辑"]]:[]),...(data?.platforms.length?[["library",`本地曲库 · ${data.local.total}`]]:[])].map(([value,label])=><button key={value} aria-pressed={section===value} onClick={()=>setSection(value)}>{label}</button>)}<div className="entity-actions">{section!=="albums"&&<CatalogSort label={remoteSorting?"平台曲目排序":"本地曲目排序"} value={remoteSorting?platformSort:sort} options={sortOptions} change={changeSort}/>}</div></nav>
  {(error||actionError)&&<p className="notice" role="alert">{error||actionError}</p>}
  {section==="albums"?<ArtistAlbums key={id} api={api} artistID={id}/>:<>
  {section!=="library"&&data?.platforms.map(platform=><section key={platform.source}><div className="section-heading"><h2>{platform.name||platform.source}</h2>{platform.total!==undefined&&<small>共 {platform.total} 首</small>}</div>{platform.notice&&<p className="muted">{platform.notice}</p>}{rows(platform.items)}{paging(platform.page??1,platform.next_page,n=>{setProvider(platform.source);setPage(n);})}</section>)}
  {(section==="library"||(section==="all"&&(!!visibleLocal.length||!!data?.local.next_page)))&&<section>{(!isAndroidClient||!!data?.platforms.length)&&<div className="section-heading"><h2>本地曲库</h2><small>{data?.local.total??tracks.length} 首</small></div>}{rows(visibleLocal)}{!visibleLocal.length&&<p className="muted">本地曲库暂无曲目</p>}{paging(localPage,data?.local.next_page,setLocalPage)}</section>}
  {section!=="library"&&!loading&&data&&!data.platforms.length&&!data.local.total&&<p className="muted">暂无可确认的平台目录</p>}
  </>}
  {about&&<Modal title={kind==="artist"?"艺术家资料":"专辑资料"} onClose={()=>setAbout(false)} wide>
   <h2>{data?.name||title}</h2><dl className="entity-facts">{Object.entries(facts).map(([key,label])=>profile[key]&&<div key={key}><dt>{label}</dt><dd>{profile[key]}</dd></div>)}</dl>
   {profile.description&&<p className="entity-full-biography">{profile.description}</p>}
   <CatalogSources api={api} kind={kind} id={id} sources={data?.field_sources} provenance={data?.provenance} canEdit={canEdit} changed={()=>setRevision(v=>v+1)}/>
   <div className="group-actions">{canEdit&&<button onClick={()=>{setAbout(false);setEditing(true);}}>编辑资料</button>}<button disabled={loading} onClick={()=>{force.current=true;setRevision(v=>v+1);}}>{loading?"正在更新…":"更新资料"}</button></div>
  </Modal>}
  {editing&&<MediaProfileEditor api={api} kind={kind} id={id} close={()=>setEditing(false)} saved={()=>setRevision(v=>v+1)}/>}
 </section>;
}
