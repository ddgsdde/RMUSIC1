import {useExternalAccount} from "./AccountAccess";
import {MediaProfileEditor} from "./MediaProfileEditor";
import {CatalogLink} from "./CatalogLinks";
import {MusicVersions} from "./music/Versions";
import {SongButtons} from "./SongCollection";
import {useEffect,useRef,useState} from "react";
import {RelatedCatalogPanel} from "./RelatedCatalog";
import {LoaderCircle} from "lucide-react";
import type {RMusicClient,Track,TrackBackground} from "@r-music/sdk";
import {cachedResource,loadResource,useResourceRefresh} from "./resourceCache";
import {MusicIdentityPanel} from "./MusicIdentity";
import {TrackComments} from "./TrackComments";
import {Cover,Modal,quality} from "./components";
export function TrackDetails({api,track,close,play,playExact,enqueue}:{api:RMusicClient;track:Track;close:()=>void;play:(track:Track)=>Promise<void>;playExact?:(track:Track)=>Promise<void>;enqueue:(track:Track)=>Promise<void>}){
  const external=useExternalAccount();
  const [snapshot,setSnapshot]=useState<{api:RMusicClient;id:string;value:TrackBackground}|null>(null);
  const data=snapshot?.api===api&&snapshot.id===track.id?snapshot.value:cachedResource<TrackBackground>(api,"background:"+track.id)||null;
  const[editing,setEditing]=useState(false);
  const [error,setError]=useState("");
  const [tab,setTab]=useState("info"),[refresh,setRefresh]=useState(0);const pulse=useResourceRefresh(60000),stamp=useRef(0);
  const [qualities,setQualities]=useState<{id:string;labels:string[]}|null>(null);
  useEffect(()=>{let active=true;if(track.source!=="local")api.get<{items:{label:string}[]}>(`/providers/${encodeURIComponent(track.source)}/tracks/${encodeURIComponent(track.id)}/qualities`).then(value=>{if(active)setQualities({id:track.id,labels:value.items.map(item=>item.label)});}).catch(()=>{});return()=>{active=false;};},[api,track.id,track.source]);
  useEffect(()=>{if(document.hidden||tab!=="info")return;let active=true;const force=stamp.current!==refresh;stamp.current=refresh;setError("");loadResource(api,"background:"+track.id,()=>api.get<TrackBackground>(`/library/tracks/${encodeURIComponent(track.id)}/background?refresh=${force}`),86400000,force).then(value=>{if(active)setSnapshot({api,id:track.id,value});}).catch(cause=>{if(active)setError(cause.message);});return()=>{active=false;};},[api,track.id,refresh,pulse,tab]);
  const info=data?.background;
  if(editing)return <MediaProfileEditor api={api} kind="track" id={track.id} close={()=>setEditing(false)} saved={()=>setRefresh(v=>v+1)}/>;
  return <Modal title="关于这首歌" onClose={close} wide>
    {!external&&<button onClick={()=>setEditing(true)}>编辑资料</button>}
    <div className={`detail-heading ${tab==="comments"?"comment-heading":""}`}><Cover api={api} track={{...track,cover_url:info?.image_url??track.cover_url}}/><div><h2>{track.title}</h2><p><CatalogLink track={track}/></p><span className="muted"><CatalogLink track={track} album/></span></div></div>
    {tab==="info"&&qualities?.id===track.id&&qualities.labels.length>0&&<p className="detail-source">音源提供：{qualities.labels.join(" · ")}。账号可播放的版本与真实格式在起播时确认。</p>}
    {tab==="info"&&<div className="song-collection-actions"><SongButtons track={track}/></div>}
    <div className="detail-tabs"><button aria-pressed={tab==="info"} onClick={()=>setTab("info")}>歌曲资料</button><button aria-pressed={tab==="related"} onClick={()=>setTab("related")}>平台专辑与歌手</button><button aria-pressed={tab==="versions"} onClick={()=>setTab("versions")}>音乐版本</button>{!external&&<button aria-pressed={tab==="identity"} onClick={()=>setTab("identity")}>身份与指纹</button>}<button aria-pressed={tab==="comments"} onClick={()=>setTab("comments")}>乐评与评论</button>{tab==="info"&&<button className="text-button" onClick={()=>setRefresh(v=>v+1)}>更新资料</button>}</div>
    {tab==="versions"?<MusicVersions key={track.id} api={api} track={track} play={play} playExact={playExact} enqueue={enqueue} initialOpen/>:tab==="identity"?<MusicIdentityPanel key={track.id} api={api} track={track} play={play} playExact={playExact} enqueue={enqueue}/>:tab==="related"?<RelatedCatalogPanel api={api} track={track} play={play} enqueue={enqueue}/>:tab==="comments"?<TrackComments key={track.id} api={api} id={track.id}/>:<>
    <dl className="detail-facts stored-track-facts"><div><dt>曲库记录音质</dt><dd>{quality(track)}</dd></div><div><dt>音源</dt><dd>{track.source==="local"?"本地":track.source}</dd></div><div><dt>时长</dt><dd>{Math.floor((track.duration||0)/60)}:{String(Math.floor((track.duration||0)%60)).padStart(2,"0")}</dd></div><div><dt>曲目位置</dt><dd>{track.disc_number?`CD ${track.disc_number} · `:""}{track.track_number||"未标注"}</dd></div></dl>
    {!data&&!error&&<p className="loading"><LoaderCircle className="spin"/>正在补充歌曲资料…</p>}
    {error&&<p role="alert" className="notice">在线补充资料暂不可用：{error}；曲库文件资料已保留。</p>}
    {data&&!data.matched&&<p className="notice">没有找到可确认的对应版本，保留本地文件资料。</p>}
    {data?.matched&&<div className="song-background">
      <dl className="detail-facts">{[["发行时间",info?.release_date],["流派",info?.genre],["语言",info?.language],["唱片公司",info?.label]].filter(([,value])=>value).map(([name,value])=><div key={name}><dt>{name}</dt><dd>{value}</dd></div>)}</dl>
      {info?.description&&<section><h3>歌曲介绍</h3><p>{info.description}</p></section>}
      {info?.album_description&&<section><h3>专辑背景</h3><p>{info.album_description}</p></section>}
      {!!info?.credits?.length&&<section><h3>创作与制作</h3><dl className="detail-facts">{info.credits.map((credit,i)=><div key={i}><dt>{credit.role}</dt><dd>{credit.names.join("、")}</dd></div>)}</dl></section>}
      {!!data.artists?.length&&<section><h3>关于艺术家</h3>{data.artists.map((artist,i)=><div key={i} className="artist-background"><strong>{artist.name}</strong>{artist.description&&<p>{artist.description}</p>}</div>)}</section>}
      {!info?.description&&!info?.album_description&&!info?.credits?.length&&<p className="muted">当前资料源暂未提供更多背景资料。</p>}
      <p className="detail-source">资料来源：{data.source}{data.fetched_at?` · ${new Date(data.fetched_at*1000).toLocaleDateString()}`:""}{data.mid&&<a href={`https://y.qq.com/n/ryqq/songDetail/${encodeURIComponent(data.mid)}`} target="_blank" rel="noreferrer">查看来源</a>}</p>
    </div>}</>}
  </Modal>;
}
