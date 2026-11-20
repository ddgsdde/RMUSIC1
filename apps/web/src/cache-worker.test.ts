import { readFileSync } from "node:fs";
import vm from "node:vm";
import { IDBFactory, IDBKeyRange } from "fake-indexeddb";
import { describe, it, expect } from "vitest";

const script = readFileSync(
  new URL("../public/cache-worker.js", import.meta.url),
  "utf8",
);
const drains=new WeakMap<IDBFactory,()=>Promise<unknown>>();
function harness(scope="https://music.test/") {
  const indexedDB = new IDBFactory();
  const listeners = new Map<string, (event: unknown) => void>();
  let network: (request: Request) => Promise<Response> = async () => {
    throw new Error("offline");
  };
  const shell=new Map<string,Response>();
  let shellWrites=true;
  const sandbox = vm.createContext({
    indexedDB,
    IDBKeyRange,
    Blob,
    Request,
    Response,
    ReadableStream,
    Headers,
    Set,
    setTimeout,
    clearTimeout,
    URL,
    Map,
    Promise,
    Date,
    caches:{open:async()=>({match:async(request:Request|string)=>shell.get(typeof request==="string"?request:request.url)?.clone(),put:async(request:Request|string,response:Response)=>{if(!shellWrites)throw new Error("quota exceeded");shell.set(typeof request==="string"?request:request.url,response.clone());},keys:async()=>[...shell.keys()].map(url=>new Request(url)),delete:async(request:Request)=>shell.delete(request.url)})},
    fetch: (request: Request) => network(request),
    self: {
      registration:{scope},
      addEventListener: (type: string, handler: (event: unknown) => void) =>
        listeners.set(type, handler),
      skipWaiting: async () => {},
      clients: { claim: async () => {} },
    },
  });
  vm.runInContext(script, sandbox);
  const context=(user="user-a",core="core-a",client="test-client")=>listeners.get("message")!({source:{id:client},data:{type:"cache-context",base:scope.replace(/\/$/,""),core_id:core,user_id:user}});
  context();
  drains.set(indexedDB,()=>vm.runInContext("pendingWrite",sandbox));
  return {context,
    flush:()=>vm.runInContext("pendingWrite",sandbox),
    indexedDB,
    shell,
    denyShellWrites(){shellWrites=false;},
    setNetwork(value: typeof network) {
      network = value;
    },
    async install(){let task:Promise<unknown>|undefined;listeners.get("install")!({waitUntil:(value:Promise<unknown>)=>{task=value;}});await task;},
    async resource(url:string,navigate=false){let response:Promise<Response>|undefined;const request=new Request(url);if(navigate)Object.defineProperty(request,"mode",{value:"navigate"});listeners.get("fetch")!({request,respondWith:(value:Promise<Response>)=>{response=value;}});return response;},
    async limit(limit: number) {
      let task: Promise<unknown> | undefined;
      let ack: unknown;
      listeners.get("message")!({
        data: { type: "cache-limit", limit },
        ports: [
          {
            postMessage(value: unknown) {
              ack = value;
            },
          },
        ],
        waitUntil(value: Promise<unknown>) {
          task = value;
        },
      });
      await task;
      return ack;
    },
    protect(trackId?: string) {
      listeners.get("message")!({
        data: { type: "cache-active", track_id: trackId || null },
        source: { id: "test-client" },
      });
    },
    async request(path: string, range?: string, client="test-client", extra:Record<string,string>={}) {
      let response: Promise<Response> | undefined;
      listeners.get("fetch")!({
        clientId:client,
        waitUntil:()=>{},
        request: new Request(`https://music.test/api/v1/${path}`, {
          headers: {...extra,...(range ? {Range:range} : {})},
        }),
        respondWith(value: Promise<Response>) {
          response = value;
        },
      });
      if (!response) throw new Error("worker did not handle request");
      return response;
    },
  };
}
describe("离线界面资源",()=>{
 it("安装时保存子路径界面及公开资源，断网后可重新加载",async()=>{
  const h=harness("https://music.test/music/");h.setNetwork(async request=>new Response(request.url.endsWith("/music/")?'<html>R-Music<script type="module" src="./assets/app.js"></script><link href="./assets/app.css" rel="stylesheet"></html>':"asset",{headers:{"Content-Type":request.url.endsWith(".js")?"text/javascript":request.url.endsWith(".css")?"text/css":"text/html"}}));
  await h.install();h.setNetwork(async()=>{throw new TypeError("offline");});
  expect(await (await h.resource("https://music.test/music/",true))!.text()).toContain("R-Music");expect(await (await h.resource("https://music.test/music/assets/app.js"))!.text()).toBe("asset");
 });
 it("不保存账号、API、外站或带查询参数的资源",async()=>{
  const h=harness();h.setNetwork(async()=>new Response("secret"));
  for(const url of ["https://music.test/api/v1/auth/me","https://music.test/api/v1/downloads","https://other.test/assets/a.js","https://music.test/assets/a.js?token=private"]){expect(await h.resource(url)).toBeUndefined();}
  expect(h.shell.size).toBe(0);
 });
 it("界面缓存有固定数量上限，且保留主页",async()=>{
  const h=harness();h.setNetwork(async request=>new Response(request.url.endsWith("/")?"<html>R-Music</html>":"asset",{headers:{"Content-Type":request.url.endsWith("/")?"text/html":"text/javascript"}}));
  await h.install();for(let i=0;i<40;i++)await h.resource(`https://music.test/assets/${i}.js`);expect(h.shell.size).toBeLessThanOrEqual(33);expect(h.shell.has("https://music.test/")).toBe(true);
 });
 it("缓存配额写入失败不破坏在线页面响应",async()=>{
  const h=harness();h.denyShellWrites();h.setNetwork(async()=>new Response("<html>online</html>",{headers:{"Content-Type":"text/html"}}));expect(await (await h.resource("https://music.test/",true))!.text()).toBe("<html>online</html>");expect(h.shell.size).toBe(0);
 });
});
function idb<T>(request: IDBRequest<T>) {
  return new Promise<T>((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}
async function entries(indexedDB: IDBFactory) {
  await drains.get(indexedDB)?.();
  const connection = await idb(indexedDB.open("r-music-audio-v1", 2));
  try {
    if(!connection.objectStoreNames.contains("entries"))return [];
    return (await idb(
      connection.transaction("entries").objectStore("entries").getAll(),
    )) as {
      id: string;
      complete: boolean;
      size: number;
      ranges: [number, number][];
      title?: string;
    }[];
  } finally {
    connection.close();
  }
}
function partial(bytes: number[], start: number, total: number) {
  return new Response(new Uint8Array(bytes), {
    status: 206,
    headers: {
      "Content-Type": "audio/flac",
      "ETag": '"revision-1"',
      "Content-Length": String(bytes.length),
      "Content-Range": `bytes ${start}-${start + bytes.length - 1}/${total}`,
    },
  });
}
describe("audio cache worker response semantics", () => {
  it("merges concurrent actual Range reads and serves verified complete coverage offline", async () => {
    const h = harness();
    h.setNetwork(async (request) =>
      request.headers.get("Range") === "bytes=0-3"
        ? partial([1, 2, 3, 4], 0, 8)
        : partial([5, 6, 7, 8], 4, 8),
    );
    await Promise.all([
      (await h.request("media/song", "bytes=0-3")).arrayBuffer(),
      (await h.request("media/song", "bytes=4-7")).arrayBuffer(),
    ]);
    const stored = await entries(h.indexedDB);
    expect(stored).toHaveLength(1);
    expect(stored[0]).toMatchObject({
      complete: true,
      size: 8,
      ranges: [[0, 8]],
    });
    h.setNetwork(async () => {
      throw new Error("offline");
    });
    const response = await h.request("media/song", "bytes=2-5");
    expect(response.status).toBe(206);
    expect([...new Uint8Array(await response.arrayBuffer())]).toEqual([
      3, 4, 5, 6,
    ]);
  });
  it("keeps a skipped track partial and cancels upstream without draining it", async () => {
    const h = harness();
    let produced = 0;
    let cancelled = false;
    h.setNetwork(
      async () =>
        new Response(
          new ReadableStream<Uint8Array>({
            pull(controller) {
              produced++;
              controller.enqueue(new Uint8Array([produced]));
              if (produced === 100) controller.close();
            },
            cancel() {
              cancelled = true;
            },
          }),
          {
            headers: { "ETag": '"revision-1"', "Content-Length": "100", "Content-Type": "audio/flac" },
          },
        ),
    );
    const response = await h.request("media/skipped");
    const reader = response.body!.getReader();
    await reader.read();
    await reader.cancel();
    await new Promise((resolve) => setTimeout(resolve, 20));
    const stored = await entries(h.indexedDB);
    expect(cancelled).toBe(true);
    expect(produced).toBeLessThan(6);
    expect(stored[0].complete).toBe(false);
    expect(stored[0].size).toBeLessThan(100);
  });
  it("never labels unknown-length adaptive audio as a complete original", async () => {
    const h = harness();
    h.setNetwork(
      async () =>
        new Response(
          new ReadableStream<Uint8Array>({
            start(controller) {
              controller.enqueue(new Uint8Array([1, 2, 3, 4]));
              controller.close();
            },
          }),
          { headers: { "Content-Type": "audio/mp4" } },
        ),
    );
    await (
      await h.request("transcode/song?bitrate=128&start=60")
    ).arrayBuffer();
    const stored = await entries(h.indexedDB);
    expect(stored).toHaveLength(0);
  });
  it("does not mistake a CORS-hidden Content-Range for a whole file", async () => {
    const h = harness();
    h.setNetwork(
      async () =>
        new Response(new Uint8Array([1, 2]), {
          status: 206,
          headers: { "Content-Length": "2" },
        }),
    );
    await (await h.request("media/opaque", "bytes=100-101")).arrayBuffer();
    const connection = await idb(h.indexedDB.open("r-music-audio-v1", 2));
    expect(await idb(connection.transaction("entries").objectStore("entries").getAll())).toHaveLength(0);
    connection.close();
  });
  it("evicts the older inactive entry within the configured byte quota", async () => {
    const h = harness();
    h.setNetwork(async () => partial([1, 2, 3, 4], 0, 4));
    await (await h.request("media/first", "bytes=0-3")).arrayBuffer();
    const connection = await idb(h.indexedDB.open("r-music-audio-v1", 2));
    await idb(
      connection
        .transaction("settings", "readwrite")
        .objectStore("settings")
        .put(6, "limit"),
    );
    connection.close();
    await (await h.request("media/second", "bytes=0-3")).arrayBuffer();
    const stored = await entries(h.indexedDB);
    expect(stored).toHaveLength(1);
    expect(stored[0].id).toContain("/media/second");
    expect(stored[0].size).toBeLessThanOrEqual(6);
  });
  it("acknowledges a smaller quota while protecting active playback, then evicts after cancellation", async () => {
    const h = harness();
    h.setNetwork(async () => partial([1, 2, 3, 4], 0, 4));
    await (await h.request("media/old")).arrayBuffer();
    h.setNetwork(
      async () =>
        new Response(
          new ReadableStream<Uint8Array>({
            pull(controller) {
              controller.enqueue(new Uint8Array([1, 2, 3, 4]));
            },
          }),
          { headers: { "ETag": '"revision-1"', "Content-Length": "100" } },
        ),
    );
    const reader = (await h.request("media/playing")).body!.getReader();
    await reader.read();
    expect(await h.limit(1)).toMatchObject({ ok: true, limit: 1 });
    const active = await entries(h.indexedDB);
    expect(active).toHaveLength(1);
    expect(active[0].id).toContain("/media/playing");
    await reader.cancel();
    await h.limit(1);
    expect(await entries(h.indexedDB)).toHaveLength(0);
  });
  it("persists registered music titles with cached audio for offline display", async () => {
    const h = harness();
    h.setNetwork(async () => partial([1, 2, 3, 4], 0, 4));
    await (await h.request("media/init")).arrayBuffer();
    const connection = await idb(h.indexedDB.open("r-music-audio-v1", 2));
    await idb(
      connection
        .transaction("settings", "readwrite")
        .objectStore("settings")
        .put({ title: "夜航", artist: "测试艺术家", core_id:"core-a" }, `track:${JSON.stringify(["core-a","user-a","https://music.test"])}:song`),
    );
    connection.close();
    await (await h.request("media/song")).arrayBuffer();
    expect(
      (await entries(h.indexedDB)).find((entry) => entry.id.endsWith("/song")),
    ).toMatchObject({ title: "夜航", artist: "测试艺术家", core_id:"core-a", complete: true });
  });
  it("隔离同地址不同核心，不能拼接两个核心的分段",async()=>{
    const h=harness();h.setNetwork(async()=>partial([1,2],0,4));await(await h.request("media/song")).arrayBuffer();await h.flush();
    h.context("user-a","core-b");h.setNetwork(async()=>partial([3,4],2,4));await(await h.request("media/song")).arrayBuffer();
    const stored=await entries(h.indexedDB);expect(stored).toHaveLength(2);expect(stored.every(e=>!e.complete)).toBe(true);
  });
  it("protects a fully buffered song while playback is active", async () => {
    const h = harness();
    h.setNetwork(async () => partial([1, 2, 3, 4], 0, 4));
    await (await h.request("media/playing")).arrayBuffer();
    h.protect("playing");
    await h.limit(1);
    expect(await entries(h.indexedDB)).toHaveLength(1);
    h.protect();
    await h.limit(1);
    expect(await entries(h.indexedDB)).toHaveLength(0);
  });
  it("retains longer cached chunks when a shorter Range starts at the same offset", async () => {
    const h = harness();
    h.setNetwork(async (request) =>
      request.headers.get("Range") === "bytes=0-3"
        ? partial([1, 2, 3, 4], 0, 4)
        : partial([1, 2], 0, 4),
    );
    await (await h.request("media/song", "bytes=0-3")).arrayBuffer();
    await (await h.request("media/song", "bytes=0-1")).arrayBuffer();
    h.setNetwork(async () => {
      throw new Error("offline");
    });
    const response = await h.request("media/song");
    expect([...new Uint8Array(await response.arrayBuffer())]).toEqual([
      1, 2, 3, 4,
    ]);
  });
  it("clears old coverage when the source ETag changes", async () => {
    const h = harness();
    let revision = "old";
    h.setNetwork(async () => {
      const response = partial(
        revision === "old" ? [1, 2, 3, 4] : [8, 9],
        0,
        4,
      );
      response.headers.set("ETag", `"${revision}"`);
      return response;
    });
    await (await h.request("media/song", "bytes=0-3")).arrayBuffer();
    revision = "new";
    await (await h.request("media/song", "bytes=0-1")).arrayBuffer();
    expect((await entries(h.indexedDB))[0]).toMatchObject({
      complete: false,
      size: 2,
      ranges: [[0, 2]],
    });
  });
});
describe("账号隔离与在线实体验证",()=>{
 it("同一 URL 在不同账号及浏览器标签中不能命中或拼接",async()=>{
  const h=harness();h.setNetwork(async()=>partial([1,2,3,4],0,4));await(await h.request("media/song?ticket=first-secret")).arrayBuffer();await h.flush();
  h.context("user-b","core-a","second-client");h.setNetwork(async()=>{throw new Error("offline");});
  await expect(h.request("media/song?ticket=second-secret",undefined,"second-client")).rejects.toThrow("offline");
  expect([...new Uint8Array(await(await h.request("media/song")).arrayBuffer())]).toEqual([1,2,3,4]);
  expect(JSON.stringify(await entries(h.indexedDB))).not.toContain("first-secret");
 });
 it("在线已缓存文件只发 HEAD 校验，权限撤销时拒绝使用缓存",async()=>{
  const h=harness();h.setNetwork(async()=>partial([1,2,3,4],0,4));await(await h.request("media/song")).arrayBuffer();await h.flush();
  const methods:string[]=[];h.setNetwork(async req=>{methods.push(req.method);return new Response(null,{headers:{ETag:'"revision-1"',"Content-Length":"4"}});});
  const response=await h.request("media/song");expect(response.headers.get("X-RMusic-Cache")).toBe("hit");expect([...new Uint8Array(await response.arrayBuffer())]).toEqual([1,2,3,4]);expect(methods).toEqual(["HEAD"]);
  h.setNetwork(async()=>new Response(null,{status:403}));expect((await h.request("media/song")).status).toBe(403);
 });
 it("退出后既不回读旧缓存，也不接受迟到的上游写入",async()=>{
  const h=harness();let release!:(r:Response)=>void;h.setNetwork(()=>new Promise(resolve=>{release=resolve;}));const pending=h.request("media/late");
  for(let i=0;i<50;i++)await new Promise(resolve=>setTimeout(resolve,0));h.context("");release(partial([1,2,3,4],0,4));await(await pending).arrayBuffer();expect(await entries(h.indexedDB)).toHaveLength(0);
  h.setNetwork(async()=>{throw new Error("offline");});await expect(h.request("media/late")).rejects.toThrow("offline");
 });
 it("缺少或弱 ETag 的分段不写入可拼接缓存",async()=>{
  const h=harness();h.setNetwork(async()=>{const response=partial([1,2],0,4);response.headers.set("ETag",'W/"weak"');return response;});await(await h.request("media/song")).arrayBuffer();expect(await entries(h.indexedDB)).toHaveLength(0);
 });
 it("离线支持尾部 Range；If-Range 不匹配时返回整个实体",async()=>{
  const h=harness();h.setNetwork(async()=>partial([1,2,3,4],0,4));await(await h.request("media/song")).arrayBuffer();await h.flush();h.setNetwork(async()=>{throw new Error("offline");});
  const tail=await h.request("media/song","bytes=-2");expect(tail.status).toBe(206);expect([...new Uint8Array(await tail.arrayBuffer())]).toEqual([3,4]);
  const full=await h.request("media/song","bytes=1-2","test-client",{"If-Range":'"another"'});expect(full.status).toBe(200);expect(full.headers.get("Content-Length")).toBe("4");
 });
 it("账号切换不会把正在读取的两个版本合并",async()=>{
  const h=harness();h.setNetwork(async()=>partial([1,2],0,4));await(await h.request("media/song")).arrayBuffer();await h.flush();h.context("user-b");h.setNetwork(async()=>partial([3,4],2,4));await(await h.request("media/song")).arrayBuffer();
  const rows=await entries(h.indexedDB);expect(rows).toHaveLength(2);expect(rows.every(row=>!row.complete)).toBe(true);
 });
});
