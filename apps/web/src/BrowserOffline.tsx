import {useEffect,useRef,useState} from "react";
import {Download,Pause,Play,RefreshCw,Square,Volume2,VolumeX} from "lucide-react";
import {browserCache,formatBytes,type BrowserCacheEntry} from "./browserCache";
import {Empty,IconButton} from "./components";
import {formatTime} from "./lyrics";
import {ListenSampler,listenJournal,type ListenOwner,type ListenSnapshot} from "./offlineStats";

export function BrowserOffline({onError}:{onError:(error:unknown)=>void}){
  const [entries,setEntries]=useState<BrowserCacheEntry[]>([]);
  const [current,setCurrent]=useState<BrowserCacheEntry>();
  const [paused,setPaused]=useState(true),[position,setPosition]=useState(0),[duration,setDuration]=useState(0),[volume,setVolume]=useState(1),[muted,setMuted]=useState(false);
  const [sourceReady,setSourceReady]=useState(false);
  const audio=useRef<HTMLAudioElement>(null),url=useRef(""),generation=useRef(0);
  const listening=useRef<{sampler:ListenSampler;owner:ListenOwner;entry:BrowserCacheEntry}|undefined>(undefined);
  function save(snapshot:ListenSnapshot|undefined){const listen=listening.current;if(snapshot&&listen)void listenJournal.save(listen.owner,snapshot).catch(onError);}
  function sample(reset=false){const player=audio.current,listen=listening.current;if(!player||!listen)return;listen.sampler.sample(player.currentTime,!player.paused&&!player.ended,reset);save(listen.sampler.flush());}
  function finish(reason:ListenSnapshot["reason"]){const listen=listening.current;if(!listen)return;sample();save(listen.sampler.finish(reason));listening.current=undefined;}
  function begin(entry:BrowserCacheEntry){if(!entry.core_id||!entry.user_id)return;const owner={core_id:entry.core_id,user_id:entry.user_id};listening.current={sampler:new ListenSampler(entry,Number.isFinite(audio.current?.duration)?audio.current!.duration:0),owner,entry};sample(true);}
  const refresh=()=>browserCache.list().then(items=>setEntries(items.filter(item=>item.complete))).catch(onError);
  function halt(){
    finish("stop");
    ++generation.current;audio.current?.pause();audio.current?.removeAttribute("src");audio.current?.load();
    if(url.current)URL.revokeObjectURL(url.current);url.current="";setCurrent(undefined);setSourceReady(false);setPaused(true);setPosition(0);setDuration(0);browserCache.protect(undefined,"offline");
  }
  useEffect(()=>{
    let active=true;
    const read=()=>browserCache.list().then(items=>{if(active)setEntries(items.filter(item=>item.complete));}).catch(error=>{if(active)onError(error);});
    read();const changed=()=>{halt();void read();};const timer=setInterval(read,5000);
    const interrupted=()=>finish("interrupted"),hidden=()=>{sample();save(listening.current?.sampler.flush(true));};
    window.addEventListener("rmusic-cache-owner-changed",changed);window.addEventListener("rmusic-online",halt);window.addEventListener("rmusic-offline-stop",halt);window.addEventListener("rmusic-listen-owner-changed",interrupted);window.addEventListener("pagehide",interrupted);document.addEventListener("visibilitychange",hidden);
    return()=>{finish("interrupted");active=false;clearInterval(timer);window.removeEventListener("rmusic-cache-owner-changed",changed);window.removeEventListener("rmusic-online",halt);window.removeEventListener("rmusic-offline-stop",halt);window.removeEventListener("rmusic-listen-owner-changed",interrupted);window.removeEventListener("pagehide",interrupted);document.removeEventListener("visibilitychange",hidden);++generation.current;audio.current?.pause();if(url.current)URL.revokeObjectURL(url.current);browserCache.protect(undefined,"offline");};
  },[onError]);
  useEffect(()=>{
    if(!current)return;
    const protect=()=>browserCache.protect(current.track_id,"offline");protect();const timer=setInterval(protect,10000);
    return()=>{clearInterval(timer);browserCache.protect(undefined,"offline");};
  },[current]);
  async function start(entry:BrowserCacheEntry){
    finish("replaced");
    const attempt=++generation.current;
    window.dispatchEvent(new Event("rmusic-offline"));audio.current?.pause();audio.current?.removeAttribute("src");audio.current?.load();if(url.current)URL.revokeObjectURL(url.current);url.current="";setSourceReady(false);setCurrent(entry);setPaused(true);setPosition(0);setDuration(0);
    browserCache.protect(entry.track_id,"offline");
    try{
      const blob=await browserCache.blob(entry.id);if(attempt!==generation.current)return;
      const next=URL.createObjectURL(blob);if(url.current)URL.revokeObjectURL(url.current);url.current=next;
      const player=audio.current;if(!player)return;player.src=next;player.volume=volume;player.muted=muted;setSourceReady(true);
      await player.play();
    }catch(error){if(attempt===generation.current)onError(error);}
  }
  function stop(){halt();window.dispatchEvent(new Event("rmusic-offline-stop"));}
  return <section className="settings-section">
    <div className="section-heading"><h2>此设备离线音乐</h2><IconButton label="刷新离线音乐" onClick={refresh}><RefreshCw size={17}/></IconButton></div>
    {!window.isSecureContext&&<p className="muted">网页离线缓存需要通过 HTTPS 访问。</p>}
    {entries.length?<div className="download-list offline-list">{entries.map(entry=><div key={entry.id}><Download size={19}/><div><strong>{entry.title||entry.track_id}</strong><span>{entry.artist?`${entry.artist} · `:""}{formatBytes(entry.size)}</span></div><button onClick={()=>start(entry)}><Play size={16}/>播放</button></div>)}</div>:<Empty icon={<Download/>} headline="还没有完整的离线音乐" description="在歌曲菜单选择下载到此设备，完整原文件会保存在此浏览器中。"/>}
    <audio ref={audio} preload="metadata" onLoadedMetadata={()=>{setDuration(Number.isFinite(audio.current?.duration)?audio.current!.duration:0);if(current&&!listening.current)begin(current);}} onTimeUpdate={()=>{setPosition(audio.current?.currentTime||0);sample();}} onPlay={()=>{if(current&&!listening.current&&audio.current!.readyState>=1)begin(current);sample();setPaused(false);}} onPause={()=>{sample();save(listening.current?.sampler.flush(true));setPaused(true);}} onSeeking={()=>sample(true)} onSeeked={()=>sample(true)} onEnded={()=>{if(audio.current?.ended){finish("ended");setPaused(true);}}} onError={()=>{if(audio.current?.error&&url.current){finish("interrupted");onError(new Error("此浏览器无法解码缓存文件"));}}}/>
    {current&&<div className="offline-player"><strong>{current.title||current.track_id}</strong><div className="offline-controls">
      <IconButton disabled={!sourceReady} label={paused?"继续离线播放":"暂停离线播放"} onClick={()=>{const player=audio.current;if(!player)return;if(paused){if(player.ended){finish("replay");player.currentTime=0;}player.play().catch(onError);}else player.pause();}}>{paused?<Play/>:<Pause/>}</IconButton>
      <IconButton label="停止离线播放" onClick={stop}><Square size={17}/></IconButton>
      <time>{formatTime(position)} / {formatTime(duration)}</time><input aria-label="离线播放进度" type="range" min={0} max={duration||0} step={0.1} value={Math.min(position,duration||0)} disabled={!duration} onChange={event=>{sample();if(audio.current)audio.current.currentTime=Number(event.target.value);sample(true);setPosition(Number(event.target.value));}}/>
      <IconButton label={muted?"取消离线静音":"离线静音"} onClick={()=>{if(audio.current)audio.current.muted=!muted;setMuted(!muted);}}>{muted?<VolumeX/>:<Volume2/>}</IconButton>
      <input aria-label="离线音量" type="range" min={0} max={1} step={0.01} value={volume} onChange={event=>{const value=Number(event.target.value);if(audio.current)audio.current.volume=value;setVolume(value);}}/>
    </div></div>}
  </section>;
}
