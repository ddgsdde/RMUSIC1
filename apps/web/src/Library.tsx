import {LibraryFilterBar,filterQuery,NewMusic,releaseDate,useReleaseClock,type LibraryFilters} from "./CatalogControls";
import {useTrackContextMenu} from "./TrackContextMenu";
import {CatalogDetail} from "./CatalogDetail";
import {CatalogLink} from "./CatalogLinks";
import {RelatedCatalogButton} from "./RelatedCatalog";
import {CacheSongButton} from "./CacheSongButton";
import {SongButtons} from "./SongCollection";
import { useEffect, useRef, useState } from "react";
import {
  ChevronRight,
  ListPlus,
  Music2,
  Play,
  Search,
  Info,
  MoreHorizontal,
} from "lucide-react";
import type {
  Album,
  Page,
  ProviderManifest,
  RMusicClient,
  Track,
  Playlist,
} from "@r-music/sdk";
import { Cover, Empty, IconButton, quality } from "./components";
import { formatTime } from "./lyrics";
import {isCloudSearch,requestRevision} from "./libraryRequest";
import {Pagination,usePaging} from "./Pagination";
import {SaveSmartPlaylist} from "./SaveSmartPlaylist";
export function Library({
  api,
  query,
  source,
  albumId,
  artistId,
  revision,
  play,
  startRadio,
  enqueue,
  onError,
  goSettings,
  provider,
  playingId,
  showDetails,
  browseAlbum,
  smartPlaylists=false,restricted=false,
  onPlaylistSaved=()=>{},
  editingPlaylist,
  storage="",folder="",catalogTitle,onCloseCatalog,canEdit=false,albumStatus="",
}: {
  api: RMusicClient;
  query: string;
  source: string;
  albumId: string;
  artistId: string;
  revision: number;
  play: (track: Track, list?: Track[]) => Promise<void>;
  startRadio?: (track: Track) => Promise<void>;
  enqueue: (track: Track) => Promise<void>;
  onError: (error: unknown) => void;
  goSettings: () => void;
  provider?: ProviderManifest;
  playingId?: string;
  showDetails:(track:Track)=>void;
  browseAlbum:(id:string)=>void;
  smartPlaylists?:boolean;restricted?:boolean;
  onPlaylistSaved?:()=>void;
  editingPlaylist?:Playlist;
  albumStatus?:string;storage?:string;folder?:string;catalogTitle?:string;onCloseCatalog?:()=>void;canEdit?:boolean;
}) {
  const now=useReleaseClock();const[filters,setFilters]=useState<LibraryFilters>({album_status:albumStatus});const filtersQuery=filterQuery(filters);
  const contextMenu=useTrackContextMenu();
  const [tracks, setTracks] = useState<Track[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(true);
  const paging=usePaging("rmusic.page.tracks:"+JSON.stringify([api.baseUrl,query,source,albumId,artistId,storage,folder,filtersQuery]));
  const [error, setError] = useState("");
  const scroll = useRef<HTMLDivElement>(null);
  const generation = useRef(0);
  const loadedQuery=useRef<{api:RMusicClient;key:string}|undefined>(undefined);
  const loadedAlbums=useRef<{api:RMusicClient;source:string}|undefined>(undefined);
  const dataRevision=editingPlaylist?revision:requestRevision(query,revision);
  const cloudSearch=!editingPlaylist&&!storage&&isCloudSearch(query,source,albumId,artistId);
  const [notice,setNotice]=useState("");
  const [albumPage,setAlbumPage]=useState<Page<Album>>({items:[],total:0});
  const showAlbums=false;
  useEffect(()=>{
    if(!showAlbums)return;
    let active=true;
    const controller=new AbortController();
    if(loadedAlbums.current?.api!==api||loadedAlbums.current.source!==source)setAlbumPage({items:[],total:0});
    loadedAlbums.current={api,source};
    api.request<Page<Album>>(`/library/albums?limit=4&offset=0&source=${encodeURIComponent(source)}`,{signal:controller.signal})
      .then(page=>{if(active)setAlbumPage(page);})
      .catch(error=>{if(active)onError(error);});
    return()=>{active=false;controller.abort();};
  },[api,source,revision,showAlbums,onError]);
  useEffect(() => {
    if((albumId||artistId)&&!editingPlaylist)return;
    const current = ++generation.current;
    const controller = new AbortController();
    const key=JSON.stringify([query,source,albumId,artistId,cloudSearch,paging.page,paging.size,storage,folder,filtersQuery]);
    const changed=loadedQuery.current?.api!==api||loadedQuery.current.key!==key;
    loadedQuery.current={api,key};
    setLoading(true);
    if(changed){setTracks([]);setTotal(0);}
    setError("");
    setNotice("");
    const timer = setTimeout(
      () => {
        const task =
          cloudSearch
            ? api
                .request<{
                  items: Track[];
                  notice?:string;
                }>(
                  source?`/providers/${encodeURIComponent(source)}/search?q=${encodeURIComponent(query)}`:`/search?q=${encodeURIComponent(query)}`,
                  {signal:controller.signal},
                )
                .then((r) => {if(current===generation.current)setNotice(r.notice||"");return { items: r.items, total: r.items.length };})
            : api.request<Page<Track>>(
                `/library/tracks?limit=${paging.size}&offset=${(paging.page-1)*paging.size}&q=${encodeURIComponent(query)}&source=${encodeURIComponent(source)}&album_id=${encodeURIComponent(albumId)}&artist_id=${encodeURIComponent(artistId)}&storage=${storage}&folder=${encodeURIComponent(folder)}&${filtersQuery}`,
                {signal:controller.signal},
              );
        task
          .then((result) => {
            if (current !== generation.current) return;
            setTracks(result.items);
            setTotal(result.total);
            if(!cloudSearch&&paging.page>Math.max(1,Math.ceil(result.total/paging.size)))paging.change(Math.max(1,Math.ceil(result.total/paging.size)));
            if(changed)scroll.current?.scrollTo(0, 0);
          })
          .catch((error) => {
            if (current === generation.current) {
              setError(error.message);
              if(changed){setTracks([]);setTotal(0);}
            }
          })
          .finally(() => {
            if (current === generation.current) setLoading(false);
          });
      },
      query ? 300 : 0,
    );
    return () => {
      ++generation.current;
      clearTimeout(timer);
      controller.abort();
    };
  }, [api, query, source, albumId, artistId, dataRevision, provider?.enabled, cloudSearch,paging.page,paging.size,storage,folder,filtersQuery]);
  if((albumId||artistId)&&!editingPlaylist)return <CatalogDetail key={`${api.baseUrl}:${artistId||albumId}`} api={api} id={artistId||albumId} kind={artistId?"artist":"album"} title={catalogTitle} playingId={playingId} back={onCloseCatalog} canEdit={canEdit} play={play} startRadio={startRadio} enqueue={enqueue} details={showDetails}/>;
  return (
    <div className="library-view">
      {showAlbums && albumPage.items.length > 0 && (
        <section className="recent-albums">
          <div className="section-heading">
            <h2>专辑一览</h2>
            <span className="muted">
              {albumPage.total.toLocaleString()} 张<ChevronRight size={15} />
            </span>
          </div>
          <div className="album-grid">
            {albumPage.items.map((album) => (
              <button
                className="album-item"
                key={album.id}
                aria-label={`浏览专辑 ${album.title}`}
                onClick={() => browseAlbum(album.id)}
              >
                <Cover api={api} track={{ ...album, album: album.title }} />
                <strong>{album.title}</strong>
                <span><CatalogLink track={{...album,album:album.title}}/></span>
                <span className="album-play">
                  <ChevronRight size={21} />
                </span>
              </button>
            ))}
          </div>
        </section>
      )}
      <section className="track-section">
        {!cloudSearch&&!editingPlaylist&&<LibraryFilterBar value={filters} change={setFilters}/>}
        <div className="section-heading">
          <h2>
            {query
              ? "搜索结果"
              : albumId
                ? catalogTitle||"专辑曲目"
                : artistId
                  ? catalogTitle||"艺术家曲目"
                  : "曲库曲目"}
          </h2>
          <span className="muted">
            {loading ? "正在搜索…" : `${total.toLocaleString()} 首`}
          </span>
          {(albumId||artistId)&&tracks[0]&&<RelatedCatalogButton api={api} track={tracks[0]} play={play} enqueue={enqueue} initialKind={artistId?"artist":"album"}/>}
          {smartPlaylists&&!cloudSearch&&!filtersQuery&&<SaveSmartPlaylist key={`${editingPlaylist?.id||""}:${query}:${source}:${albumId}:${artistId}`} api={api} playlist={editingPlaylist} query={{q:query,source,album_id:albumId,artist_id:artistId}} onError={onError} onSaved={onPlaylistSaved}/>}
        </div>
        {source && source !== "local" && !provider?.enabled && (
          <div className="notice">
            {provider?.name || source} 插件尚未启用。
            <button className="text-button" onClick={goSettings}>
              前往设置
            </button>
          </div>
        )}
        {error && <div className="form-error">{error}</div>}
        {notice&&<p className="notice">{notice}</p>}
        {!loading && !tracks.length ? (
          <Empty
            icon={query ? <Search /> : <Music2 />}
            headline={query ? "没有找到这首音乐" : folder?"此文件夹下没有直属音乐文件":"你的音乐库，等待第一首歌"}
            description={
              query
                ? "试试歌曲名、艺术家、拼音或其他音源。"
                : folder?"可继续浏览上方子文件夹。":restricted?"当前授权范围内暂无音乐，请联系管理员调整音源权限。":"添加本地音乐目录，或连接音源。"
            }
            action={
              !restricted&&!query&&!folder ? (
                <button className="primary" onClick={goSettings}>
                  添加音乐源
                </button>
              ) : undefined
            }
          />
        ) : (
          <>
            <div className="track-table-header">
              <span>曲目</span>
              <span>艺术家</span>
              <span>专辑</span>
              <span>音质</span>
              <span>时长</span>
              <span />
            </div>
            <div className="track-scroll" ref={scroll} aria-label="曲目列表">
              <div className="track-page">
                {tracks.map((track) => {
                  return (
                    <div
                      className={`track-row ${playingId === track.id ? "current" : ""} ${track.available ? "" : "unavailable"}`}
                      key={track.id} onContextMenu={e=>contextMenu?.(e,track)}
                    >
                      <button
                        className="track-title"
                        onClick={() => play(track, tracks)}
                        title={`播放 ${track.title}`}
                      >
                        <Cover api={api} track={track} />
                        <span>{track.title} <NewMusic date={releaseDate(track)} now={now}/><small className="mobile-track-artist">{track.artist}</small><small className="track-catalog-facts">{releaseDate(track)||"发行日期未知"} · 聆听 {(track.provider_data as any)?.catalog_display?.play_count??0} 次 · {track.source==="local"?"本地文件":provider?.name||track.source}</small></span>
                        <Play className="row-play" size={16} />
                      </button>
                      <span className="track-artist" title={track.artist}>
                        <CatalogLink track={track}/>
                      </span>
                      <span className="track-album" title={track.album}>
                        <CatalogLink track={track} album/>
                      </span>
                      <span className="track-quality" title={quality(track)}>
                        {quality(track)}
                      </span>
                      <time>{formatTime(track.duration)}</time>
                      <div className="row-actions"><SongButtons track={track}/>
                        <IconButton
                          label={`添加 ${track.title} 到队列`}
                          onClick={() => enqueue(track)}
                        >
                          <ListPlus size={17} />
                        </IconButton>
                        <IconButton label={`关于 ${track.title}`} onClick={()=>showDetails(track)}><Info size={17}/></IconButton>
                        <CacheSongButton api={api} track={track} onError={onError}/><IconButton className="icon-button mobile-track-more" label={`歌曲操作 ${track.title}`} onClick={e=>contextMenu?.(e,track)}><MoreHorizontal size={22}/></IconButton>
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>
          </>
        )}
      </section>
      {!cloudSearch&&<Pagination page={paging.page} size={paging.size} total={total} busy={loading} onChange={paging.change}/>}
    </div>
  );
}
