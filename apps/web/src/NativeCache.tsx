import { useEffect, useRef, useState } from "react";
import {NativeStorageSettings} from "./NativeStorageSettings";
import {NativeDownloads} from "./NativeDownloads";
import {clientId} from "./clientId";
import { Download, Pause, Play, RefreshCw, Square, Trash2,Volume2,VolumeX,SkipBack,SkipForward } from "lucide-react";
import { invokeNative, nativeOutput, nativeProfiles, type NativeCacheEntry,type NativePlayback } from "./native";
import type {Zone} from "@r-music/sdk";
import {formatTime} from "./lyrics";
import { formatBytes } from "./browserCache";
import { Empty, IconButton } from "./components";
interface Status {
  limit_bytes: number;
  used_bytes: number;
  download_bytes: number;
  entries: NativeCacheEntry[];
}
export function NativeCacheSettings({
  api,
  notify,
  onError,
}: {
  api: {baseUrl:string;token:string};
  notify: (message: string) => void;
  onError: (error: unknown) => void;
}) {
  const [status, setStatus] = useState<Status | null>(null);
  const [limit, setLimit] = useState(5);
  const [policy, setPolicy] = useState<{
    unmetered_wifi: boolean;
    reason: string;
  } | null>(null);
  const [upgrade, setUpgrade] = useState<{
    enabled: boolean;
    active?: string;
    error?: string;
  } | null>(null);
  const refresh = () =>
    invokeNative<Status>("native_cache_status",{address:api.baseUrl})
      .then((value) => {
        setStatus(value);
        setLimit(value.limit_bytes / 1024 ** 3);
      })
      .catch(onError);
  useEffect(() => {
    refresh();
    invokeNative<{ unmetered_wifi: boolean; reason: string }>(
      "native_network_policy",
    )
      .then(setPolicy)
      .catch(onError);
    invokeNative<{ enabled: boolean; active?: string; error?: string }>(
      "native_upgrades_status",
    )
      .then(setUpgrade)
      .catch(onError);
  }, []);
  return (
    <section className="settings-section">
      <div className="section-heading">
        <h2>此客户端原生缓存</h2>
        <IconButton label="刷新本机缓存" onClick={refresh}>
          <RefreshCw size={17} />
        </IconButton>
      </div>
      <div className="storage-summary">
        <strong>{formatBytes(status?.used_bytes || 0)}</strong>
        <span>/ {formatBytes(status?.limit_bytes || 5 * 1024 ** 3)}</span>
      </div>
      <label className="inline-form">
        <span>容量上限 (GiB)</span>
        <input
          type="number"
          min={0.1}
          max={100000}
          step={0.5}
          value={limit}
          onChange={(e) => setLimit(Number(e.target.value))}
        />
        <button
          onClick={() =>
            invokeNative("native_cache_configure", {
              limitBytes: Math.round(limit * 1024 ** 3),
            })
              .then(() => {
                notify("客户端缓存容量已保存");
                refresh();
              })
              .catch(onError)
          }
        >
          保存
        </button>
      </label>
      <p className="fine-print">
        自动缓存只保存实际收到的原格式数据；部分缓存不参与离线播放。手动下载占用{" "}
        {formatBytes(status?.download_bytes || 0)}，独立于自动缓存容量。
      </p>
      {!/Android/i.test(navigator.userAgent)&&<label className="setting-row">
        <span>
          <strong>非计费 Wi-Fi 自动升级完整缓存</strong>
          <small>
            {policy?.reason || "正在确认网络计费状态"}
            。仅升级同一录音的完整缓存，验证成功后替换。
          </small>
        </span>
        <input
          className="toggle"
          type="checkbox"
          checked={upgrade?.enabled || false}
          disabled={!upgrade}
          onChange={(e) =>
            invokeNative("native_upgrades_configure", {
              enabled: e.target.checked,
            })
              .then(() =>
                invokeNative<{
                  enabled: boolean;
                  active?: string;
                  error?: string;
                }>("native_upgrades_status"),
              )
              .then(setUpgrade)
              .catch(onError)
          }
        />
      </label>}
      {!/Android/i.test(navigator.userAgent)&&upgrade?.error && (
        <p className="fine-print">最近升级：{upgrade.error}。旧文件保留。</p>
      )}
      {/Android/i.test(navigator.userAgent)&&<NativeStorageSettings notify={notify} onError={onError} changed={refresh}/>}
      <div className="cache-list">
        {status?.entries.map((entry) => (
          <div key={entry.id}>
            <span>{entry.title || entry.track_id}</span>
            <small>
              {entry.complete
                ? entry.verified
                  ? "完整 · 已校验"
                  : "完整 · 待校验"
                : "部分"}{" "}
              · {entry.format.toUpperCase()}
              {entry.manual_download ? " · 手动下载" : ""}
              {entry.channels>2?` · ${entry.channels}声道源，本机混音输出`:""}
              {entry.document_uri?" · 所选目录":""}
            </small>
            <IconButton
              label="移除本机文件"
              onClick={() =>
                invokeNative("native_cache_remove", { id: entry.id })
                  .then(refresh)
                  .catch(onError)
              }
            >
              <Trash2 size={16} />
            </IconButton>
          </div>
        ))}
      </div>
    </section>
  );
}
export function NativeOffline({
  api,
  onError,
  onRestore,
  active: visible = true,
}: {
  api: {baseUrl:string;token:string};
  onError: (error: unknown) => void;
  onRestore?:()=>void;
  active?:boolean;
}) {
  const [entries, setEntries] = useState<NativeCacheEntry[]>([]);
  useEffect(()=>{setEntries([]);setCurrent("");},[api.baseUrl,api.token]);
  const [current, setCurrent] = useState("");
  const [paused, setPaused] = useState(false);
  const [status,setStatus]=useState<NativePlayback>({});
  const playback=useRef("");
  const localQueue=useRef<{profileId:string;id:string}|undefined>(undefined);
  const controlRevision=useRef(0);
  const allowRestore=useRef(true);
  useEffect(() => {
    const online = () => {controlRevision.current++;allowRestore.current=false;playback.current="";localQueue.current=undefined;setCurrent("");setStatus(old=>({volume:old.volume,muted:old.muted}));};
    window.addEventListener("rmusic-online", online);
    window.addEventListener("rmusic-offline-stop",online);
    return () => {playback.current="";window.removeEventListener("rmusic-online", online);window.removeEventListener("rmusic-offline-stop",online);};
  }, []);
  const refresh = () =>
    invokeNative<Status>("native_cache_status",{address:api.baseUrl})
      .then((result) => setEntries(result.entries.filter((e) => e.complete)))
      .catch(onError);
  useEffect(() => {
    let active=true,polling=false;
    const read=()=>invokeNative<Status>("native_cache_status",{address:api.baseUrl}).then(result=>{if(active)setEntries(result.entries.filter(e=>e.complete));}).catch(error=>{if(active)onError(error);});
    const poll=async()=>{
      if(polling)return;polling=true;let expected=playback.current;const revision=controlRevision.current;
      try{
        const context=localQueue.current;
        const queue=context?await invokeNative<Zone>("native_request",{profileId:context.profileId,method:"GET",path:`/api/v1/zones/${context.id}/state`}):undefined;
        const next=await invokeNative<NativePlayback>("native_status");if(!active||playback.current!==expected||controlRevision.current!==revision||!next)return;
        if(queue){
          if(!queue.playback_id){setStatus(old=>({...old,status:"ended"}));setPaused(true);return;}
          if(queue.playback_id!==next.playback_id)return;
          next.offline=true;expected=next.playback_id;playback.current=expected;
        }
        if(!next.offline){if(!expected)setStatus(old=>({...old,volume:next.volume??old.volume,muted:next.muted??old.muted}));return;}
        if(!next.playback_id||!next.cache_id||!next.track_id)return;
        if(expected&&next.playback_id!==expected)return;
        if(!expected){if(!allowRestore.current)return;playback.current=next.playback_id;window.dispatchEvent(new CustomEvent("rmusic-offline",{detail:{playbackId:next.playback_id,trackId:next.track_id}}));onRestore?.();}
        setStatus(next);setCurrent(next.track_id);setPaused(!!next.paused);
      }catch(error){if(active&&playback.current===expected&&controlRevision.current===revision)onError(error);}finally{polling=false;}
    };
    const wake=()=>{if(visible&&!document.hidden){void read();void poll();}};
    read();poll();const timer=setInterval(()=>{if(visible&&!document.hidden)void read();},300000),clock=setInterval(()=>{if(visible&&!document.hidden)void poll();},1000);
    document.addEventListener("visibilitychange",wake);
    return()=>{active=false;document.removeEventListener("visibilitychange",wake);clearInterval(timer);clearInterval(clock);};
  }, [onError,onRestore,visible]);
  function queueCommand(action:string,body?:unknown,method="POST"){
    const context=localQueue.current;if(!context)return Promise.reject(new Error("请先选择一首离线音乐"));
    return invokeNative<Zone>("native_request",{profileId:context.profileId,method,path:`/api/v1/zones/${context.id}${action?`/${action}`:""}`,body});
  }
  async function startQueue(entry:NativeCacheEntry,position:number){
    const profileId=entry.profile_id||(await nativeProfiles()).find(p=>p.address.replace(/\/+$/,"")===api.baseUrl.replace(/\/+$/,""))?.id;
    if(!profileId)throw new Error("离线音乐的 Core 档案已移除");
    const endpointId=localStorage.getItem(`rmusic.profile-endpoint:${profileId}`)||localStorage.getItem("rmusic.endpoint-id")||`offline-${profileId}`;
    await invokeNative("native_endpoint_register",{requestId:clientId(),registrationId:clientId(),profileId,endpointId,name:localStorage.getItem("rmusic.device-name")||"本机 Android 客户端",output:nativeOutput()});
    localQueue.current={profileId,id:`client-zone-${endpointId}`};
    const rows=entries.filter(e=>!e.profile_id||e.profile_id===profileId);
    const tracks=rows.map(e=>({id:e.track_id,recording_id:e.recording_id,title:e.title||e.track_id,artist:e.artist||"",album:"",duration:0,format:e.format,sample_rate:e.sample_rate,bit_depth:e.bit_depth||0,channels:e.channels,available:true,source:"local"}));
    await queueCommand("queue",{track_ids:tracks.map(t=>t.id),tracks},"PUT");
    const zone=await queueCommand("play",{queue_index:Math.max(0,rows.findIndex(e=>e.id===entry.id))});
    playback.current=zone.playback_id||"";setCurrent(entry.track_id);
    if(position>0)await queueCommand("seek",{position});
  }
  function start(entry:NativeCacheEntry,position=0){
    const id=clientId(),revision=++controlRevision.current;playback.current=id;allowRestore.current=true;
    setCurrent(entry.title||entry.track_id);setStatus(old=>({volume:old.volume,muted:old.muted,offline:true,cache_id:entry.id,track_id:entry.track_id,playback_id:id,status:"preparing",position}));setPaused(false);
    window.dispatchEvent(new CustomEvent("rmusic-offline",{detail:{playbackId:id,trackId:entry.track_id}}));
    (/Android/i.test(navigator.userAgent)?startQueue(entry,position):invokeNative("native_offline_play",{playbackId:id,id:entry.id,position,sampleRate:entry.sample_rate,channels:entry.channels,output:nativeOutput()}))
      .catch(error=>{if(controlRevision.current===revision){setStatus(old=>({...old,status:"error"}));setPaused(true);onError(error);}});
  }
  const entry=entries.find(item=>item.id===status.cache_id);
  function navigate(action:string){
    controlRevision.current++;
    void queueCommand(action,{}).then(zone=>{playback.current=zone.playback_id||"";setCurrent(zone.track?.id||"");setStatus(old=>({...old,status:zone.playback_id?"preparing":"ended",playback_id:zone.playback_id||undefined,track_id:zone.track?.id,cache_id:entries.find(e=>e.track_id===zone.track?.id)?.id,position:zone.position}));setPaused(zone.status!=="playing");}).catch(onError);
  }
  function gain(volume?:number,muted?:boolean){
    const expected=playback.current,revision=++controlRevision.current;
    const args={...(volume===undefined?{}:{volume}),...(muted===undefined?{}:{muted})};
    setStatus(old=>({...old,...args}));(localQueue.current?queueCommand("",args,"PATCH"):invokeNative("native_volume",args)).catch(error=>{if(playback.current===expected&&controlRevision.current===revision)onError(error);});
  }
  return (
    <section className="settings-section">
      {/Android/i.test(navigator.userAgent)&&<><details className="remote-detail"><summary>离线存储位置</summary><NativeStorageSettings notify={()=>{}} onError={onError} changed={refresh}/></details>
      <NativeDownloads api={api} onError={onError}/></>}
      <div className="section-heading">
        <h2>本机离线音乐</h2>
        <IconButton label="刷新离线音乐" onClick={refresh}>
          <RefreshCw size={17} />
        </IconButton>
      </div>
      {entries.length ? (
        <div className="download-list offline-list">
          {entries.map((entry) => (
            <div key={entry.id}>
              <Download size={19} />
              <div>
                <strong>{entry.title || entry.track_id}</strong>
                <span>
                  {entry.artist ? `${entry.artist} · ` : ""}
                  {entry.format.toUpperCase()} · {entry.sample_rate / 1000} kHz
                  · {formatBytes(entry.total_bytes)}
                  {entry.manual_download ? " · 下载" : " · 完整缓存"}
                </span>
              </div>
              <button
                disabled={!entry.sample_rate || !entry.channels}
                onClick={() => start(entry)}
              >
                <Play size={16} />
                播放
              </button>
            </div>
          ))}
        </div>
      ) : (
        <Empty
          icon={<Download />}
          headline="还没有完整的离线音乐"
          description="播放完整歌曲或手动下载后，可以在不连接核心时聆听。"
        />
      )}
      {current && (
        <div className="offline-player">
          <strong>{entry?.title||current}</strong>
          <div className="offline-controls">
            {localQueue.current&&<IconButton label="离线上一首" onClick={()=>navigate("previous")}><SkipBack size={17}/></IconButton>}
            <button
              disabled={status.status==="error"&&!entry}
              onClick={() => {if(["ended","error"].includes(status.status||"")&&entry){start(entry,status.status==="error"?status.position||0:0);return;}const expected=playback.current,revision=++controlRevision.current;(localQueue.current?queueCommand(paused?"play":"pause",{}):invokeNative("native_pause", { paused: !paused })).then(()=>{if(playback.current===expected&&controlRevision.current===revision)setPaused(!paused);}).catch(error=>{if(playback.current===expected&&controlRevision.current===revision)onError(error);});}}
            >
              {paused ? <Play size={17} /> : <Pause size={17} />}{" "}
              {status.status==="error"?"重试播放":status.status==="ended"?"重新播放":paused ? "继续" : "暂停"}
            </button>
            {localQueue.current&&<IconButton label="离线下一首" onClick={()=>navigate("next")}><SkipForward size={17}/></IconButton>}
            <button
              onClick={() =>
                {controlRevision.current++;const expected=playback.current;return (localQueue.current?queueCommand("stop",{}):invokeNative("native_stop"))
                  .then(() => {if(playback.current===expected){setCurrent("");window.dispatchEvent(new Event("rmusic-offline-stop"));}})
                  .catch(error=>{if(playback.current===expected)onError(error);});}
              }
            >
              <Square size={15} />
              停止
            </button>
            <time>{formatTime(status.position||0)} / {formatTime(status.duration||0)}</time>
            <input aria-label="离线播放进度" type="range" min={0} max={status.duration||0} step={0.1} value={Math.min(status.position||0,status.duration||0)} disabled={!status.duration} onChange={event=>{const position=Number(event.target.value),id=localQueue.current?playback.current:clientId();controlRevision.current++;playback.current=id;setStatus(old=>({...old,playback_id:id,position}));window.dispatchEvent(new CustomEvent("rmusic-offline",{detail:{playbackId:id,trackId:status.track_id}}));(localQueue.current?queueCommand("seek",{position}):invokeNative("native_seek",{position,playbackId:id})).catch(error=>{if(playback.current===id)onError(error);});}}/>
            <IconButton label={status.muted?"取消离线静音":"离线静音"} onClick={()=>gain(undefined,!status.muted)}>{status.muted?<VolumeX/>:<Volume2/>}</IconButton>
            <input aria-label="离线音量" type="range" min={0} max={1} step={0.01} value={status.volume??1} onChange={event=>gain(Number(event.target.value))}/>
          </div>
        </div>
      )}
    </section>
  );
}
