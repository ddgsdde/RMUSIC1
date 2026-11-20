import {DailyPlaylist,type Picks} from "./PersonalRecommendations";
import {useTrackContextMenu,useTrackMenu} from "./trackMenuContext";
import {ListeningDeck} from "./studio/ListeningDeck";
import {AlbumArtworkLink,CatalogLink,useCatalogNavigation} from "./CatalogLinks";
import {useEffect,useMemo,useRef,useState} from "react";
import {ArrowRight,Ellipsis,Play,RefreshCw,Speaker} from "lucide-react";
import type {Album,Page,PlatformRecommendations,ProviderManifest,ProviderPlaylist,RecommendationSection,Recommendations,RMusicClient,Track,Zone} from "@r-music/sdk";
import {Cover} from "./components";
import {cachedResource,loadResource} from "./resourceCache";
import {ProviderPlaylistsView} from "./ProviderPlaylists";

type Actions={api:RMusicClient;coverActions?:boolean;play:(track:Track,list?:Track[])=>Promise<void>;showDetails:(track:Track)=>void};
type Props=Actions&{
 plugins:ProviderManifest[];zone?:Zone;enqueue:(track:Track)=>Promise<void>;refreshZones:()=>Promise<unknown>;
 openDaily?:(picks:Picks)=>void;startRadio?:(track:Track)=>Promise<void>;onError:(error:unknown)=>void;goSettings:()=>void;goDevices?:()=>void;goAlbums?:()=>void;external?:boolean;cacheScope?:string;unifiedSources?:boolean;
};

function mergeRecommendations(primary: Track[], cloud: Track[]): Track[] {
 const items = [...primary];
 const ids = new Set(primary.map(track => track.id));
 const recordings = new Set(primary.map(track => track.recording_id).filter(Boolean));
 let insertion = 3;
 for (const track of cloud) {
  if (ids.has(track.id) || track.recording_id && recordings.has(track.recording_id)) continue;
  ids.add(track.id);
  if (track.recording_id) recordings.add(track.recording_id);
  items.splice(insertion, 0, track);
  insertion += 4;
 }
 return items;
}

export function DiscoverView({api,plugins,zone,play,enqueue,goSettings,goDevices,goAlbums,showDetails,refreshZones,onError,external=false,cacheScope="",coverActions=false,unifiedSources=false,openDaily}:Props){
 const [tab,setTab]=useState("discover");
 const [selectedSource,setSource]=useState<"all"|"local"|"cloud">("all");
 const source=unifiedSources?"all":selectedSource;
 const key="radio-discover:"+cacheScope+":"+source;
 const sections=[["discover","为你推荐"],["cloud","云端探索"],["recently-added","新入库"],["rediscover","久未重听"],["new-releases","新发行"]] as const;
 const platformsOpen=tab==="platforms";
 const nav=useCatalogNavigation();
 const albumKey="recent-albums:"+cacheScope;
 const [recent,setRecent]=useState(()=>cachedResource<Page<Album>>(api,albumKey));
 const [feed,setFeed]=useState(()=>({key,result:cachedResource<Recommendations>(api,key)}));
 const result=feed.key===key?feed.result:cachedResource<Recommendations>(api,key);
 const cloudSection=result?.sections?.find(section=>section.id==="cloud");
 const items=useMemo(()=>unifiedSources&&cloudSection
  ?mergeRecommendations(result?.items??[],cloudSection.items)
  :result?.items??[],[unifiedSources,result?.items,cloudSection]);
 const [revision,setRevision]=useState(0),[loading,setLoading]=useState(true),[error,setError]=useState("");
 const refreshed=useRef(0);
 const [playlist,setPlaylist]=useState<{provider:ProviderManifest;item:ProviderPlaylist}|null>(null);
 const [notice,setNotice]=useState("");
 const [reduced,setReduced]=useState<string[]>([]);
 const selectedSection=result?.sections?.find(section=>section.id===tab);
 const platforms=plugins.filter(plugin=>plugin.enabled&&plugin.capabilities.includes(external?"public_recommendations_v1":"recommendations"));
 const actions={api,play,showDetails,coverActions};
 const daily=external?undefined:platforms.find(provider=>provider.id==="qqmusic"||provider.capabilities.includes("daily_recommendations_v1"));
 useEffect(()=>{
  if(!daily||platformsOpen)return;
  const timer=setTimeout(()=>void platformPage(api,daily,cacheScope,1).catch(()=>{}),400);
  return()=>clearTimeout(timer);
 },[api,cacheScope,platformsOpen,daily?.id,daily?.version,daily?.account_context]);
 useEffect(()=>{
  if(platformsOpen)return;
  let active=true;
  const refresh=refreshed.current!==revision;
  refreshed.current=revision;
  const path="/recommendations/discover?limit=24"+(source==="all"?"":"&source="+source);
  setLoading(true);setError("");
  void loadResource(api,key,()=>api.get<Recommendations>(path+"&quick=true"+(refresh?"&refresh=true":"")),30000,refresh).then(async value=>{
   if(active)setFeed({key,result:value});
   if(value.refreshing){
    const updated=await loadResource(api,key,()=>api.get<Recommendations>(path),30000,true);
    if(active)setFeed({key,result:updated});
   }
   if(active)setReduced([]);
  }).catch(cause=>{if(active)setError(cause instanceof Error?cause.message:"推荐暂不可用");}).finally(()=>{if(active)setLoading(false);});
  return()=>{active=false;};
 },[api,key,platformsOpen,revision,source]);
 useEffect(()=>{
  if(platformsOpen||!goAlbums)return;
  let active=true;
  void loadResource(api,albumKey,()=>api.get<Page<Album>>("/library/albums?sort=added&source=local&limit=6"),120000,revision>0).then(value=>{if(active)setRecent(value);}).catch(()=>{});
  return()=>{active=false;};
 },[api,albumKey,platformsOpen,revision,!!goAlbums]);
 async function reduce(track:Track){
  try{
   const less=!reduced.includes(track.id);
   await api.put("/recommendations/feedback/"+encodeURIComponent(track.id),{less});
   setReduced(ids=>less?[...ids,track.id]:ids.filter(id=>id!==track.id));
  }catch(cause){onError(cause);}
 }
 const personalize={reasons:result?.reasons,reduce,reduced,sourceName:(track:Track)=>track.source==="local"?"本地":plugins.find(plugin=>plugin.id===track.source)?.name||"云端"};
 const availableSections=sections.filter(([id])=>id!=="new-releases"||result?.sections?.some(section=>section.id===id))
  .filter(([id])=>!unifiedSources||id!=="cloud")
  .filter(([id])=>source==="all"||source==="local"&&id!=="cloud"||source==="cloud"&&id!=="recently-added");
 const sourceButtons=<div className="discover-source" role="group" aria-label="推荐音源">
  {([["all","全部音源"],["local","本地"],["cloud","云端"]] as const).map(([value,label])=><button key={value} aria-pressed={source===value} onClick={()=>{setSource(value);setTab("discover");setLoading(true);}}>{label}</button>)}
 </div>;
 if(playlist)return <ProviderPlaylistsView api={api} provider={playlist.provider} initialPlaylist={playlist.item} onBack={()=>setPlaylist(null)} zone={zone} play={play} enqueue={enqueue} refreshZones={refreshZones} onError={onError} notify={setNotice} goSettings={goSettings} onImported={()=>setNotice("歌单已导入，可在播放列表中查看")}/>;
 return <div className="discover-view radio-discover">
  {notice&&<p className="notice">{notice}</p>}
  {coverActions?<header className="studio-page-title studio-discover-title">
   <h1>发现音乐</h1><div className="studio-discover-tools">
    <button className="discover-platform-button" aria-pressed={platformsOpen} onClick={()=>setTab(platformsOpen?"discover":"platforms")}>{platformsOpen?"返回发现":"平台推荐"}</button>
    <button className="studio-tool refresh-recommendations" aria-label="更新内容" title="更新内容" disabled={!platformsOpen&&loading} onClick={()=>setRevision(value=>value+1)}><RefreshCw size={17} className={!platformsOpen&&loading?"spin":""}/></button>
   </div>
  </header>:<>
  <header className="studio-page-title studio-discover-title">
   <div><h1>{platformsOpen?"平台推荐":"发现音乐"}<span aria-hidden="true"/></h1></div>
   {goDevices&&<button className="record-mobile-output" aria-label="选择播放设备" onClick={goDevices}><Speaker size={20}/></button>}
  </header>
  <div className="studio-discover-tools">
   <div className="studio-segments record-discover-tabs" aria-label="发现栏目">
    {availableSections.map(([id,label])=><button key={id} aria-pressed={tab===id} onClick={()=>setTab(id)}>{label}</button>)}
   </div>
   <button className="record-platform-link" aria-pressed={platformsOpen} onClick={()=>setTab(platformsOpen?"discover":"platforms")}>{platformsOpen?"返回发现":"平台推荐"}<ArrowRight size={15}/></button>
   <button className="studio-tool refresh-recommendations" aria-label="更新内容" title="更新内容" disabled={!platformsOpen&&loading} onClick={()=>setRevision(value=>value+1)}><RefreshCw size={16} className={!platformsOpen&&loading?"spin":""}/></button>
  </div>
  </>}
  {!platformsOpen?<>
   {coverActions&&<nav className="studio-segments discover-mobile-tabs" aria-label="推荐栏目">
     {availableSections.map(([id,label])=><button key={id} aria-pressed={tab===id} onClick={()=>setTab(id)}>{label}</button>)}
   </nav>}
   {!unifiedSources&&sourceButtons}
   {error&&<p className="notice" role="status">{error}</p>}
   {openDaily&&tab==="discover"&&<DailyPlaylist api={api} play={play} canPlay={!!zone} onError={onError} onOpen={openDaily}/>}
   {!openDaily&&tab==="discover"&&(loading||items.length>0)&&<section className="studio-feature"><ListeningDeck api={api} tracks={items} reasons={result?.reasons} feedback={track=>({label:reduced.includes(track.id)?"撤销减少推荐":"少推荐这首",apply:()=>reduce(track)})} canPlay={!!zone} play={play} details={showDetails} setup={goSettings} loading={loading&&!result} coverActions={coverActions}/></section>}
   {source!=="cloud"&&(tab==="discover"||tab==="recently-added")&&!!recent?.items?.length&&<section className="record-recent">
    <header className="section-heading"><div><h2>最近入库</h2></div><button className="text-button" onClick={goAlbums}>查看全部 <ArrowRight size={16}/></button></header>
    <div className="record-albums">{recent.items.map(album=><button className="record-album" key={album.id} onClick={()=>nav?.browse("album",album.id,album.title)}><Cover api={api} track={{title:album.title,album:album.title,cover_url:album.cover_url}}/><strong title={album.title}>{album.title}</strong><small title={album.artist}>{album.artist}</small></button>)}</div>
   </section>}
   {goDevices&&<button className="record-mobile-platform text-button" onClick={()=>setTab("platforms")}>平台推荐 <ArrowRight size={16}/></button>}
   {tab==="discover"&&items.length>0&&<RecommendationShelf {...actions} {...personalize} section={{id:"personal",title:coverActions?"你的推荐":"为你推荐的曲目",source:result?.source??"local",items}}/>}
   {!unifiedSources&&tab==="discover"&&source==="all"&&cloudSection&&<RecommendationShelf {...actions} {...personalize} section={cloudSection}/>}
   {selectedSection&&<RecommendationShelf {...actions} {...personalize} section={selectedSection}/>}
   {!loading&&!error&&(tab==="discover"?!items.length:!selectedSection&&!(tab==="recently-added"&&recent?.items?.length))&&<p className="muted">{tab==="discover"?(source==="cloud"?"本轮未找到可完整播放的云端曲目。可以更新内容，或浏览平台推荐。":"暂时没有可推荐的歌曲。"):"这个栏目暂时没有曲目。"}</p>}

  </>:<>
   {!platforms.length&&<p className="muted">暂无可用的推荐平台。</p>}
   {platforms.map(provider=><PlatformShelf key={JSON.stringify([cacheScope,provider.id,provider.version,provider.account_context])} {...actions} provider={provider} cacheScope={cacheScope} revision={revision} openPlaylist={item=>setPlaylist({provider,item})}/>)}
  </>}
 </div>;
}

function platformKey(provider:ProviderManifest,scope:string,page:number){
 return JSON.stringify(["platform-recommendations",scope,provider.id,provider.version,provider.account_context,page]);
}
function platformPage(api:RMusicClient,provider:ProviderManifest,scope:string,page:number,refresh=false){
 return loadResource(api,platformKey(provider,scope,page),()=>api.get<PlatformRecommendations>(`/providers/${encodeURIComponent(provider.id)}/recommendations?page=${page}&refresh=${refresh}`),300000,refresh);
}
function PlatformShelf({provider,cacheScope,revision,openPlaylist,...actions}:Actions&{provider:ProviderManifest;cacheScope:string;revision:number;openPlaylist:(item:ProviderPlaylist)=>void}){
 const {api}=actions;
 const key=platformKey(provider,cacheScope,1);
 const [open,setOpen]=useState(true),[pages,setPages]=useState<PlatformRecommendations[]>(()=>{
  const first=cachedResource<PlatformRecommendations>(api,key);return first?[first]:[];
 }),[error,setError]=useState(""),[loading,setLoading]=useState(!pages.length);
 useEffect(()=>{
  if(!open)return;
  let active=true;setLoading(true);setError("");
  void platformPage(api,provider,cacheScope,1,revision>0).then(value=>{if(active)setPages([value]);}).catch(cause=>{if(active)setError(cause instanceof Error?cause.message:"平台推荐暂不可用");}).finally(()=>{if(active)setLoading(false);});
  return()=>{active=false;};
 },[api,key,open,revision]);
 const next=pages.at(-1)?.next_page;
 async function more(number:number){
  setLoading(true);
  try{const value=await platformPage(api,provider,cacheScope,number);setPages(items=>[...items,value]);setError("");}
  catch(cause){setError(cause instanceof Error?cause.message:"无法继续加载");}
  finally{setLoading(false);}
 }
 return <details className="radio-platform" open={open} onToggle={event=>setOpen(event.currentTarget.open)}>
  <summary>{provider.name}<small>{provider.status==="login_required"?"需要登录":""}</small></summary>
  {open&&<>
   {loading&&!pages.length&&<p className="muted">正在加载推荐</p>}
   {error&&<p className="muted" role="status">{error}</p>}
   {pages.flatMap(page=>page.sections.map(section=><RecommendationShelf key={page.page+":"+section.id} {...actions} section={section} openPlaylist={openPlaylist}/>))}
   {!loading&&!error&&pages.length>0&&pages.every(page=>!page.sections.length)&&<p className="muted">这个平台暂无推荐。</p>}
   {next&&<button disabled={loading} onClick={()=>void more(next)}>加载更多</button>}
  </>}
 </details>;
}

type Personalize={reasons?:Record<string,string>;reduced?:string[];reduce?:(track:Track)=>Promise<void>;sourceName?:(track:Track)=>string};
function RecommendationShelf({section,openPlaylist,...actions}:Actions&Personalize&{section:RecommendationSection;openPlaylist?:(item:ProviderPlaylist)=>void}){
 const {api}=actions;
 return <section className="discovery-section">
   <div className="section-heading"><h2>{section.title}</h2></div>
   <div className="recommendation-rail">
    {section.items.map(track=><RecommendationCard key={track.id} track={track} album={section.kind==="albums"} list={section.items} {...actions}/>)}
    {section.playlists?.map(item=><article className="recommendation-album" key={item.id}><button className="recommendation-cover" aria-label={`打开歌单 ${item.name}`} onClick={()=>openPlaylist?.(item)}><Cover api={api} track={{cover_url:item.cover_url,title:item.name,album:item.name}}/></button><button className="recommendation-title" title={item.name} onClick={()=>openPlaylist?.(item)}>{item.name}</button><p>{item.track_count} 首歌曲</p></article>)}
   </div>
 </section>;
}
function RecommendationCard({track,api,album,list,play,showDetails,reasons,reduced,reduce,sourceName,coverActions=false}:Actions&Personalize&{track:Track;album:boolean;list:Track[]}){
 const feedbackLabel=reduced?.includes(track.id)?"撤销减少推荐":"少推荐这首";
 const feedback=reduce?{recommendationFeedback:{label:feedbackLabel,apply:()=>reduce(track)}}:undefined;
 const menu=useTrackMenu(track,feedback),openMenu=useTrackContextMenu();
 const cover=album?<AlbumArtworkLink track={track} coverMenu={coverActions}><Cover api={api} track={track}/></AlbumArtworkLink>
  :<button className="recommendation-cover" onClick={()=>void play(track,list)} aria-label={`播放 ${track.title}`} aria-haspopup={coverActions?"menu":undefined} aria-description={coverActions?"长按打开推荐操作":undefined}><Cover api={api} track={track}/><span className="album-play"><Play fill="currentColor" size={21}/></span></button>;
 return <article className="recommendation-album" onContextMenu={coverActions?undefined:event=>openMenu?.(event,track,undefined,feedback)}>
  {coverActions?<div className="recommendation-cover-gesture" {...menu}>{cover}</div>:<div className="record-recommendation-art">
    {cover}
    {openMenu&&<button type="button" className="record-cover-menu" aria-label={`推荐操作 ${track.title}`} aria-haspopup="menu" onClick={event=>openMenu(event,track,undefined,feedback)}><Ellipsis size={19}/></button>}
  </div>}
  {album?<div className="recommendation-title"><CatalogLink track={track} album/></div>:<button className="recommendation-title" onClick={()=>showDetails(track)}>{track.title}</button>}
  <p><CatalogLink track={track}/></p>{sourceName&&<p className="recommendation-source">{sourceName(track)}</p>}{reasons?.[track.id]&&<p className="muted recommendation-reason" title={reasons[track.id]}>{reasons[track.id]}</p>}
 </article>;
}
