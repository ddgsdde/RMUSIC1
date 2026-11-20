import {MiniLyricTitle,type MiniLyricsOptions} from "./PlayerLyrics";
import {PlaybackButton} from "./PlaybackButton";
import {VolumeControl} from "./VolumeControl";
import {artistNames,CatalogLink} from "./CatalogLinks";
import {SongButtons} from "./SongCollection";
import {
  Speaker,
  ChevronDown,
  ListMusic,
  Repeat,
  Repeat1,
  Shuffle,
  SkipBack,
  SkipForward,
  Info,
} from "lucide-react";
import type { RMusicClient, Zone, Endpoint } from "@r-music/sdk";
import { Cover, IconButton } from "./components";
import { formatTime } from "./lyrics";
import {SignalLight} from "./SignalLight";
import type {SignalGrade} from "./signalGrade";
export function Transport({
  api,
  inert=false,
  mobile=false,
  endpoint,
  outputLabel,
  zone,
  position,
  bufferedEnd,
  miniLyrics,
  playing,
  waiting=false,
  command,
  patchZone,
  seek,
  showPlayer,
  showQueue,
  busy,
  showZones,showSignal,resolution,showDetails,grade,
}: {
  api: RMusicClient;
  inert?:boolean;
  mobile?:boolean;
  endpoint?: Endpoint;
  outputLabel?:string;
  zone?: Zone;
  position: number;
  bufferedEnd?: number;
  miniLyrics?: MiniLyricsOptions;
  playing: boolean;
  waiting?:boolean;
  command: (action: string, body?: unknown) => Promise<void>;
  patchZone: (patch: Partial<Zone>) => Promise<void>;
  seek: (position: number) => void;
  showPlayer: () => void;
  showQueue: () => void;
  notify: (message: string) => void;
  onError: (error: unknown) => void;
  busy: boolean;
  showZones?:()=>void;showSignal:()=>void;resolution?:string;showDetails:()=>void;
  grade:SignalGrade;
}) {
  const track = zone?.track;
  return (
    <footer className="transport" data-empty={!track} inert={inert||undefined}>
      <div className="mobile-mini-progress" aria-hidden="true" style={{"--played":`${Math.max(0,Math.min(100,track?.duration?100*position/track.duration:0))}%`} as React.CSSProperties}>
        {bufferedEnd !== undefined && !!track?.duration && <span className="mobile-mini-buffered" style={{width:`${100 * bufferedEnd / track.duration}%`}}/>}
        <i/>
      </div>
      <div className="now-playing">
        <button className="now-playing-info" onClick={showPlayer}>
          <Cover api={api} track={track} />
          <div>
            {miniLyrics&&track?<MiniLyricTitle key={track.id} api={api} track={track}
              position={position} playbackId={zone?.playback_id??undefined} playing={playing&&!waiting&&!inert} options={miniLyrics}/>
              :<strong>{track?.title || "还没有正在播放的音乐"}</strong>}
            <span>{waiting?"等待设备播放…":track?(mobile?<>{miniLyrics&&<>{track.title} · </>}{artistNames(track).join(" · ")}</>:<CatalogLink track={track} separator=" · "/>):"从音乐库开始聆听"}</span>
          </div>
        </button>
        <SongButtons track={track} current/>
        <IconButton
          label="关于当前歌曲"
          disabled={!track}
          onClick={showDetails}
        >
          <Info size={19} />
        </IconButton>
      </div>
      <div className="transport-center">
        <div className="transport-controls">
          <IconButton
            label="随机播放"
            className={`icon-button ${zone?.shuffle ? "selected" : ""}`}
            disabled={!zone}
            onClick={() => patchZone({ shuffle: !zone?.shuffle })}
          >
            <Shuffle size={20} />
          </IconButton>
          <IconButton
            label="上一首"
            disabled={!track||busy}
            onClick={() => command("previous")}
          >
            <SkipBack size={22} fill="currentColor" />
          </IconButton>
          <PlaybackButton playing={playing} waiting={waiting} busy={busy} disabled={!zone||(!track&&!zone.queue.length)} command={command}/>
          <IconButton
            label="下一首"
            className="icon-button next-control"
            disabled={!track||busy}
            onClick={() => command("next")}
          >
            <SkipForward size={22} fill="currentColor" />
          </IconButton>
          <IconButton
            label={`循环：${zone?.repeat === "one" ? "单曲" : zone?.repeat === "all" ? "列表" : "关闭"}`}
            className={`icon-button ${zone?.repeat !== "off" ? "selected" : ""}`}
            disabled={!zone}
            onClick={() =>
              patchZone({
                repeat:
                  zone?.repeat === "off"
                    ? "all"
                    : zone?.repeat === "all"
                      ? "one"
                      : "off",
              })
            }
          >
            {zone?.repeat === "one" ? (
              <Repeat1 size={20} />
            ) : (
              <Repeat size={20} />
            )}
          </IconButton>
        </div>
        <div className="seek-row">
          <time>{formatTime(position)}</time>
          <input
            type="range"
            aria-label="播放进度"
            data-buffered={bufferedEnd!==undefined||undefined}
            min={0}
            max={track?.duration || 1}
            step={0.1}
            value={Math.min(position, track?.duration || 0)}
            onChange={(e) => seek(Number(e.target.value))}
            disabled={!track||busy}
            style={
              {
                "--progress": `${track?.duration ? Math.max(0,Math.min(100,100*position/track.duration)) : 0}%`,
                "--buffered": `${track?.duration&&bufferedEnd!==undefined ? Math.max(0,Math.min(100,100*bufferedEnd/track.duration)) : 0}%`,
              } as React.CSSProperties
            }
          />
          <time>{formatTime(track?.duration || 0)}</time>
        </div>
      </div>
      <div className="transport-output">
        <button className="transport-zone" aria-label="选择播放设备与编排分组" title={outputLabel||zone?.name||"选择播放设备"} onClick={showZones}><Speaker size={16}/><span>{outputLabel||zone?.name||"选择播放设备"}</span><ChevronDown size={14}/></button>
        <div className="volume-row">
          <VolumeControl scope={zone?.endpoint_id||""} value={endpoint?.output_config?.volume_control==="device"?(endpoint.device_volume?.value??1):endpoint?.output_config?.volume_control==="fixed"?1:zone?.volume??1} muted={!!zone?.muted} disabled={!zone||endpoint?.output_config?.volume_control==="fixed"} muteDisabled={endpoint?.output_config?.volume_control==="device"} label={endpoint?.output_config?.volume_control==="device"?"硬件设备音量":zone?.muted?"取消静音":"静音"} change={value=>patchZone({volume:value})} mute={()=>void patchZone({muted:!zone?.muted})}/>
          <button className="output-button" aria-label="音频链路" onClick={showSignal}>
            <SignalLight level={grade.level}/>
            <span>{grade.label}</span>
          </button>
          <IconButton label="播放队列" onClick={showQueue}>
            <ListMusic size={22} />
          </IconButton>
        </div>
        <button className="output-quality text-button" onClick={showSignal}>{resolution || "等待链路报告"}</button>
      </div>
    </footer>
  );
}
