import {useTrackSort} from "./TrackSort";
import {useTrackContextMenu} from "./TrackContextMenu";
import {CatalogLink} from "./CatalogLinks";
import {useResourceRefresh} from "./resourceCache";
import {playlistPage,rememberPlaylistPage} from "./playlistPageCache";
import {SongButtons} from "./SongCollection";
import { useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  Heart,
  Import,
  ListMusic,
  ListPlus,
  LoaderCircle,
  Play,
  X,
} from "lucide-react";
import type {
  ProviderManifest,
  ProviderPlaylist,
  ProviderPlaylistTracks,
  RMusicClient,
  Track,
  Zone,
} from "@r-music/sdk";
import { Cover, Empty, IconButton, Modal, quality } from "./components";
import { collectPlaylistTracks, PLAYLIST_TRACK_LIMIT } from "./playlistPaging";
import { formatTime } from "./lyrics";

type Action = "play" | "append" | "import";
function PlaylistCover({api,item}:{api:RMusicClient;item:ProviderPlaylist}){
 return item.kind==="liked"&&!item.cover_url?<div className="cover liked-cover" role="img" aria-label="我喜欢"><Heart/></div>:<Cover api={api} track={{cover_url:item.cover_url,title:item.name,album:item.name}}/>;
}

export function ProviderPlaylistsView({
  api,
  provider,
  zone,
  play,
  enqueue,
  refreshZones,
  onError,
  notify,
  goSettings,
  onImported,
  initialPlaylist,onBack,
}: {
  api: RMusicClient;
  provider: ProviderManifest;
  zone?: Zone;
  play: (track: Track, list?: Track[]) => Promise<void>;
  enqueue: (track: Track) => Promise<void>;
  refreshZones: () => Promise<unknown>;
  onError: (error: unknown) => void;
  notify: (message: string) => void;
  goSettings: () => void;
  onImported: () => void;
  initialPlaylist?:ProviderPlaylist;onBack?:()=>void;
}) {
  const contextMenu=useTrackContextMenu();
  const [collection, setCollection] = useState<"all" | "created" | "collected">(
    "all",
  );
  const [items, setItems] = useState<ProviderPlaylist[]>([]);
  const [nextPage, setNextPage] = useState<number | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [selected, setSelected] = useState<ProviderPlaylist | null>(initialPlaylist||null);
  const [trackPage, setTrackPage] = useState(1);
  const [pageHistory, setPageHistory] = useState<number[]>([]);
  const [tracks, setTracks] = useState<ProviderPlaylistTracks | null>(null);
  const sorted=useTrackSort(tracks?.items||[],"本页排序");
  const [trackLoading, setTrackLoading] = useState(false);
  const [trackError, setTrackError] = useState("");
  const [trackRevision,setTrackRevision]=useState(0);const pulse=useResourceRefresh();
  useEffect(()=>{const changed=(event:Event)=>{if((event as CustomEvent).detail===api)setTrackRevision(value=>value+1);};window.addEventListener("rmusic-playlists-changed",changed);return()=>window.removeEventListener("rmusic-playlists-changed",changed);},[api]);
  const [progress, setProgress] = useState<{
    count: number;
    total: number;
  } | null>(null);
  const [bulkReady, setBulkReady] = useState<{
    action: Action;
    tracks: Track[];
    total: number;
    name: string;
    truncated: boolean;
  } | null>(null);
  const controller = useRef<AbortController | null>(null);
  const [applying, setApplying] = useState(false);
  const kindLabel = {
    created: "我创建的",
    collected: "收藏歌单",
    liked: "我喜欢",
  };
  useEffect(() => {
    let active = true;
    setLoading(true);
    setError("");
    setItems([]);
    setNextPage(null);
    api
      .providerPlaylists(provider.id, collection)
      .then((result) => {
        if (active) {
          setItems(result.items);
          setNextPage(result.next_page);
        }
      })
      .catch((cause) => {
        if (active) setError(cause.message);
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, [api, provider.id, collection]);
  useEffect(() => {
    if (!selected||document.hidden) return;
    let active = true;
    const cacheKey=JSON.stringify([provider.id,selected.account_context,selected.id,trackPage]);
    const cached=playlistPage<ProviderPlaylistTracks>(api,cacheKey);
    setTrackLoading(!cached);
    setTrackError("");
    setTracks(cached?.value||null);
    if(cached?.fresh)return;
    api
      .providerPlaylistTracks(provider.id, selected.id, trackPage)
      .then((result) => {
        if (active){rememberPlaylistPage(api,cacheKey,result);setTracks(result);}
      })
      .catch((cause) => {
        if (active) setTrackError(cause.message);
      })
      .finally(() => {
        if (active) setTrackLoading(false);
      });
    return () => {
      active = false;
    };
  }, [api, provider.id, selected?.id, trackPage, trackRevision,pulse]);
  useEffect(() => () => controller.current?.abort(), [api, provider.id]);
  async function apply(action: Action, list: Track[], name: string) {
    if(action!=="import")list=list.filter(track=>track.available);
    if (!list.length) {
      notify("这份歌单没有可操作的歌曲");
      return;
    }
    if (action !== "import" && !zone) {
      notify("请先选择一个音区");
      return;
    }
    setApplying(true);
    try {
      if (action === "import") {
        await api.post("/playlists", {
          name: `${name} · ${provider.name}`.slice(0, 100),
          track_ids: list.map((item) => item.id),
        });
        notify(`已导入 ${list.length} 首至本地歌单`);
        onImported();
      } else if (action === "play") {
        await play(list[0], list);
      } else {
        await api.post(`/zones/${zone!.id}/queue`, {
          track_ids: list.map((item) => item.id),
        });
        await refreshZones();
        notify(`已追加 ${list.length} 首至队列`);
      }
      setBulkReady(null);
    } catch (cause) {
      onError(cause);
    } finally {
      setApplying(false);
    }
  }
  async function full(action: Action) {
    if (!selected||document.hidden) return;
    const abort = new AbortController();
    controller.current = abort;
    setProgress({ count: 0, total: tracks?.total || selected.track_count });
    try {
      const result = await collectPlaylistTracks(
        (page, signal) =>
          api.providerPlaylistTracks(provider.id, selected.id, page, signal),
        abort.signal,
        (count, total) => setProgress({ count, total }),
      );
      abort.signal.throwIfAborted();
      controller.current = null;
      setProgress(null);
      if (result.truncated || result.total > result.tracks.length)
        setBulkReady({ ...result, action, name: selected.name });
      else await apply(action, result.tracks, selected.name);
    } catch (cause) {
      if (abort.signal.aborted) notify("已取消读取，歌单和队列未更改");
      else onError(cause);
    } finally {
      controller.current = null;
      setProgress(null);
    }
  }
  const blocked = !!progress || applying;
  const loginNotice = (message: string) => (
    <div className="provider-notice" role="alert">
      <strong>无法读取{provider.name}歌单</strong>
      <p>{message}</p>
      <button onClick={goSettings}>前往音源设置</button>
    </div>
  );
  return (
    <section className="provider-playlists-view">
      {selected ? (
        <>
          <button
            className="text-button provider-back"
            disabled={blocked}
            onClick={() => onBack?onBack():setSelected(null)}
          >
            <ArrowLeft size={17} />
            返回歌单
          </button>
          <div className="provider-playlist-heading">
            <PlaylistCover api={api} item={selected}/>
            <div className="provider-playlist-summary">
              <span className="playlist-byline">{provider.name} · {kindLabel[selected.kind]}</span>
              <h2>{selected.name}</h2>
              <p className="muted">{tracks?.total??selected.track_count} 首</p>
              <div className="list-actions provider-bulk-actions">
                <button className="primary" disabled={blocked||!zone||!tracks?.items.some(track=>track.available)} onClick={()=>full("play")}><Play size={17} fill="currentColor"/>播放</button>
                <button disabled={blocked||!zone||!tracks?.items.length} onClick={()=>full("append")}><ListPlus size={17}/>添加到队列</button>
                <button disabled={blocked||!tracks?.items.length} onClick={()=>full("import")}><Import size={17}/>复制本地副本</button>
              </div>
            </div>
          </div>
          {progress && (
            <div className="provider-progress" role="status">
              <LoaderCircle className="spin" size={17} />
              <span>
                已读取 {progress.count.toLocaleString()} 首 · 平台报告{" "}
                {progress.total.toLocaleString()} 首
              </span>
              <button onClick={() => controller.current?.abort()}>
                <X size={16} />
                取消
              </button>
            </div>
          )}
          {trackError ? (
            loginNotice(trackError)
          ) : trackLoading ? (
            <div className="loading">
              <LoaderCircle className="spin" />
              正在读取歌单…
            </div>
          ) : (
            tracks && (
              <>
                <div className="section-heading provider-page-actions">
                  <p className="muted">第 {trackPage} 页</p>{sorted.control}
                  <div className="list-actions">
                    <button
                      disabled={blocked || !zone || !tracks.items.length}
                      onClick={() => full("play")}
                    >
                      播放全部
                    </button>
                    <button
                      disabled={blocked || !zone || !tracks.items.length}
                      onClick={() =>
                        apply("append", sorted.tracks, selected.name)
                      }
                    >
                      追加当前页
                    </button>
                  </div>
                </div>
                <div className="provider-table-header"><span>#</span><span>歌曲</span><span>专辑</span><span>音质</span><span>时长</span><span/></div>
                <div className="provider-track-list">
                  {sorted.tracks.map((track, index) => (
                    <div
                      className="provider-track"
                      key={`${track.id}-${index}`} onContextMenu={e=>contextMenu?.(e,track)}
                    >
                      <span className="track-number">{(trackPage-1)*100+index+1}</span>
                      <button
                        className="provider-track-play"
                        disabled={blocked || !track.available || !zone}
                        onClick={() => play(track,sorted.tracks)}
                      >
                        <Cover api={api} track={track} />
                        <span>
                          <strong>{track.title}</strong>
                          <small>
                            <CatalogLink track={track}/>
                          </small>
                        </span>
                      </button>
                      <span className="provider-track-album"><CatalogLink track={track} album/></span>
                      <span className="provider-track-quality">
                        {quality(track)}
                      </span>
                      <time>{formatTime(track.duration)}</time>
                      <div className="row-actions"><SongButtons track={track}/><IconButton
                        label={`追加 ${track.title}`}
                        disabled={blocked || !zone || !track.available}
                        onClick={() => enqueue(track)}
                      >
                        <ListPlus size={17} />
                      </IconButton></div>
                    </div>
                  ))}
                </div>
                {!tracks.items.length && (
                  <Empty
                    icon={<ListMusic />}
                    headline="这份歌单还没有歌曲"
                    description="平台本页未返回曲目。"
                  />
                )}
                <div className="pagination-controls">
                  <button
                    disabled={blocked || !pageHistory.length}
                    onClick={() => {
                      setTrackPage(pageHistory.at(-1)!);
                      setPageHistory((old) => old.slice(0, -1));
                    }}
                  >
                    上一页
                  </button>
                  <span>第 {trackPage} 页</span>
                  <button
                    disabled={blocked || tracks.next_page === null}
                    onClick={() => {
                      setPageHistory((old) => [...old, trackPage]);
                      setTrackPage(tracks.next_page!);
                    }}
                  >
                    下一页
                  </button>
                </div>
              </>
            )
          )}
        </>
      ) : (
        <>
          <div className="source-tabs provider-collection-tabs">
            {(
              [
                ["all", "全部歌单"],
                ["created", "我创建的"],
                ["collected", "收藏歌单"],
              ] as const
            ).map(([id, label]) => (
              <button
                key={id}
                disabled={loading}
                className={collection === id ? "selected" : ""}
                onClick={() => setCollection(id)}
              >
                {label}
              </button>
            ))}
          </div>
          {error ? (
            loginNotice(error)
          ) : loading && !items.length ? (
            <div className="loading">
              <LoaderCircle className="spin" />
              正在读取我的歌单…
            </div>
          ) : (
            <>
              <div className="provider-playlist-grid">
                {items.map((item) => (
                  <button
                    className="provider-playlist-card"
                    key={`${item.kind}-${item.id}`}
                    onClick={() => {
                      setSelected(item);
                      setTrackPage(1);
                      setPageHistory([]);
                    }}
                  >
                    <PlaylistCover api={api} item={item}/>
                    <span className="provider-playlist-kind">
                      {item.kind === "liked" && (
                        <Heart size={12} fill="currentColor" />
                      )}
                      {kindLabel[item.kind]}
                    </span>
                    <strong>{item.name}</strong>
                    <small>{item.track_count} 首</small>
                  </button>
                ))}
              </div>
              {!items.length && (
                <Empty
                  icon={<ListMusic />}
                  headline="这里还没有歌单"
                  description="当前平台分类没有返回歌单。"
                />
              )}
              {nextPage !== null && (
                <button
                  className="load-more"
                  disabled={loading}
                  onClick={async () => {
                    setLoading(true);
                    try {
                      const result = await api.providerPlaylists(
                        provider.id,
                        collection,
                        nextPage,
                      );
                      setItems((old) => [...old, ...result.items]);
                      setNextPage(result.next_page);
                    } catch (cause) {
                      setError(
                        cause instanceof Error ? cause.message : String(cause),
                      );
                    } finally {
                      setLoading(false);
                    }
                  }}
                >
                  {loading ? "正在加载…" : "加载更多歌单"}
                </button>
              )}
            </>
          )}
        </>
      )}
      {bulkReady && (
        <Modal
          title={
            bulkReady.truncated ? "歌单超过操作上限" : "确认平台返回的曲目"
          }
          onClose={() => !applying && setBulkReady(null)}
        >
          <p>
            已读取 {bulkReady.tracks.length.toLocaleString()} 首；平台报告{" "}
            {bulkReady.total.toLocaleString()} 首。
            {bulkReady.truncated
              ? `为控制队列与内存，本次最多操作前 ${PLAYLIST_TRACK_LIMIT.toLocaleString()} 首。`
              : "平台分页已结束，未返回的曲目无法纳入本次操作。"}
          </p>
          <div className="list-actions">
            <button onClick={() => setBulkReady(null)} disabled={applying}>
              取消
            </button>
            <button
              className="primary"
              disabled={applying}
              onClick={() =>
                apply(bulkReady.action, bulkReady.tracks, bulkReady.name)
              }
            >
              {bulkReady.action === "play"
                ? "播放"
                : bulkReady.action === "append"
                  ? "追加"
                  : "导入"}
              {bulkReady.truncated ? "前" : "已读取的"}{" "}
              {bulkReady.tracks.length.toLocaleString()} 首
            </button>
          </div>
        </Modal>
      )}
    </section>
  );
}
