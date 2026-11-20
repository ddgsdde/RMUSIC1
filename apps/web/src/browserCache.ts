import type {ListenOwner} from "./offlineStats";
export interface BrowserCacheEntry {
  id: string;
  track_id: string;
  size: number;
  complete: boolean;
  total: number;
  mime: string;
  last_access: number;
  pinned?: boolean;
  title?: string;
  etag?: string | null;
  artist?: string;
  core_id?: string;
  user_id?: string;
  account_key?: string;
  source?: string;
  ranges: [number, number][];
}
interface Chunk {
  id: string;
  entry_id: string;
  start: number;
  end: number;
  blob: Blob;
}
function database(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open("r-music-audio-v1", 2);
    request.onupgradeneeded = () => {
      const db = request.result;
      if (!db.objectStoreNames.contains("entries")) db.createObjectStore("entries", { keyPath: "id" });
      if (!db.objectStoreNames.contains("chunks")) {
        const chunks = db.createObjectStore("chunks", { keyPath: "id" }); chunks.createIndex("entry_id", "entry_id");
      }
      if (!db.objectStoreNames.contains("settings")) db.createObjectStore("settings");
      const entries = request.transaction!.objectStore("entries");
      if (!entries.indexNames.contains("account_key")) entries.createIndex("account_key", "account_key");
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}
function request<T>(req: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}
interface Context extends ListenOwner {base:string;account_key:string}
const ownerKey="rmusic.browser-cache-owner";
function sourceBase(base="") { return new URL(base || location.origin+location.pathname.replace(/\/index\.html$/,"").replace(/\/$/,""),location.href).href.replace(/\/+$/,""); }
function context():Context|undefined {
  try { const value=JSON.parse(sessionStorage.getItem(ownerKey)||"null") as Context|null;
    if(value?.core_id&&value.user_id&&value.base===sourceBase()&&value.account_key===JSON.stringify([value.core_id,value.user_id,value.base]))return value;
  } catch {} return undefined;
}
function sameContext(value:Context) { return context()?.account_key===value.account_key; }
function postContext(worker:ServiceWorker|null|undefined) {
  const owner=context();worker?.postMessage({type:"cache-context",base:sourceBase(),core_id:owner?.core_id,user_id:owner?.user_id});
}
let registered=false;
export const browserCache = {
  bind(base:string,owner:ListenOwner|null) {
    const source=sourceBase(base),old=context();
    if(owner) {
      const value={...owner,base:source,account_key:JSON.stringify([owner.core_id,owner.user_id,source])};
      if(old?.account_key===value.account_key)return;
      sessionStorage.setItem(ownerKey,JSON.stringify(value));
    } else sessionStorage.removeItem(ownerKey);
    postContext(navigator.serviceWorker?.controller);
    window.dispatchEvent(new Event("rmusic-cache-owner-changed"));
  },
  protect(trackId?: string, owner = "online") {
    navigator.serviceWorker?.controller?.postMessage({
      type: "cache-active",
      track_id: trackId || null,
      owner,
    });
  },
  async rememberTrack(track: { id: string; title: string; artist?: string }, base = "") {
    const owner=context();if(!owner || owner.base!==sourceBase(base))return;
    const core=owner.core_id;
    const db = await database();
    try {
      await request(
        db
          .transaction("settings", "readwrite")
          .objectStore("settings")
          .put(
            { title: track.title, artist: track.artist, core_id: core, user_id:owner.user_id },
            `track:${owner.account_key}:${track.id}`,
          ),
      );
      const entries = await request<BrowserCacheEntry[]>(
        db.transaction("entries").objectStore("entries").index("account_key").getAll(owner.account_key),
      );
      const matching = entries.filter((entry) => entry.track_id === track.id && entry.account_key===owner.account_key);
      if (matching.length && sameContext(owner)) {
        const tx = db.transaction("entries", "readwrite");
        await Promise.all(
          matching.map((entry) =>
            request(
              tx
                .objectStore("entries")
                .put({ ...entry, title: track.title, artist: track.artist }),
            ),
          ),
        );
      }
    } finally {
      db.close();
    }
  },
  async list(): Promise<BrowserCacheEntry[]> {
    const owner=context();if(!owner)return [];
    const db = await database();
    try {
      const entries=await request<BrowserCacheEntry[]>(db.transaction("entries").objectStore("entries").index("account_key").getAll(owner.account_key));
      return sameContext(owner)?entries:[];
    } finally {
      db.close();
    }
  },
  async remove(id: string) {
    const owner=context();if(!owner)throw new Error("请先登录所属账号");
    const db = await database();
    try {
      const entry=await request<BrowserCacheEntry|undefined>(db.transaction("entries").objectStore("entries").get(id));
      if(entry?.account_key!==owner.account_key || !sameContext(owner))throw new Error("不能管理其他账号的缓存");
      await new Promise<void>((resolve, reject) => {
        const tx = db.transaction(["entries", "chunks"], "readwrite");
        tx.objectStore("entries").delete(id);
        const cursor = tx
          .objectStore("chunks")
          .index("entry_id")
          .openCursor(IDBKeyRange.only(id));
        cursor.onsuccess = () => {
          const item = cursor.result;
          if (item) {
            item.delete();
            item.continue();
          }
        };
        tx.oncomplete = () => resolve();
        tx.onerror = () => reject(tx.error);
      });
    } finally {
      db.close();
    }
  },
  async cacheFile(url:string,trackId:string) {
    const owner=context(),worker=navigator.serviceWorker?.controller;
    if(!owner||!worker||!window.isSecureContext)throw new Error("请使用 HTTPS 打开网页，启用本机离线缓存后重试");
    const source=new URL(url,location.href);
    if(source.origin!==location.origin||!source.pathname.startsWith(new URL(owner.base).pathname.replace(/\/$/,"")+"/api/v1/media/")||decodeURIComponent(source.pathname.split("/").at(-1)!)!==trackId)throw new Error("离线音源与当前核心不匹配");
    const controller=new AbortController();const changed=()=>controller.abort();window.addEventListener("rmusic-cache-owner-changed",changed);
    try {
      const response=await fetch(source,{headers:{"X-RMusic-Offline-Save":"1"},signal:controller.signal,credentials:"omit",redirect:"error",cache:"no-store"});
      if(!response.ok||!response.body||!/^"[^"\r\n]+"$/.test(response.headers.get("ETag")||"")||!Number.isSafeInteger(Number(response.headers.get("Content-Length")))||Number(response.headers.get("Content-Length"))<=0){await response.body?.cancel();throw new Error("音源没有提供可验证的完整原文件，暂不能离线保存");}
      const reader=response.body.getReader();try{while(!(await reader.read()).done){if(!sameContext(owner))throw new Error("账号已切换，下载已停止");}}finally{await reader.cancel().catch(()=>{});reader.releaseLock();}
      await new Promise<void>((resolve,reject)=>{
        const channel=new MessageChannel(),timeout=setTimeout(()=>{channel.port1.close();reject(new Error("缓存写入仍在处理中，请稍后查看"));},5000);
        channel.port1.onmessage=()=>{clearTimeout(timeout);channel.port1.close();resolve();};worker.postMessage({type:"cache-sync"},[channel.port2]);
      });
      if(!sameContext(owner))throw new Error("账号已切换");
      const items=await this.list();if(!items.some(entry=>entry.track_id===trackId&&entry.complete))throw new Error("缓存未完整保存，请检查缓存空间后重试");
    } finally {window.removeEventListener("rmusic-cache-owner-changed",changed);}
  },
  async setLimit(limit: number) {
    if (!Number.isFinite(limit) || limit <= 0)
      throw new Error("缓存上限必须大于 0");
    const db = await database();
    try {
      await request(
        db
          .transaction("settings", "readwrite")
          .objectStore("settings")
          .put(limit, "limit"),
      );
    } finally {
      db.close();
    }
    const controller = navigator.serviceWorker?.controller;
    if (controller) {
      await new Promise<void>((resolve, reject) => {
        const channel = new MessageChannel();
        const timeout = setTimeout(() => {
          channel.port1.close();
          reject(new Error("缓存管理器未响应，请刷新页面后重试"));
        }, 5000);
        channel.port1.onmessage = (event) => {
          clearTimeout(timeout);
          channel.port1.close();
          if (event.data?.ok) resolve();
          else reject(new Error(event.data?.error || "缓存容量设置失败"));
        };
        controller.postMessage({ type: "cache-limit", limit }, [channel.port2]);
      });
    }
    // Without a controller, defer eviction until the worker owns active streams.
  },
  async blob(id: string): Promise<Blob> {
    const owner=context();if(!owner)throw new Error("请先登录所属账号");
    const db = await database();
    try {
      const tx = db.transaction(["entries", "chunks"]);
      const entry = await request<BrowserCacheEntry | undefined>(
        tx.objectStore("entries").get(id),
      );
      if(entry?.account_key!==owner.account_key || !sameContext(owner))throw new Error("不能读取其他账号的缓存");
      if (!entry?.complete) throw new Error("歌曲缓存尚未完整，不能离线播放");
      const chunks = await request<Chunk[]>(
        db
          .transaction("chunks")
          .objectStore("chunks")
          .index("entry_id")
          .getAll(id),
      );
      chunks.sort((a, b) => a.start - b.start);
      let offset = 0;
      const parts: Blob[] = [];
      for (const chunk of chunks) {
        if (chunk.start > offset) throw new Error("缓存数据存在缺口");
        if (chunk.end > offset) {
          parts.push(chunk.blob.slice(Math.max(0, offset - chunk.start)));
          offset = chunk.end;
        }
      }
      if (offset < entry.total) throw new Error("缓存未覆盖整首歌曲");
      if(!sameContext(owner))throw new Error("缓存账号已切换");
      return new Blob(parts, { type: entry.mime });
    } finally {
      db.close();
    }
  },
  async register(base: string) {
    if (!("serviceWorker" in navigator) || !window.isSecureContext)
      return false;
    const scope = new URL("./", base.endsWith("/") ? base : `${base}/`);
    if (scope.origin !== location.origin) return false;
    const registration=await navigator.serviceWorker.register(new URL("cache-worker.js", scope), {
      scope: scope.pathname,
    });
    if(!registered) {
      registered=true;
      navigator.serviceWorker.addEventListener("controllerchange",()=>postContext(navigator.serviceWorker.controller));
      navigator.serviceWorker.addEventListener("message",event=>{if(event.data?.type==="cache-context-needed")postContext(navigator.serviceWorker.controller);});
    }
    postContext(navigator.serviceWorker.controller || registration.active);
    return true;
  },
};
export function formatBytes(value: number) {
  if (value >= 1024 ** 3) return `${(value / 1024 ** 3).toFixed(1)} GiB`;
  if (value >= 1024 ** 2) return `${(value / 1024 ** 2).toFixed(1)} MiB`;
  return `${(value / 1024).toFixed(0)} KiB`;
}
