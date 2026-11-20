import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import type { RMusicClient, Track } from "@r-music/sdk";
import { Cover, IconButton } from "./components";
import { ChevronLeft, ChevronRight, Play } from "lucide-react";
import { useTrackContextMenu } from "./trackMenuContext";
import "./PersonalRecommendations.css";

export type Picks = { date: string | null; items: Track[]; reasons: Record<string, string>; scores?:Record<string,number> };
type Play = (track: Track, tracks: Track[]) => Promise<void>;
type Seed = Pick<Track, "id" | "title">;
type SimilarActions = {
  find: (track: Seed, automatic?: boolean) => void;
  unliked: (track: Track) => void;
};
const Similar = createContext<SimilarActions | null>(null);
export const useSimilarSongs = () => useContext(Similar);
type Notice = {
  seed: Seed;
  automatic: boolean;
  items: Track[];
  loading: boolean;
  expanded: boolean;
  error?: string;
};

export function PersonalRecommendations({
  api,
  play,
  canPlay,
  onError,
  children,
}: {
  api: RMusicClient;
  play: Play;
  canPlay: boolean;
  onError: (error: unknown) => void;
  children: ReactNode;
}) {
  const [notice, setNotice] = useState<Notice | null>(null);
  const active = useRef(false);
  const pending = useRef<{ id: string; automatic: boolean; abort: AbortController } | undefined>(undefined);
  function close() {
    pending.current?.abort.abort();
    pending.current = undefined;
    setNotice(null);
  }
  useEffect(() => {
    active.current = true;
    setNotice(null);
    return () => {
      active.current = false;
      pending.current?.abort.abort();
      pending.current = undefined;
    };
  }, [api]);
  function find(seed: Seed, automatic = false) {
    if (!active.current || (automatic && pending.current && !pending.current.automatic)) return;
    pending.current?.abort.abort();
    const abort = new AbortController();
    pending.current = { id: seed.id, automatic, abort };
    setNotice(automatic ? null : { seed, automatic, items: [], loading: true, expanded: true });
    void api
      .request<Picks>("/recommendations/similar", {
        method: "POST",
        body: JSON.stringify({ track_id: seed.id }),
        signal: abort.signal,
      })
      .then((result) => {
        if (abort.signal.aborted) return;
        if (automatic && !result.items.length) {
          close();
          return;
        }
        setNotice({ seed, automatic, items: result.items, loading: false, expanded: !automatic });
      })
      .catch(() => {
        if (abort.signal.aborted) return;
        if (automatic) {
          close();
          return;
        }
        setNotice({
          seed,
          automatic,
          items: [],
          loading: false,
          expanded: true,
          error: "相似歌曲暂时无法读取",
        });
      });
  }
  useEffect(() => {
    if (!notice?.automatic || notice.expanded) return;
    const timer = window.setTimeout(close, 12000);
    return () => clearTimeout(timer);
  }, [notice]);
  function unliked(track: Track) {
    if (pending.current?.automatic && pending.current.id === track.id) close();
  }
  return (
    <Similar.Provider value={{ find, unliked }}>
      {children}
      {notice && (
        <aside className="personal-similar" aria-label="相似歌曲">
          <div className="section-heading">
            <strong>《{notice.seed.title}》的相似歌曲</strong>
            <button type="button" onClick={close} aria-label="关闭相似歌曲">
              ×
            </button>
          </div>
          <div role="status">
            {notice.loading ? "正在查找…" : (notice.error ?? (!notice.items.length ? "暂无匹配曲目" : ""))}
          </div>
          {notice.items.slice(0, notice.expanded ? 8 : 1).map((t) => (
            <button
              key={t.id}
              className="personal-song"
              disabled={!canPlay}
              onClick={() => {
                void play(t, notice.items).catch(onError);
                close();
              }}
            >
              <span>{t.title}</span>
              <small>{t.artist}</small>
            </button>
          ))}
          {!notice.expanded && notice.items.length > 1 && (
            <button className="text-button" onClick={() => setNotice({ ...notice, expanded: true })}>
              查看全部 {notice.items.length} 首
            </button>
          )}
        </aside>
      )}
    </Similar.Provider>
  );
}

export function DailyPlaylist({
  api,
  play,
  canPlay,
  onError,
  onOpen,
}: {
  api: RMusicClient;
  play: Play;
  canPlay: boolean;
  onError: (error: unknown) => void;
  onOpen: (picks: Picks) => void;
}) {
  const [result, setResult] = useState<Picks | null>(null),
    [error, setError] = useState(false);
  const [revision, retry] = useState(0);
  useEffect(() => {
    const abort = new AbortController();
    let reading = false;
    async function load() {
      if (reading || document.visibilityState === "hidden") return;
      reading = true;
      try {
        const picks = await api.request<Picks>("/recommendations/daily", { signal: abort.signal });
        if (!abort.signal.aborted) {
          setResult(picks);
          setError(false);
        }
      } catch {
        if (!abort.signal.aborted) setError(true);
      } finally {
        reading = false;
      }
    }
    void load();
    const timer = window.setInterval(load, 30 * 60 * 1000);
    document.addEventListener("visibilitychange", load);
    return () => {
      abort.abort();
      clearInterval(timer);
      document.removeEventListener("visibilitychange", load);
    };
  }, [api, revision]);
  const rail=useRef<HTMLDivElement>(null);
  const menu=useTrackContextMenu();
  const tracks=result?.items??[];
  const start=(track:Track)=>void play(track,tracks).catch(onError);
  const scroll=(direction:number)=>{
    const list=rail.current;
    if(list)list.scrollBy({left:direction*list.clientWidth,behavior:matchMedia("(prefers-reduced-motion: reduce)").matches?"instant":"smooth"});
  };
  return <section className="personal-daily record-recent" aria-label="每日歌单">
    <header className="section-heading">
      <h2>每日歌单</h2>
      <div className="daily-shelf-actions">
        {tracks.length>0&&<button className="text-button" onClick={()=>result&&onOpen(result)}>查看全部 {tracks.length} 首</button>}
        {tracks.length>3&&<>
          <IconButton label="向左滚动每日歌单" onClick={()=>scroll(-1)}><ChevronLeft size={15}/></IconButton>
          <IconButton label="向右滚动每日歌单" onClick={()=>scroll(1)}><ChevronRight size={15}/></IconButton>
        </>}
        {error&&<button className="text-button" onClick={()=>retry(v=>v+1)}>重试</button>}
      </div>
    </header>
    {tracks.length>0?<div className="record-albums daily-albums" ref={rail}>
      {tracks.map(track=><button key={track.id} className="record-album" disabled={!canPlay}
        aria-label={`播放 ${track.title}`} onClick={()=>start(track)} onContextMenu={event=>menu?.(event,track)}>
        <Cover api={api} track={track}/><strong title={track.title}>{track.title}</strong><small title={track.artist}>{track.artist}</small>
      </button>)}
    </div>:<p className="muted" role="status">{error?"暂时无法读取":result?"暂无可用曲目":"正在选曲…"}</p>}
  </section>;
}
