/* Audio caching only persists chunks requested by the player. Cancellation never drains upstream audio. */
const DATABASE = "r-music-audio-v1";
const SHELL_ROOT = new URL(self.registration.scope);
const SHELL_CACHE = `rmusic-shell-v1:${SHELL_ROOT.href}`;
function shellAsset(url) {
  return url.origin === SHELL_ROOT.origin && !url.search &&
    url.pathname.startsWith(`${SHELL_ROOT.pathname}assets/`) &&
    /\.(?:js|css|woff2?|png|jpe?g|webp|svg)$/.test(url.pathname);
}
async function shellResponse(request, document = false) {
  let cache;
  try { cache = await caches.open(SHELL_CACHE); } catch { return fetch(request); }
  const key = document ? SHELL_ROOT.href : request.url;
  try {
    const response = await fetch(request);
    const type = response.headers.get("Content-Type") || "";
    if (response.ok && !response.redirected && Number(response.headers.get("Content-Length")) <= 4 * 1024 ** 2 &&
      (document ? /text\/html/i.test(type) : /javascript|text\/css|image\/|font\//i.test(type))) {
      try {
        await cache.put(key, response.clone());
        const keys = await cache.keys();
        for (const old of keys.filter(item => item.url !== SHELL_ROOT.href).slice(0, Math.max(0, keys.length - 33))) await cache.delete(old);
      } catch { /* Storage limits must not break a successful online response. */ }
    }
    if (!response.ok) return await cache.match(key) || response;
    return response;
  } catch (error) {
    const saved = await cache.match(key);
    if (saved) return saved;
    throw error;
  }
}
async function warmShell() {
  try {
    const response = await shellResponse(new Request(SHELL_ROOT.href), true);
    if (!response.ok || !/text\/html/i.test(response.headers.get("Content-Type") || "")) return;
    const html = await response.text();
    const assets = [...html.matchAll(/(?:src|href)=["']([^"']+)["']/g)].map(match => new URL(match[1], SHELL_ROOT)).filter(shellAsset);
    await Promise.allSettled([...new Set(assets.map(url => url.href))].map(url => shellResponse(new Request(url))));
  } catch { /* Offline first installation can retry on the next successful visit. */ }
}
// A context belongs to one browser client and is replaced on login/logout.
// No bearer token or expiring media ticket is persisted.
let pendingWrite = Promise.resolve();
let queuedBytes = 0;
const contexts = new Map();
const activeEntries = new Map();
const activePlayback = new Map();
const contextWaiters = new Map();
function valid(context) { return context && contexts.get(context.client) === context; }
function protectedEntry(entry) {
  if (entry.pinned || activeEntries.has(entry.id)) return true;
  for (const [client, playback] of activePlayback) {
    if (playback.expires < Date.now()) activePlayback.delete(client);
    else if (playback.account_key === entry.account_key && playback.track_id === entry.track_id) return true;
  }
  return false;
}
function queuePersist(context, ...args) {
  const bytes = args[3];
  // Dropped writes stay as explicit coverage gaps; playback never waits for disk.
  if (!valid(context) || queuedBytes + bytes.byteLength > 2 * 1024 ** 2) return;
  queuedBytes += bytes.byteLength;
  const task = pendingWrite.then(() => valid(context) && persist(context, ...args));
  pendingWrite = task.catch(() => {}).finally(() => { queuedBytes -= bytes.byteLength; });
  return pendingWrite;
}
function release(id) {
  const remaining = (activeEntries.get(id) || 1) - 1;
  if (remaining > 0) activeEntries.set(id, remaining); else activeEntries.delete(id);
}
function db() {
  return new Promise((resolve, reject) => {
    const r = indexedDB.open(DATABASE, 2);
    r.onupgradeneeded = () => {
      const connection = r.result;
      if (!connection.objectStoreNames.contains("entries")) connection.createObjectStore("entries", { keyPath: "id" });
      if (!connection.objectStoreNames.contains("chunks")) {
        const chunks = connection.createObjectStore("chunks", { keyPath: "id" });
        chunks.createIndex("entry_id", "entry_id");
      }
      if (!connection.objectStoreNames.contains("settings")) connection.createObjectStore("settings");
      const entries = r.transaction.objectStore("entries");
      if (!entries.indexNames.contains("account_key")) entries.createIndex("account_key", "account_key");
    };
    r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error);
  });
}
function result(r) {
  return new Promise((resolve, reject) => { r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error); });
}
function mergedRanges(ranges) {
  const output = [];
  for (const range of ranges.sort((a,b) => a[0]-b[0])) {
    const last = output.at(-1);
    if (last && range[0] <= last[1]) last[1] = Math.max(last[1],range[1]); else output.push([...range]);
  }
  return output;
}
async function persist(context,id,trackId,start,bytes,total,mime,etag) {
  if (!valid(context)) return;
  const connection = await db();
  try {
    let current = await result(connection.transaction("entries").objectStore("entries").get(id));
    if (current && (current.total !== total || current.etag !== etag)) {
      await removeEntry(connection,id); current = undefined;
    }
    const metadata = await result(connection.transaction("settings").objectStore("settings").get(`track:${context.account_key}:${trackId}`));
    const limit = await result(connection.transaction("settings").objectStore("settings").get("limit")) || 5 * 1024 ** 3;
    const all = await result(connection.transaction("entries").objectStore("entries").getAll());
    let used = all.reduce((sum,entry) => sum+entry.size,0);
    const chunkId = `${id}:${start}:${start+bytes.byteLength}`;
    const previous = await result(connection.transaction("chunks").objectStore("chunks").get(chunkId));
    const added = bytes.byteLength - (previous?.blob.size || 0);
    for (const entry of all.sort((a,b) => a.last_access-b.last_access)) {
      if (used+added <= limit) break;
      if (entry.id===id || protectedEntry(entry)) continue;
      if (!valid(context)) return;
      await removeEntry(connection,entry.id); used -= entry.size;
    }
    if (!valid(context) || used+added>limit) return;
    const ranges = mergedRanges([...(current?.ranges || []),[start,start+bytes.byteLength]]);
    await new Promise((resolve,reject) => {
      const tx = connection.transaction(["entries","chunks"],"readwrite");
      context.writes.add(tx);
      tx.objectStore("entries").put({...current,id,track_id:trackId,account_key:context.account_key,core_id:context.core_id,user_id:context.user_id,source:context.base,
        title:metadata?.title || current?.title,artist:metadata?.artist || current?.artist,size:(current?.size || 0)+added,total,mime,etag,last_access:Date.now(),ranges,
        complete:ranges.length===1 && ranges[0][0]===0 && ranges[0][1]===total});
      tx.objectStore("chunks").put({id:chunkId,entry_id:id,start,end:start+bytes.byteLength,blob:new Blob([bytes])});
      tx.oncomplete=()=>{context.writes.delete(tx);resolve();};
      tx.onabort=tx.onerror=()=>{context.writes.delete(tx);reject(tx.error || new Error("缓存写入已取消"));};
    });
  } finally { connection.close(); }
}
function removeEntry(connection,id) {
  return new Promise((resolve,reject) => {
    const tx=connection.transaction(["entries","chunks"],"readwrite");tx.objectStore("entries").delete(id);
    const r=tx.objectStore("chunks").index("entry_id").openCursor(id);
    r.onsuccess=()=>{if(r.result){r.result.delete();r.result.continue();}};
    tx.oncomplete=resolve;tx.onerror=()=>reject(tx.error);
  });
}
function strongEtag(value) { return value && /^"[^"\r\n]+"$/.test(value); }
async function cachedResponse(request,id,context) {
  if (!valid(context)) return null;
  const connection=await db();
  try {
    const entry=await result(connection.transaction("entries").objectStore("entries").get(id));
    if (!entry?.complete || entry.account_key!==context.account_key || !strongEtag(entry.etag)) return null;
    const chunks=await result(connection.transaction("chunks").objectStore("chunks").index("entry_id").getAll(id));
    chunks.sort((a,b)=>a.start-b.start);let offset=0;const parts=[];
    for(const chunk of chunks){if(chunk.start>offset)return null;if(chunk.end>offset){parts.push(chunk.blob.slice(Math.max(0,offset-chunk.start),entry.total-chunk.start));offset=chunk.end;}}
    if(offset<entry.total || !valid(context))return null;
    let start=0,end=entry.total-1,partial=false;
    const requested=request.headers.get("Range"),ifRange=request.headers.get("If-Range");
    if(requested && (!ifRange || ifRange===entry.etag)) {
      const range=requested.match(/^bytes=(\d*)-(\d*)$/);
      if(!range || !range[1]&&!range[2])return null;
      start=range[1]?Number(range[1]):Math.max(0,entry.total-Number(range[2]));
      end=range[1]&&range[2]?Math.min(Number(range[2]),end):end;partial=true;
      if(!Number.isSafeInteger(start)||!Number.isSafeInteger(end)||start>=entry.total||end<start || !range[1]&&Number(range[2])===0)
        return new Response(null,{status:416,headers:{"Content-Range":`bytes */${entry.total}`}});
    }
    const headers={"Content-Type":entry.mime,"Accept-Ranges":"bytes","Content-Length":String(end-start+1),"ETag":entry.etag,"Cache-Control":"private, no-store","X-RMusic-Cache":"hit"};
    if(partial)headers["Content-Range"]=`bytes ${start}-${end}/${entry.total}`;
    const blob=new Blob(parts,{type:entry.mime});
    return new Response(blob.slice(start,end+1),{status:partial?206:200,headers});
  } finally { connection.close(); }
}
async function getContext(client) {
  if(contexts.has(client))return contexts.get(client);
  const page=await self.clients.get?.(client);if(!page)return null;
  return new Promise(resolve=>{
    const timer=setTimeout(()=>{contextWaiters.delete(client);resolve(null);},250);
    contextWaiters.set(client,value=>{clearTimeout(timer);contextWaiters.delete(client);resolve(value);});
    page.postMessage({type:"cache-context-needed"});
  });
}
async function audio(event,url,trackId) {
  const request=event.request,context=await getContext(event.clientId);
  // Converted PCM/AAC streams have a different entity and no stable byte length.
  // They keep the player's bounded RAM buffering and are never called originals.
  if(!valid(context) || url.origin+url.pathname!==`${context.base}/api/v1/media/${encodeURIComponent(trackId)}`) return fetch(request);
  const id=`${context.account_key}|${url.origin}${url.pathname}`;
  let cached=await cachedResponse(request,id,context).catch(()=>null);
  if(cached) {
    try {
      const headers=new Headers(request.headers);headers.delete("Range");headers.delete("If-Range");
      const head=await fetch(new Request(request,{method:"HEAD",headers,cache:"no-store"}));
      if(!valid(context))return fetch(request);
      // Authorization failures and policy changes must never be hidden by cache.
      if(head.status===401 || head.status===403 || head.status===404)return head;
      if(head.ok && head.headers.get("ETag")===cached.headers.get("ETag") && Number(head.headers.get("Content-Length"))===Number(cached.headers.get("Content-Range")?.split("/")[1] || cached.headers.get("Content-Length")))return cached;
    } catch(error) { if(valid(context))return cached; throw error; }
  }
  let response;
  try { response=await fetch(request); }
  catch(error) { cached=await cachedResponse(request,id,context).catch(()=>null);if(cached)return cached;throw error; }
  if(!response.ok || !response.body || ![200,206].includes(response.status) || !valid(context))return response;
  const range=response.headers.get("Content-Range")?.match(/^bytes (\d+)-(\d+)\/(\d+)$/);
  if(response.status===206 && !range)return response;
  let offset=range?Number(range[1]):0;
  const total=range?Number(range[3]):Number(response.headers.get("Content-Length")),etag=response.headers.get("ETag");
  if(!Number.isSafeInteger(total)||total<=0 || !strongEtag(etag) || !Number.isSafeInteger(offset) || offset>=total || range && (Number(range[2])<offset || Number(range[2])>=total) || response.headers.get("Content-Encoding") && response.headers.get("Content-Encoding")!=="identity")return response;
  activeEntries.set(id,(activeEntries.get(id)||0)+1);
  const reader=response.body.getReader(),mime=response.headers.get("Content-Type")||"application/octet-stream";
  let closed=false,finish;const lifetime=new Promise(resolve=>{finish=resolve;});
  event.waitUntil(lifetime.then(()=>pendingWrite));
  const close=()=>{if(!closed){closed=true;release(id);finish();}};
  return new Response(new ReadableStream({
    async pull(controller) {
      try {
        const {done,value}=await reader.read();
        if(done){close();controller.close();return;}
        const start=offset;offset+=value.byteLength;
        if(offset<=total) {
          // Explicit offline saves accept disk backpressure. Live playback never
          // waits for cache writes and retains its bounded best-effort queue.
          const saving=request.headers.get("X-RMusic-Offline-Save")==="1";
          if(saving)await pendingWrite;
          const write=queuePersist(context,id,trackId,start,value,total,mime,etag);
          if(saving)await write;
        }
        controller.enqueue(value);
      }catch(error){close();controller.error(error);}
    },cancel(reason){close();return reader.cancel(reason);}
  }),{status:response.status,statusText:response.statusText,headers:response.headers});
}
self.addEventListener("install",event=>event.waitUntil(Promise.all([self.skipWaiting(),warmShell()])));
self.addEventListener("activate",event=>event.waitUntil(self.clients.claim()));
self.addEventListener("fetch",event=>{
  const url=new URL(event.request.url),match=url.pathname.match(/\/api\/v1\/(?:media|transcode)\/([^/]+)$/);
  if(event.request.method==="GET" && url.origin===SHELL_ROOT.origin && match)
    event.respondWith(audio(event,url,decodeURIComponent(match[1])));
  else if(event.request.method==="GET" && url.origin===SHELL_ROOT.origin && !url.search && ((event.request.mode==="navigate" && [SHELL_ROOT.pathname,`${SHELL_ROOT.pathname}index.html`].includes(url.pathname))||shellAsset(url)))
    event.respondWith(shellResponse(event.request,event.request.mode==="navigate"));
});
self.addEventListener("message",event=>{
  const source=event.source?.id;
  if(event.data?.type==="cache-context" && source) {
    const old=contexts.get(source);contexts.delete(source);
    if(old)for(const tx of old.writes){try{tx.abort();}catch{}}
    for(const key of activePlayback.keys())if(key.startsWith(`${source}:`))activePlayback.delete(key);
    const value=event.data;
    if(value.core_id && value.user_id && typeof value.core_id==="string" && typeof value.user_id==="string" && value.core_id.length<=200 && value.user_id.length<=200) {
      try {
        const base=new URL(value.base,SHELL_ROOT);
        if(base.origin===SHELL_ROOT.origin && !base.search && !base.hash && !base.username && !base.password) {
          const context={client:source,core_id:value.core_id,user_id:value.user_id,base:base.href.replace(/\/+$/,""),account_key:JSON.stringify([value.core_id,value.user_id,base.href.replace(/\/+$/,"")]),writes:new Set()};
          contexts.set(source,context);
        }
      }catch{}
    }
    contextWaiters.get(source)?.(contexts.get(source));
    event.ports?.[0]?.postMessage({ok:true});return;
  }
  const context=contexts.get(source);
  if(event.data?.type==="cache-active") {
    if(!context)return;
    const client=`${source}:${event.data.owner || "online"}`;
    if(event.data.track_id)activePlayback.set(client,{account_key:context.account_key,track_id:event.data.track_id,expires:Date.now()+30000});else activePlayback.delete(client);
    return;
  }
  if(event.data?.type==="cache-sync") {event.waitUntil(pendingWrite.then(()=>event.ports?.[0]?.postMessage({ok:true})));return;}
  if(event.data?.type!=="cache-limit")return;
  const task=pendingWrite.then(async()=>{
    const limit=Number(event.data.limit);if(!Number.isFinite(limit)||limit<=0)throw new Error("缓存上限必须大于0");
    const connection=await db();
    try {
      await result(connection.transaction("settings","readwrite").objectStore("settings").put(limit,"limit"));
      const entries=await result(connection.transaction("entries").objectStore("entries").getAll());let used=entries.reduce((sum,e)=>sum+e.size,0);
      for(const entry of entries.sort((a,b)=>a.last_access-b.last_access)){if(used<=limit)break;if(protectedEntry(entry))continue;await removeEntry(connection,entry.id);used-=entry.size;}
      event.ports?.[0]?.postMessage({ok:true,used,limit});
    }finally{connection.close();}
  });pendingWrite=task.catch(()=>{});event.waitUntil(task.catch(error=>event.ports?.[0]?.postMessage({ok:false,error:String(error.message||error)})));
});
