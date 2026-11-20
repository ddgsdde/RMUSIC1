import {LibraryFilterBar,filterQuery,NewMusic,useReleaseClock,type LibraryFilters} from "./CatalogControls";
import {CatalogLink} from "./CatalogLinks";
import { useEffect, useRef, useState } from "react";
import { ChevronRight, UserRound } from "lucide-react";
import type { Album, Artist, Page, RMusicClient } from "@r-music/sdk";
import {Pagination,usePaging} from "./Pagination";
import { Cover, Empty } from "./components";

export function CollectionBrowser({
  api,
  kind,
  source,
  revision,
  select,
  onError,storage="",selectUnassigned,
}: {
  api: RMusicClient;
  kind: "albums" | "artists";
  source: string;storage?:string;
  revision: number;
  select: (id: string) => void;
  onError: (error: unknown) => void;
  selectUnassigned?:()=>void;
}) {
  const now=useReleaseClock();const[filters,setFilters]=useState<LibraryFilters>({});const filtersQuery=filterQuery(filters);
  const paging=usePaging("rmusic.page.collections:"+JSON.stringify([api.baseUrl,kind,source,storage,filtersQuery]));
  const [items, setItems] = useState<(Album | Artist)[]>([]);
  const [total, setTotal] = useState(0),[unassignedTotal,setUnassignedTotal]=useState(0);
  const [loading, setLoading] = useState(false);
  const generation = useRef(0);
  const loadedQuery=useRef<{api:RMusicClient;key:string}|undefined>(undefined);
  useEffect(() => {
    const current = ++generation.current;
    const key=JSON.stringify([kind,source,paging.page,paging.size,storage,filtersQuery]);
    if(loadedQuery.current?.api!==api||loadedQuery.current.key!==key){setItems([]);setTotal(0);}
    loadedQuery.current={api,key};
    setLoading(true);
    const controller=new AbortController();
    api
      .request<Page<Album | Artist>&{unassigned_total?:number}>(`/library/${kind}?limit=${paging.size}&offset=${(paging.page-1)*paging.size}&source=${encodeURIComponent(source)}&storage=${storage}&${filtersQuery}`,{signal:controller.signal})
      .then((result) => {
        if (current !== generation.current) return;
        setItems(result.items);
        setTotal(result.total);setUnassignedTotal(result.unassigned_total||0);
        const pages=Math.max(1,Math.ceil(result.total/paging.size));if(paging.page>pages)paging.change(pages);
      })
      .catch(error=>{if(current===generation.current)onError(error);})
      .finally(() => {
        if (current === generation.current) setLoading(false);
      });
    return()=>{generation.current++;controller.abort();};
  }, [api, kind, source, revision, onError,paging.page,paging.size,storage,filtersQuery]);
  if (!items.length&&!total&&!unassignedTotal && !filtersQuery)
    return (
      <Empty
        headline={
          loading
            ? "正在读取曲库…"
            : kind === "albums"
              ? "专辑会在这里汇聚"
              : "艺术家在音乐中相遇"
        }
        description="扫描曲库后自动整理，按名称分页浏览。"
      />
    );
  return (
    <>
      <LibraryFilterBar value={filters} change={setFilters} grouped/>
      <p className="muted collection-count">
        共 {total.toLocaleString()} 项 {kind==="albums"&&unassignedTotal>0&&(selectUnassigned?<button className="text-button" onClick={selectUnassigned}>未归属专辑 · {unassignedTotal} 首</button>:<small>未归属专辑 · {unassignedTotal} 首</small>)}
      </p>
      <div
        className={kind === "albums" ? "album-grid expanded" : "artist-list"}
      >
        {items.map((item) =>
          "title" in item ? (
            <button
              className="album-item"
              key={item.id}
              onClick={() => select(item.id)}
            >
              <Cover api={api} track={{ ...item, album: item.title }} />
              <strong>{item.title}</strong><NewMusic date={(item as Album & {release_date?:string}).release_date||""} now={now} album/>
              <small>{(item as Album & {release_date?:string}).release_date||"发行日期未知"} · 聆听 {(item as Album & {play_count?:number}).play_count||0} 次</small>
              <span>
                <CatalogLink track={{...item,album:item.title}}/> · {item.track_count} 首
              </span>
            </button>
          ) : (
            <button key={item.id} onClick={() => select(item.id)}>
              <div className="artist-avatar">
                {item.image_url ? <Cover api={api} track={{cover_url:item.image_url,album:item.name,title:item.name}}/> : <UserRound />}
              </div>
              <div>
                <strong>{item.name}</strong>
                <span>{item.track_count} 首歌曲</span>
              </div>
              <ChevronRight />
            </button>
          ),
        )}
      </div>
      <Pagination page={paging.page} size={paging.size} total={total} busy={loading} onChange={paging.change}/>
    </>
  );
}
