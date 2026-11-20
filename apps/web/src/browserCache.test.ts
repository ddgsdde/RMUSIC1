// @vitest-environment jsdom
import {Blob as NodeBlob} from "node:buffer";
import {IDBFactory,IDBKeyRange} from "fake-indexeddb";
import {beforeEach,afterEach,it,expect,vi} from "vitest";
import {browserCache} from "./browserCache";
const owner={core_id:"core-a",user_id:"user-a"};
const base=()=>location.origin+location.pathname.replace(/\/$/,"");
const key=(user:string)=>JSON.stringify(["core-a",user,base()]);
function request<T>(r:IDBRequest<T>):Promise<T>{return new Promise((resolve,reject)=>{r.onsuccess=()=>resolve(r.result);r.onerror=()=>reject(r.error);});}
beforeEach(()=>{sessionStorage.clear();vi.stubGlobal("Blob",NodeBlob);vi.stubGlobal("indexedDB",new IDBFactory());vi.stubGlobal("IDBKeyRange",IDBKeyRange);});
afterEach(()=>vi.unstubAllGlobals());
async function seed(){
 browserCache.bind(base(),owner);await browserCache.list();const db=await request(indexedDB.open("r-music-audio-v1",2));
 const tx=db.transaction(["entries","chunks"],"readwrite");
 for(const [id,user]of [["a","user-a"],["b","user-b"],["legacy",undefined]]){
  tx.objectStore("entries").put({id,track_id:"song",account_key:user?key(user):undefined,core_id:"core-a",user_id:user,source:base(),total:4,size:4,complete:true,mime:"audio/flac",ranges:[[0,4]]});
  tx.objectStore("chunks").put({id:`${id}:0`,entry_id:id,start:0,end:4,blob:new Blob([new Uint8Array([1,2,3,4])])});
 }
 await new Promise<void>((resolve,reject)=>{tx.oncomplete=()=>resolve();tx.onerror=()=>reject(tx.error);});db.close();
}
it("列表、文件读取和删除均按账号隔离，旧缓存不推定归属",async()=>{
 await seed();expect((await browserCache.list()).map(e=>e.id)).toEqual(["a"]);expect((await browserCache.blob("a")).size).toBe(4);
 await expect(browserCache.blob("b")).rejects.toThrow("其他账号");await expect(browserCache.blob("legacy")).rejects.toThrow("其他账号");await expect(browserCache.remove("b")).rejects.toThrow("其他账号");
 browserCache.bind(base(),{...owner,user_id:"user-b"});expect((await browserCache.list()).map(e=>e.id)).toEqual(["b"]);
 browserCache.bind(base(),null);expect(await browserCache.list()).toEqual([]);await expect(browserCache.blob("a")).rejects.toThrow("先登录");
});
it("曲目资料只更新所属账号的缓存",async()=>{
 await seed();await browserCache.rememberTrack({id:"song",title:"自己的歌曲",artist:"A"},base());expect((await browserCache.list())[0].title).toBe("自己的歌曲");
 browserCache.bind(base(),{...owner,user_id:"user-b"});expect((await browserCache.list())[0].title).toBeUndefined();
});
