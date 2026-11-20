import {CatalogSort,NewMusic,useReleaseClock} from './CatalogControls';
import {useEffect, useState, useRef} from 'react';
import type {RMusicClient} from '@r-music/sdk';
import {Cover} from './components';
import {useCatalogNavigation} from './CatalogLinks';

type Album = {id: string; title: string; source: string; cover_url?: string; release_date?: string; track_count?: number};
type Page = {source?: string; items: Album[]; total?: number; page?: number; next_page?: number; notice?: string;sorts?:string[]};
type Catalog = {local: Page; platforms: Page[]};
export function ArtistAlbums({api, artistID}: {api: RMusicClient; artistID: string}) {
  const now=useReleaseClock();const[sort,setSort]=useState("default"),[localSort,setLocalSort]=useState("added");
  const navigation = useCatalogNavigation();
  const force = useRef(false);
  const [data, setData] = useState<Catalog>();
  const [page, setPage] = useState(1), [provider, setProvider] = useState('');
  const [revision, setRevision] = useState(0), [error, setError] = useState(''), [busy, setBusy] = useState(false);
  useEffect(() => {
    const abort = new AbortController(); setBusy(true); setError('');
    const query = new URLSearchParams({page: String(page), provider, sort, local_sort:localSort, refresh: String(force.current)}); force.current = false;
    api.request<Catalog>(`/library/catalog/artist/${encodeURIComponent(artistID)}/albums?${query}`, {signal: abort.signal})
      .then(value => {if (!abort.signal.aborted) setData(value);})
      .catch(e => {if (!abort.signal.aborted) setError(e.message);})
      .finally(() => {if (!abort.signal.aborted) setBusy(false);});
    return () => abort.abort();
  }, [api, artistID, page, provider, revision,sort,localSort]);
  function group(value: Page, title: string, source: string) {
    const current = value.page || 1;
    return <section key={source} className="artist-discography">
      <div className="section-heading"><h2>{title}</h2><CatalogSort label={source==="local"?"本地专辑排序":"平台专辑排序"} value={source==="local"?localSort:sort} options={source==="local"?["added","title","title_desc","release_new","release_old","plays"]:value.sorts||["default"]} change={v=>{if(source==="local")setLocalSort(v);else setSort(v);setPage(1);}}/>{value.total !== undefined && <small>{value.total} 张</small>}</div>
      {value.notice && <p className="muted">{value.notice}</p>}
      <div className="artist-album-grid">{value.items.map(album => <button key={album.id} className="artist-album-card" onClick={() => navigation?.browse('album', album.id, album.title)} aria-label={`打开专辑 ${album.title}`}>
        <Cover api={api} track={{cover_url: album.cover_url, title: album.title, album: album.title}}/>
        <strong>{album.title}</strong><NewMusic date={album.release_date||""} now={now} album/><small>{album.release_date || album.source}</small>
      </button>)}</div>
      {(current > 1 || value.next_page) && <div className="entity-paging">
        <button disabled={busy || current === 1} onClick={() => {setProvider(source); setPage(current - 1);}}>上一页</button><small>第 {current} 页</small>
        <button disabled={busy || !value.next_page} onClick={() => {setProvider(source); setPage(value.next_page!);}}>下一页</button>
      </div>}
    </section>;
  }
  return <div>
    <div className="section-heading"><h2>专辑</h2><button disabled={busy} onClick={() => {force.current = true; setRevision(n => n + 1);}}>更新专辑</button></div>
    {busy && <p role="status">正在读取专辑…</p>}{error && <p role="alert">{error}</p>}
    {data?.platforms.map(value => group(value, value.source === 'qqmusic' ? 'QQ 音乐' : value.source || '平台专辑', value.source || ''))}
    {!!data?.local.total && <details><summary>已收录专辑 · {data.local.total} 张</summary>{group(data.local, '已收录专辑', 'local')}</details>}
    {!busy && data && !data.local.items.length && data.platforms.every(p => !p.items.length) && <p className="muted">暂无可确认的专辑</p>}
  </div>;
}
