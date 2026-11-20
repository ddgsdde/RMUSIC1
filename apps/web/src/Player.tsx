import {PlaybackButton} from "./PlaybackButton";
import {cachedLyrics,loadLyrics} from "./lyricResource";
import {mobileQuery} from "./MobileShell";
import {activeSkin} from "./studio/skins";
import {sceneColor} from "./mobileAppearance";
import {usePlayerDismiss} from "./mobileMotion";
import {usePresentationVisible} from "./presentationVisibility";
import {VolumeControl} from "./VolumeControl";
import {CatalogLink} from "./CatalogLinks";
import "./Player.css";
import "./ListeningRoom.css";
import {extractCoverPalette,fallbackPalette} from "./coverPalette";
import {SongButtons,SongMenuButton} from "./SongCollection";
import { useEffect, useCallback, useLayoutEffect, useMemo, useRef, useState, type RefObject } from "react";
import { Rnd } from "react-rnd";
import {
  ChevronDown,
  ChevronLeft,
  ListMusic,
  Link,
  Languages,
  Check,
  Grip,
  LayoutDashboard,
  Maximize2,
  Minimize2,
  Monitor,
  RotateCcw,
  SkipBack,
  SkipForward,
  Type,
  Volume2,
  Settings2,
  Shuffle,
  Repeat,
  Repeat1,
  X,
} from "lucide-react";
import type {
  LyricDocument,
  AudioInfo,
  Endpoint,
  PlaybackModule,
  RMusicClient,
  Zone,
} from "@r-music/sdk";
import type { Preferences } from "./App";
import { Cover, Empty, IconButton, Modal } from "./components";
import { activeLyric, formatTime, parseLyrics, translatedLyrics } from "./lyrics";
import { Lyrics } from "./PlayerLyrics";
import {SignalPanel} from "./SignalPanel";
import {signalResolution} from "./signalResolution";
import { isNative, setSceneWindowAppearance, type NativePlayback } from "./native";
const labels: Record<string, string> = {
  cover: "专辑封面",
  info: "歌曲信息",
  controls: "播放控制",
  lyrics: "歌词",
  queue: "播放队列",
  zones: "音区",
  signal: "信号链",
  meter: "电平",
  spectrum: "频谱",
};
function layout(preset: string, phone: boolean): PlaybackModule[] {
  const all = Object.keys(labels).map((id) => ({
    id,
    x: 0,
    y: 0,
    w: 40,
    h: 200,
    visible: false,
  }));
  const values: Record<string, Partial<PlaybackModule>> = phone
    ? {
        info: { x: 0, y: 0, w: 100, h: 80, visible: true },
        lyrics: { x: 0, y: 100, w: 100, h: 530, visible: true },
        cover: { x: 0, y: 90, w: 100, h: 330 },
        controls: { x: 0, y: 440, w: 100, h: 100 },
        signal: { x: 0, y: 550, w: 100, h: 200 },
      }
    : {
        cover: { x: 0, y: 0, w: 39, h: 360, visible: true },
        info: { x: 0, y: 380, w: 39, h: 100, visible: true },
        controls: { x: 0, y: 500, w: 39, h: 100, visible: true },
        lyrics: { x: 44, y: 0, w: 56, h: 470, visible: true },
        signal: { x: 44, y: 500, w: 56, h: 205, visible: true },
        queue: { x: 44, y: 0, w: 56, h: 480 },
        zones: { x: 0, y: 620, w: 39, h: 100 },
        meter: { x: 44, y: 500, w: 56, h: 100 },
        spectrum: { x: 44, y: 620, w: 56, h: 150 },
      };
  if (preset === "cover") {
    Object.values(values).forEach((value) => (value.visible = false));
    values.cover = {
      x: phone ? 0 : 25,
      y: 0,
      w: phone ? 100 : 50,
      h: phone ? 330 : 430,
      visible: true,
    };
    values.info = {
      x: phone ? 0 : 25,
      y: phone ? 350 : 450,
      w: phone ? 100 : 50,
      h: 100,
      visible: true,
    };
    values.controls = {
      x: phone ? 0 : 25,
      y: phone ? 460 : 560,
      w: phone ? 100 : 50,
      h: 100,
      visible: true,
    };
  }
  if (preset === "lyrics") {
    Object.values(values).forEach((value) => (value.visible = false));
    values.info = { x: 0, y: 0, w: 100, h: 80, visible: true };
    values.lyrics = {
      x: 0,
      y: 100,
      w: 100,
      h: phone ? 530 : 580,
      visible: true,
    };
  }
  if (preset === "professional") {
    values.lyrics.visible = false;
    values.signal = { x: 44, y: 0, w: 56, h: 220, visible: true };
    values.meter = { x: 44, y: 250, w: 56, h: 90, visible: true };
    values.spectrum = { x: 44, y: 370, w: 56, h: 220, visible: true };
  }
  return all.map((module) => ({ ...module, ...values[module.id] }));
}
export function readableModules(modules:PlaybackModule[]):PlaybackModule[] {
 const result:PlaybackModule[]=[];
 for(const original of modules.filter(m=>m.visible).sort((a,b)=>a.y-b.y||a.x-b.x)){
  const m={...original,h:Math.max(original.h,original.id==="info"?208:original.id==="controls"?118:75)};
  for(const prior of result)if(m.x<prior.x+prior.w-.5&&m.x+m.w>prior.x+.5&&m.y<prior.y+prior.h+16&&m.y+m.h>prior.y)m.y=prior.y+prior.h+16;
  result.push(m);
 }return result;
}
function validLayout(value: unknown): value is PlaybackModule[] {
  return (
    Array.isArray(value) &&
    value.length <= 12 &&
    value.every(
      (m) =>
        m &&
        typeof m.id === "string" &&
        m.id in labels &&
        ["x", "y", "w", "h"].every(
          (key) => typeof m[key] === "number" && Number.isFinite(m[key]),
        ) &&
        m.w >= 10 &&
        m.w <= 100 &&
        m.x >= 0 &&
        m.x + m.w <= 101 &&
        m.y >= 0 &&
        m.h >= 50 &&
        m.h <= 2000 &&
        typeof m.visible === "boolean",
    )
  );
}
export function Player({
  api,
  zone,
  position,
  bufferedEnd,
  androidCoverLyrics=false,
  legacyAndroid=false,
  onSceneReady,
  appearanceActive=true,
  playing,
  waiting=false,
  preferences,
  savePreferences,
  seek,
  command,
  busy=false,
  analyser,
  adaptive,
  nativeState,
  localOutput,
  endpoint,
  outputLabel,
  expanded=false,toggleExpanded,onError,sourceInfo,sourceError,contextRate,onClose,showQueue,showZones,patchZone,onDsp,
}: {
  api: RMusicClient;
  zone?: Zone;
  position: number;
  bufferedEnd?: number;
  androidCoverLyrics?: boolean;
  legacyAndroid?: boolean;
  onSceneReady?:()=>void;
  appearanceActive?:boolean;
  playing:boolean;
  waiting?:boolean;
  preferences: Preferences;
  savePreferences: (patch: Partial<Preferences>) => Promise<void>;
  seek: (position: number) => void;
  command: (action: string, body?: unknown) => Promise<void>;
  busy?:boolean;
  analyser: RefObject<AnalyserNode | null>;
  adaptive: number;
  nativeState: NativePlayback;
  localOutput: boolean;
  endpoint?: Endpoint;
  outputLabel?:string;
  onError: (error: unknown) => void;
  onDsp?: () => void;
  sourceInfo:AudioInfo|null;sourceError:string;contextRate:number;
  expanded?:boolean;toggleExpanded?:()=>void;
  onClose?:()=>void;showQueue?:()=>void;showZones?:()=>void;
  patchZone?:(patch:Partial<Zone>)=>Promise<void>;
}) {
  const activeTrack = zone?.track;
  const playback = localOutput ? nativeState : endpoint?.playback;
  const currentPath = playback?.track_id === activeTrack?.id &&
    (!zone?.playback_id || !playback?.core_playback_id || playback.core_playback_id === zone.playback_id) ? playback?.signal_path : undefined;
  const resolution = signalResolution(activeTrack,sourceInfo,currentPath,localOutput && !isNative ? contextRate : 0);
  const [editing, setEditing] = useState(false);
  const [androidLyricsOpen,setAndroidLyricsOpen]=useState(false);
  const [retainAndroidLyrics,setRetainAndroidLyrics]=useState(false);
  useEffect(()=>{
    if(androidLyricsOpen||!retainAndroidLyrics)return;
    const timer=setTimeout(()=>setRetainAndroidLyrics(false),260);
    return()=>clearTimeout(timer);
  },[androidLyricsOpen,retainAndroidLyrics]);
  const coverFirst=androidCoverLyrics&&window.matchMedia(mobileQuery).matches;
  const view=coverFirst?(androidLyricsOpen?"lyrics":"album"):window.matchMedia(mobileQuery).matches?(preferences.player_view==="lyrics"?"lyrics":"album"):(preferences.player_view||"album");
  const closeView=()=>{if(coverFirst&&androidLyricsOpen)setAndroidLyricsOpen(false);else onClose?.();};
  const [signalDetails,setSignalDetails]=useState(false);
  const [options,setOptions]=useState(false);
  const [palette,setPalette]=useState(fallbackPalette);
  const [androidBackdrop,setAndroidBackdrop]=useState({current:"",previous:""});
  const cancelPalette=useRef<()=>void>(undefined);
  const coverUrl=zone?.track?.cover_url;
  useLayoutEffect(()=>{onSceneReady?.();},[onSceneReady]);
  useLayoutEffect(()=>()=>cancelPalette.current?.(),[api,coverUrl]);
  const coverUnavailable=useCallback(()=>{
    cancelPalette.current?.();
    setPalette(fallbackPalette);
    if(legacyAndroid)setAndroidBackdrop(state=>({current:"",previous:state.current}));
  },[legacyAndroid]);
  const coverLoaded=useCallback((image:HTMLImageElement)=>{
    cancelPalette.current?.();
    const read=()=>{
      cancelPalette.current=undefined;
      setPalette(extractCoverPalette(image));
      if(legacyAndroid){
        const thumbnail=document.createElement("canvas");thumbnail.width=96;thumbnail.height=96;
        try {
          const context=thumbnail.getContext("2d");
          if(context){context.drawImage(image,0,0,96,96);const current=thumbnail.toDataURL("image/jpeg",.65);
            setAndroidBackdrop(state=>state.current===current?state:{current,previous:state.current});}
        } catch {/* The theme colours still provide a background. */}
      }
    };
    if(androidCoverLyrics&&typeof requestIdleCallback==="function"){
      const id=requestIdleCallback(read,{timeout:220});cancelPalette.current=()=>cancelIdleCallback(id);
    }else read();
  },[coverUrl,androidCoverLyrics,legacyAndroid]);
  const mobileLayout=window.matchMedia(mobileQuery).matches;
  const dismissGesture=usePlayerDismiss(mobileLayout&&!preferences.low_performance&&(!coverFirst||!androidLyricsOpen),closeView);
  const atmosphere=preferences.player_atmosphere!==false&&!preferences.low_performance;
  const playerSkin=preferences.player_skin||"silk";
  const followsLight=preferences.mode==="system"?matchMedia("(prefers-color-scheme: light)").matches:preferences.mode==="light";
  const sceneLight=playerSkin==="paper"||legacyAndroid&&playerSkin!=="midnight"&&followsLight;
  const baseScene=useMemo(()=>activeSkin(preferences,legacyAndroid)[sceneLight?"light":"dark"],[preferences,legacyAndroid,sceneLight]);
  const sceneTheme=playerSkin==="paper"?{...baseScene,bg:"#f2ede4",surface:"#f2ede4",raised:"#e6ded0",text:"#302b25",muted:"#7e7569",border:"#d6cbbb",accent:"#a54e37",accentText:"#ffffff"}:playerSkin==="midnight"?{...baseScene,bg:"#0b1020",surface:"#0b1020",raised:"#1a2440",text:"#f0f4ff",muted:"#a6b3d1",border:"#2b3857",accent:"#b2caff",accentText:"#101a33"}:legacyAndroid?baseScene:{...baseScene,bg:"#252440",surface:"#252440",raised:"#37344f",text:"#fff2e9",muted:"#cdc3d4",border:"#514860",accent:"#ffbe9a",accentText:"#312139"};
  const mobileBottom=sceneTheme.bg;
  const mobileTop=sceneColor(palette.shade,mobileBottom,sceneLight,atmosphere);
  const statusBar=legacyAndroid?mobileTop:sceneTheme.bg;
  const sceneGlow=sceneColor(palette.base,mobileBottom,sceneLight,atmosphere);
  const playingLabel=waiting?"等待设备播放":playing?"正在播放":zone?.status==="paused"?"已暂停":"待播放";
  const output=outputLabel||zone?.name||"选择播放设备";
  const ownsAppearance=useRef(false);
  useEffect(()=>{
    if(mobileLayout&&appearanceActive){
      ownsAppearance.current=true;setSceneWindowAppearance(statusBar,sceneLight,mobileBottom);
    }else if(ownsAppearance.current){
      ownsAppearance.current=false;setSceneWindowAppearance();
    }
  },[mobileLayout,appearanceActive,statusBar,mobileBottom,sceneLight]);
  useEffect(()=>()=>{if(ownsAppearance.current)setSceneWindowAppearance();},[]);
  const pageVisible=usePresentationVisible();
  const setView=(player_view:"album"|"lyrics"|"custom")=>void savePreferences({player_view}).catch(onError);
  const [breakpoint, setBreakpoint] = useState(() =>
    window.matchMedia(mobileQuery).matches
      ? "phone"
      : window.innerWidth < 1100
        ? "tablet"
        : "desktop",
  );
  useEffect(()=>{
    const resize=()=>setBreakpoint(window.matchMedia(mobileQuery).matches?"phone":window.innerWidth<1100?"tablet":"desktop");
    window.addEventListener("resize",resize);return()=>window.removeEventListener("resize",resize);
  },[]);
  const [preset] = useState("mixed");
  const [modules, setModules] = useState<PlaybackModule[]>(() => {
    const name =
      window.matchMedia(mobileQuery).matches
        ? "phone"
        : window.innerWidth < 1100
          ? "tablet"
          : "desktop";
    const saved = preferences.layouts?.[name];
    return validLayout(saved) ? saved : layout("mixed", name === "phone");
  });
  const [width, setWidth] = useState(800);
  const [lyrics, setLyrics] = useState<LyricDocument | null>(null);
  const [lyricsLoading,setLyricsLoading]=useState(false),[lyricsFailed,setLyricsFailed]=useState(false),[lyricsRevision,setLyricsRevision]=useState(0);
  const [lyricSettings, setLyricSettings] = useState(false);
  const canvas = useRef<HTMLDivElement>(null);
  const phone = mobileLayout || breakpoint === "phone";
  useEffect(() => {
    if (!canvas.current) return;
    const observer = new ResizeObserver((entries) =>
      setWidth(entries[0].contentRect.width),
    );
    observer.observe(canvas.current);
    return () => observer.disconnect();
  }, [editing,view]);
  useEffect(() => {
    const saved = preferences.layouts?.[breakpoint];
    setModules(validLayout(saved) ? saved : layout(preset, phone));
  }, [breakpoint]);
  useEffect(() => {
    let active = true;
    const saved=zone?.track?cachedLyrics(api,zone.track.id):undefined;
    setLyrics(saved||null);
    setLyricsLoading(!!zone?.track&&!saved);setLyricsFailed(false);
    if (zone?.track)
      loadLyrics(api,zone.track.id,lyricsRevision>0)
        .then((result) => {
          if (active) setLyrics(result);
        })
        .catch(() => {if(active)setLyricsFailed(true);})
        .finally(()=>{if(active)setLyricsLoading(false);});
    return () => {
      active = false;
    };
  }, [api, zone?.track?.id,lyricsRevision]);
  function change(id: string, value: Partial<PlaybackModule>) {
    setModules((current) =>
      current.map((m) => (m.id === id ? { ...m, ...value } : m)),
    );
  }
  async function save() {
    await savePreferences({
      layouts: { ...preferences.layouts, [breakpoint]: modules },
    });
    setEditing(false);
  }
  const visibleModules=editing?modules.filter(m=>m.visible):readableModules(modules);
  useEffect(()=>{
    const key=(event:KeyboardEvent)=>{
      if(event.key!=="Escape"||event.defaultPrevented||editing||lyricSettings||signalDetails||options||
        document.querySelector('[role="menu"],[role="dialog"][aria-modal="true"]')||
        (event.target as HTMLElement)?.closest("input,textarea,select,[contenteditable=true]"))return;
      if(expanded&&toggleExpanded)toggleExpanded();
      else closeView();
    };
    window.addEventListener("keydown",key);
    return ()=>window.removeEventListener("keydown",key);
  },[onClose,coverFirst,androidLyricsOpen,editing,lyricSettings,signalDetails,options,expanded,toggleExpanded]);
  const hasTranslation=!!(lyrics?.translation||lyrics?.translations?.length||parseLyrics(lyrics?.text||"").some(line=>line.translation));
  const lyricMode=preferences.lyric_mode||(preferences.lyric_translation===false?"original":"bilingual");
  const previewTranslation=lyrics?.translations?.find(item=>item.id===preferences.lyric_language)?.text||lyrics?.translations?.[0]?.text||lyrics?.translation;
  const previewLines=useMemo(()=>coverFirst&&lyrics?.text?translatedLyrics(lyrics.text,previewTranslation):[],[coverFirst,lyrics?.text,previewTranslation]);
  const previewIndex=Math.max(0,activeLyric(previewLines,position-((preferences.lyric_offset||0)+(lyrics?.offset_ms||0))/1000));
  const previewText=(index:number)=>{
    const line=previewLines[index];
    return lyricMode==="translation"?line?.translation||line?.text:line?.text;
  };
  const previewFallback=lyricsLoading?"正在读取歌词…":lyricsFailed?"歌词加载失败":"暂无歌词";
  const toggleTranslation=()=>{if(hasTranslation)void savePreferences({lyric_mode:lyricMode==="original"?"bilingual":lyricMode==="bilingual"?"translation":"original"}).catch(onError);};
  useEffect(()=>{const key=(event:KeyboardEvent)=>{if(event.repeat||event.ctrlKey||event.metaKey||event.altKey||(event.target as HTMLElement)?.closest("input,textarea,select,[contenteditable=true]"))return;if(event.key.toLowerCase()==="t"&&hasTranslation){event.preventDefault();toggleTranslation();}};window.addEventListener("keydown",key);return()=>window.removeEventListener("keydown",key);},[hasTranslation,lyricMode,savePreferences,onError]);
  const height = Math.max(
    phone ? 650 : 710,
    ...visibleModules.map((m) => m.y + m.h + 30),
  );
  function renderModule(id: string) {
    switch (id) {
      case "cover":
        return <Cover api={api} track={zone?.track} className="player-cover" onImageLoad={coverLoaded} onImageUnavailable={coverUnavailable}/>;
      case "info":
        return (
          <div className="player-song-info">
            {phone && <Cover api={api} track={zone?.track} onImageLoad={coverLoaded} onImageUnavailable={coverUnavailable}/>}
            <div>
              <h2>{zone?.track?.title || "选择一首，开始聆听"}</h2>
              <p>{zone?.track?<CatalogLink track={zone.track}/>:"你的音乐，随时在这里"}</p>
              <span>{zone?.track&&<CatalogLink track={zone.track} album/>}</span><div className="song-collection-actions"><SongButtons track={zone?.track} current/></div>
            </div>
          </div>
        );
      case "lyrics":
        return (
          <Lyrics key={zone?.track?.id}
            document={lyrics}
            loading={lyricsLoading}
            failed={lyricsFailed}
            retry={()=>setLyricsRevision(value=>value+1)}
            position={position}
            playbackId={zone?.playback_id||zone?.track?.id}
            remote={!localOutput}
            playing={playing&&(!coverFirst||androidLyricsOpen)}
            lowPerformance={preferences.low_performance}
            offset={(preferences.lyric_offset || 0) / 1000}
            font={preferences.lyric_font || 30}
            mode={preferences.lyric_mode||(preferences.lyric_translation===false?"original":"bilingual")}
            language={preferences.lyric_language}
            romanization={preferences.lyric_romanization!==false}
            seek={seek}
          />
        );
      case "controls":
        return (
          <div className="module-transport">
            <IconButton
              label="上一首"
              disabled={!zone?.track||busy}
              onClick={() => command("previous")}
            >
              <SkipBack fill="currentColor" />
            </IconButton>
            <PlaybackButton playing={playing} waiting={waiting} busy={busy} disabled={!zone?.track} command={command}/>
            <IconButton
              label="下一首"
              disabled={!zone?.track||busy}
              onClick={() => command("next")}
            >
              <SkipForward fill="currentColor" />
            </IconButton>
            <span>
              {formatTime(position)} / {formatTime(zone?.track?.duration || 0)}
            </span>
          </div>
        );
      case "signal":
        return <SignalPanel inline zone={zone} info={sourceInfo} error={sourceError} adaptive={adaptive} nativeState={nativeState} localOutput={localOutput} contextRate={contextRate} endpoint={endpoint} onDsp={onDsp}/>;
      case "queue":
        return (
          <div className="module-queue">
            <h3>播放队列</h3>
            {zone?.queue.map((track, index) => (
              <button
                key={`${track.id}-${index}`}
                disabled={busy}
                onClick={() => command("play", { queue_index: index })}
              >
                <span className={index === zone.queue_index ? "selected" : ""}>
                  {track.title}
                </span>
                <small><CatalogLink track={track}/></small>
              </button>
            ))}
          </div>
        );
      case "zones":
        return (
          <div className="module-zone">
            <Monitor />
            <div>
              <strong>{outputLabel||zone?.name || "未选择播放设备"}</strong>
              <span>
                {waiting?"等待设备播放…":playing ? "正在播放" : "等待播放"}
              </span>
            </div>
            <Volume2 size={18} />
            <span>{Math.round((zone?.volume || 0) * 100)}%</span>
          </div>
        );
      case "meter":
        if (!localOutput)
          return (
            <Empty
              headline="设备电平由输出端提供"
              description="当前只显示此设备的实时电平，不混用其他音区数据。"
            />
          );
        if (isNative) return <NativeMeter state={nativeState} />;
        return (
          <Visualizer
            analyser={analyser}
            spectrum={false}
            low={preferences.low_performance}
          />
        );
      case "spectrum":
        if (isNative)
          return (
            <Empty
              headline="原生频谱尚未开放"
              description="原生实时电平可用，频谱数据将在后续版本提供。"
            />
          );
        return (
          <Visualizer
            analyser={analyser}
            spectrum={id === "spectrum"}
            low={preferences.low_performance}
          />
        );
      default:
        return null;
    }
  }
  const caption=<><h1 title={zone?.track?.title}>{zone?.track?.title||"选择一首，开始聆听"}</h1><p>{zone?.track?<CatalogLink track={zone.track}/>:"你的音乐，随时在这里"}</p>{zone?.track&&<div className="listening-album"><CatalogLink track={zone.track} album/></div>}</>;
  return (
    <div
      className={`player-view scene-player listening-room ${phone ? "phone-preview" : ""} ${editing ? "editing" : ""} player-skin-${preferences.player_skin||"silk"} ${preferences.low_performance?"still-player":""}`}
      data-android-lyrics={coverFirst?androidLyricsOpen:undefined}
      data-atmosphere={atmosphere}
      data-mobile-atmosphere={phone&&atmosphere}
      data-background-motion={playing&&pageVisible&&preferences.player_rotate!==false&&!preferences.low_performance}
      data-active={playing}
      data-standard-player={!editing&&view!=="custom"}
      style={{...Object.fromEntries(Object.entries(sceneTheme).flatMap(([key,value])=>[["--"+key,value],["--mobile-"+key,value]])),"--scene-tint":`rgb(${palette.shade.map(n=>Math.round(n*255)).join(",")})`,"--mobile-scene-top":mobileTop,"--mobile-scene-bottom":mobileBottom,"--record-scene-glow":sceneGlow} as React.CSSProperties}
      data-playing={playing&&pageVisible&&preferences.player_rotate!==false&&!preferences.low_performance}
    >
      {legacyAndroid&&atmosphere&&<div className="mobile-player-backdrop" aria-hidden="true">
        {androidBackdrop.previous&&<div className="mobile-blur-reveal backdrop-previous"><img src={androidBackdrop.previous} alt=""/></div>}
        {androidBackdrop.current&&<div className="mobile-blur-reveal" key={androidBackdrop.current}><img src={androidBackdrop.current} alt=""/></div>}
        <div className="mobile-player-glow"/>
      </div>}
      {phone&&<header className="mobile-player-top" {...dismissGesture}>
        <IconButton label={coverFirst&&androidLyricsOpen?"返回封面":"收起播放页"} onClick={closeView}>
          {coverFirst&&androidLyricsOpen?<ChevronLeft/>:<ChevronDown/>}
        </IconButton>
        {coverFirst?(legacyAndroid?<div className="mobile-player-lyric-title" aria-hidden={!androidLyricsOpen}>
          <strong>{zone?.track?.title}</strong><small>{zone?.track?.artist}</small>
        </div>:<div className="record-player-heading">
          <div className="record-player-cover-title" aria-hidden={androidLyricsOpen}><strong>{playingLabel}</strong><button className="text-button" onClick={()=>showZones?.()}>{output}</button></div>
          <div className="mobile-player-lyric-title" aria-hidden={!androidLyricsOpen}><strong>歌词</strong><small>{zone?.track?.title} · {zone?.track?.artist}</small></div>
        </div>):<div className="studio-player-switch" aria-label="播放视图">{(["album","lyrics"] as const).map(tab=><button key={tab} aria-pressed={view===tab} onClick={()=>{setEditing(false);setView(tab);}}>{tab==="album"?"封面":"歌词"}</button>)}</div>}
        <SongMenuButton track={zone?.track} current/><IconButton label="播放页设置" onClick={()=>setOptions(true)}><Settings2/></IconButton>
      </header>}
      <div className="scene-toolbar">
        <div className="scene-navigation">
          {legacyAndroid?<><IconButton label="收起播放页" onClick={()=>onClose?.()}><ChevronLeft/></IconButton><button className="scene-zone" onClick={()=>showZones?.()}><Monitor size={15}/><span>{output}</span><ChevronDown size={13}/></button></>:<button className="text-button scene-back-label" aria-label="收起播放页" onClick={()=>onClose?.()}><ChevronLeft size={19}/>返回</button>}
        </div>
        {!legacyAndroid&&<span className="record-scene-state">{playingLabel}</span>}
        {legacyAndroid&&!coverFirst&&<div className="scene-view-switch" role="group" aria-label="播放视图">{([["album","封面"],["lyrics","歌词"]] as const).map(([id,label])=><button key={id} aria-pressed={view===id&&!editing} onClick={()=>{setEditing(false);setView(id);}}>{label}</button>)}</div>}
        <div className="scene-tools">
          {toggleExpanded&&<IconButton label={expanded?"退出整页播放":"展开至整页"} aria-pressed={expanded} onClick={toggleExpanded}>{expanded?<Minimize2 size={18}/>:<Maximize2 size={18}/>}</IconButton>}
          {!legacyAndroid&&<IconButton label={view==="lyrics"?"切换到封面":"聚焦歌词"} aria-pressed={view==="lyrics"&&!editing} onClick={()=>{setEditing(false);setView(view==="lyrics"?"album":"lyrics");}}><Languages size={18}/></IconButton>}
          <IconButton label="音频链路" onClick={()=>setSignalDetails(true)}><Link size={18}/></IconButton>
          <SongMenuButton track={zone?.track} current/><IconButton label="播放页设置" onClick={()=>setOptions(true)}><Settings2/></IconButton>
        </div>
      </div>
      {editing && (
        <div className="layout-edit-toolbar">
          <div>
            {[
              ["desktop", "桌面"],
              ["tablet", "平板"],
              ["phone", "手机"],
            ].map(([id, label]) => (
              <button
                className={breakpoint === id ? "selected" : ""}
                key={id}
                onClick={() => setBreakpoint(id)}
              >
                {label}
              </button>
            ))}
          </div>
          <div>
            <IconButton
              label="恢复预设布局"
              onClick={() => setModules(layout(preset, phone))}
            >
              <RotateCcw size={17} />
            </IconButton>
            <button
              onClick={() => {
                const saved = preferences.layouts?.[breakpoint];
                setModules(validLayout(saved) ? saved : layout(preset, phone));
                setEditing(false);
              }}
            >
              取消
            </button>
            <button className="primary" onClick={save}>
              <Check size={17} />
              保存布局
            </button>
          </div>
        </div>
      )}
      {!editing&&view!=="custom"?<>
        <div className={`listening-stage ${view==="lyrics"?"lyrics-focus":""}`} aria-label="播放内容">
          <section className="listening-artwork" inert={coverFirst&&androidLyricsOpen||undefined}>
            <div className="listening-eyebrow"><span>{waiting?"等待设备播放":playing?"正在播放":zone?.status==="paused"?"已暂停":"待播放"}</span>{zone?.queue.length?<span>{zone.queue_index+1} / {zone.queue.length}</span>:null}</div>
            <Cover api={api} track={zone?.track} className="listening-cover" onImageLoad={coverLoaded} onImageUnavailable={coverUnavailable}/>
            <div className="listening-caption">{legacyAndroid?caption:<div className="record-track-meta"><div>{caption}</div><SongButtons track={zone?.track} mode="like"/></div>}
              {coverFirst&&zone?.track&&<button className="listening-lyric-preview" aria-label="展开歌词" onClick={()=>{setRetainAndroidLyrics(true);setAndroidLyricsOpen(true);}}>
                <span className="lyric-preview-lines" key={previewIndex}>
                  {previewLines.length>0&&<span aria-hidden="true">{previewText(previewIndex-1)||" "}</span>}
                  <strong className="preview-current">{previewText(previewIndex)||previewFallback}</strong>
                  {previewLines.length>0&&<span aria-hidden="true">{previewText(previewIndex+1)||" "}</span>}
                </span>
              </button>}
            </div>
          </section>
          <section className="listening-lyrics" aria-label="同步歌词" inert={coverFirst&&!androidLyricsOpen||undefined}>
            <div className="listening-lyric-heading"><span>歌词</span><div><button disabled={!hasTranslation} title="T 切换歌词翻译" onClick={toggleTranslation}><Languages size={16}/>{hasTranslation?lyricMode==="original"?"原文":lyricMode==="translation"?"译文":"双语":"无译文"}</button><IconButton label="歌词样式" onClick={()=>setLyricSettings(true)}><Type size={17}/></IconButton></div></div>
            {(!coverFirst||retainAndroidLyrics)&&renderModule("lyrics")}
          </section>
        </div>
        <footer className="listening-footer">
          <div className="listening-progress">
            <time>{formatTime(position)}</time>
            <input aria-label="播放进度" type="range" min={0} max={Math.max(1,zone?.track?.duration||0)} step={0.1}
              value={Math.max(0,Math.min(position,zone?.track?.duration||0))}
              data-buffered={bufferedEnd !== undefined || undefined}
              style={{
                "--progress": `${zone?.track?.duration ? Math.max(0, Math.min(100, 100 * position / zone.track.duration)) : 0}%`,
                "--buffered": `${bufferedEnd !== undefined && zone?.track?.duration ? 100 * bufferedEnd / zone.track.duration : 0}%`,
              } as React.CSSProperties}
              disabled={!zone?.track||busy} onChange={event=>seek(Number(event.target.value))}/>
            <time>{formatTime(zone?.track?.duration||0)}</time>
          </div>
          <div className="listening-control-row"><div className="listening-control-start"><IconButton label="选择播放设备" onClick={()=>showZones?.()}><Monitor size={20}/></IconButton><span>{outputLabel||zone?.name||"选择播放设备"}</span></div><div className="listening-transport"><IconButton label="随机播放" aria-pressed={!!zone?.shuffle} disabled={!zone||!patchZone||busy} onClick={()=>void patchZone?.({shuffle:!zone?.shuffle}).catch(onError)}><Shuffle size={20}/></IconButton>{renderModule("controls")}<IconButton label={`循环：${zone?.repeat==="one"?"单曲":zone?.repeat==="all"?"列表":"关闭"}`} aria-pressed={zone?.repeat==="all"||zone?.repeat==="one"} disabled={!zone||!patchZone||busy} onClick={()=>void patchZone?.({repeat:zone?.repeat==="off"?"all":zone?.repeat==="all"?"one":"off"}).catch(onError)}>{zone?.repeat==="one"?<Repeat1 size={20}/>:<Repeat size={20}/>}</IconButton></div>{legacyAndroid?<div className="listening-mobile-like"><SongButtons track={zone?.track} mode="like"/></div>:<button className="record-player-quality text-button" onClick={()=>setSignalDetails(true)}><i/>{resolution || "音频链路"}</button>}<div className="listening-control-end"><VolumeControl scope={zone?.endpoint_id||""} value={endpoint?.output_config?.volume_control==="device"?(endpoint.device_volume?.value??1):zone?.volume??1} muted={!!zone?.muted} disabled={!zone||endpoint?.output_config?.volume_control==="fixed"} muteDisabled={endpoint?.output_config?.volume_control==="device"} change={async value=>{await patchZone?.({volume:value});}} mute={()=>void patchZone?.({muted:!zone?.muted})}/>{coverFirst&&!legacyAndroid&&<IconButton label={androidLyricsOpen?"返回封面":"显示全屏歌词"} aria-pressed={androidLyricsOpen} onClick={()=>{if(!androidLyricsOpen)setRetainAndroidLyrics(true);setAndroidLyricsOpen(!androidLyricsOpen);}}><Languages size={20}/></IconButton>}<IconButton label="播放队列" onClick={()=>showQueue?.()}><ListMusic size={21}/></IconButton></div></div>
        </footer>
      </>:<div className="player-layout">
        {phone && zone?.track?.cover_url && !editing && (
          <Cover api={api} track={zone.track} className="player-atmosphere" />
        )}
        <div className="player-canvas" ref={canvas} style={{ height }}>
          {visibleModules.map((module) => (
              <Rnd
                key={module.id}
                bounds="parent"
                disableDragging={!editing}
                enableResizing={editing}
                dragHandleClassName="module-handle"
                minWidth={Math.min(160, width)}
                minHeight={75}
                size={{ width: (width * module.w) / 100, height: module.h }}
                position={{ x: (width * module.x) / 100, y: module.y }}
                onDragStop={(_, data) =>
                  change(module.id, {
                    x: Math.max(0, (data.x / width) * 100),
                    y: Math.max(0, data.y),
                  })
                }
                onResizeStop={(_, __, ref, ___, position) =>
                  change(module.id, {
                    w: Math.min(100, (ref.offsetWidth / width) * 100),
                    h: ref.offsetHeight,
                    x: Math.max(0, (position.x / width) * 100),
                    y: Math.max(0, position.y),
                  })
                }
                className={`playback-module module-${module.id}`}
              >
                <div className="module-content">{renderModule(module.id)}</div>
                {editing && (
                  <div className="module-handle">
                    <Grip size={15} />
                    {labels[module.id]}
                    <IconButton
                      label={`隐藏${labels[module.id]}`}
                      onClick={() => change(module.id, { visible: false })}
                    >
                      <X size={13} />
                    </IconButton>
                  </div>
                )}
                {editing && <Maximize2 className="resize-icon" size={12} />}
              </Rnd>
            ))}
        </div>
        {editing && (
          <aside className="layout-inspector">
            <h3>布局模块</h3>
            {modules.map((module) => (
              <label key={module.id}>
                <span>{labels[module.id]}</span>
                <input
                  className="toggle"
                  type="checkbox"
                  checked={module.visible}
                  disabled={isNative && module.id === "spectrum"}
                  onChange={(e) =>
                    change(module.id, { visible: e.target.checked })
                  }
                />
              </label>
            ))}
            <p className="fine-print">
              拖动模块标题调整位置，拖动边缘缩放。不同屏幕分别保存。
            </p>
          </aside>
        )}
      </div>}
      {options&&<Modal title="播放页设置" onClose={()=>setOptions(false)}><fieldset className="player-skin-picker"><legend>播放页皮肤</legend><div>{([["silk","封面氛围"],["midnight","午夜唱片"],["paper","纸上唱片"]] as const).map(([id,name])=><button key={id} aria-pressed={(preferences.player_skin||"silk")===id} onClick={()=>void savePreferences({player_skin:id}).catch(onError)}><i data-player-preview={id} aria-hidden="true"/><span>{name}</span></button>)}</div></fieldset><div className="player-option-actions"><button onClick={()=>{setOptions(false);setSignalDetails(true);}}><Link size={17}/>音频链路</button><button onClick={()=>{setOptions(false);setLyricSettings(true);}}><Type size={17}/>歌词样式</button>{!phone&&<button onClick={()=>{setOptions(false);if(!editing)setView("custom");setEditing(!editing);}}><LayoutDashboard size={17}/>{editing?"结束编辑":"自定义布局"}</button>}</div>{!!lyrics?.translations?.length&&<label>翻译版本<select aria-label="翻译版本" value={preferences.lyric_language||lyrics.translations[0].id} onChange={event=>void savePreferences({lyric_language:event.target.value}).catch(onError)}>{lyrics.translations.map(item=><option key={item.id} value={item.id}>{item.label}</option>)}</select></label>}<label className="setting-row"><span>封面动效<small>唱片缓慢旋转，方形封面轻微呼吸；暂停时停止</small></span><input className="toggle" type="checkbox" checked={preferences.player_rotate!==false} onChange={event=>void savePreferences({player_rotate:event.target.checked}).catch(onError)}/></label><label className="setting-row"><span>封面氛围<small>关闭后使用当前主题的纯色背景</small></span><input className="toggle" type="checkbox" checked={preferences.player_atmosphere!==false} onChange={event=>void savePreferences({player_atmosphere:event.target.checked}).catch(onError)}/></label>{lyrics?.romanization&&<label className="setting-row"><span>显示发音</span><input className="toggle" type="checkbox" checked={preferences.lyric_romanization!==false} onChange={event=>void savePreferences({lyric_romanization:event.target.checked}).catch(onError)}/></label>}</Modal>}
      {signalDetails&&<Modal title="播放详情" onClose={()=>setSignalDetails(false)}><SignalPanel inline zone={zone} info={sourceInfo} error={sourceError} adaptive={adaptive} nativeState={nativeState} localOutput={localOutput} contextRate={contextRate} endpoint={endpoint} onDsp={onDsp?()=>{setSignalDetails(false);onDsp();}:undefined}/></Modal>}
      {lyricSettings && (
        <Modal title="歌词样式" onClose={() => setLyricSettings(false)}>
          <label>
            字号 {preferences.lyric_font || 30}px
            <input
              type="range"
              min={18}
              max={54}
              value={preferences.lyric_font || 30}
              onChange={(e) =>
                savePreferences({ lyric_font: Number(e.target.value) })
              }
            />
          </label>
          <label>
            时间偏移 (ms)
            <input
              type="number"
              step={100}
              min={-30000}
              max={30000}
              value={preferences.lyric_offset || 0}
              onChange={(e) =>
                savePreferences({ lyric_offset: Number(e.target.value) })
              }
            />
          </label>
        </Modal>
      )}
    </div>
  );
}
function NativeMeter({ state }: { state: NativePlayback }) {
  const peak = state.meter?.peak || 0;
  return (
    <div className="native-meter">
      <h3>峰值电平</h3>
      <div>
        <i style={{ width: `${Math.min(peak, 1) * 100}%` }} />
      </div>
      <span>
        {peak > 0 ? (20 * Math.log10(peak)).toFixed(1) : "-∞"} dBFS · 削波样本{" "}
        {state.meter?.clipped_samples || 0}
      </span>
    </div>
  );
}
function Visualizer({
  analyser,
  spectrum,
  low,
}: {
  analyser: RefObject<AnalyserNode | null>;
  spectrum: boolean;
  low: boolean;
}) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [available, setAvailable] = useState(false);
  useEffect(() => {
    let frame = 0;
    let previous = 0;
    const draw = (time: number) => {
      frame = requestAnimationFrame(draw);
      if (time - previous < (low ? 100 : 33)) return;
      previous = time;
      const node = analyser.current;
      const element = canvas.current;
      if (!node || !element) {
        setAvailable(false);
        return;
      }
      setAvailable(true);
      const context = element.getContext("2d");
      if (!context) return;
      const values = new Uint8Array(node.frequencyBinCount);
      if (spectrum) node.getByteFrequencyData(values);
      else node.getByteTimeDomainData(values);
      const width = (element.width = element.clientWidth * devicePixelRatio);
      const height = (element.height = element.clientHeight * devicePixelRatio);
      context.clearRect(0, 0, width, height);
      context.fillStyle =
        getComputedStyle(element).getPropertyValue("--accent");
      if (spectrum) {
        const count = 40;
        const step = Math.max(1, Math.floor(values.length / count));
        for (let i = 0; i < count; i++) {
          const h = (values[i * step] / 255) * height;
          context.fillRect(
            (i * width) / count,
            height - h,
            width / count - 3,
            h,
          );
        }
      } else {
        const peak = Math.max(...values.map((v) => Math.abs(v - 128))) / 128;
        context.fillRect(0, height * 0.3, width * peak, height * 0.4);
        context.fillStyle =
          getComputedStyle(element).getPropertyValue("--muted");
        context.font = `${12 * devicePixelRatio}px sans-serif`;
        context.fillText(
          `${peak > 0 ? (20 * Math.log10(peak)).toFixed(1) : "-∞"} dBFS`,
          0,
          height * 0.95,
        );
      }
    };
    frame = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(frame);
  }, [analyser, spectrum, low]);
  return (
    <div className="visualizer">
      <h3>{spectrum ? "频谱" : "峰值电平"}</h3>
      <canvas ref={canvas} />
      {!available && <p>启用浏览器 DSP 后可显示实时音频数据。</p>}
    </div>
  );
}
