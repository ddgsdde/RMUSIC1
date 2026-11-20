// @vitest-environment jsdom
import "fake-indexeddb/auto";
import {beforeEach,expect,it} from "vitest";
import {ListenSampler,listenJournal,listenTransaction} from "../offlineStats";
import {pcmJournal} from "./pcm-journal";
const owner={core_id:"core",user_id:"user"},boundary={playback_id:"source",track_id:"track",position:0,frames:0};
beforeEach(async()=>{localStorage.clear();sessionStorage.clear();await new Promise<void>((yes,no)=>{const read=indexedDB.deleteDatabase("r-music-listens-v1");read.onsuccess=()=>yes();read.onerror=()=>no(read.error);});});
function begin(){return pcmJournal.begin({owner,endpoint:"browser-qa",zone:"zone",plan:"plan",rate:48000,heard:[boundary],terminal:false});}
function snapshot(){const sampler=new ListenSampler({track_id:"track"},120);sampler.sample(0,true,false,1000,0);sampler.sample(1,true,false,2000,1000);const value=sampler.finish("interrupted");value.measured={playback_id:"source",listen_session_id:"group",segments:[{start:0,end:1,last_at:2000}]};return value;}
it("upgrades an existing listen database while retaining acknowledged history",async()=>{
  const connection=await new Promise<IDBDatabase>((yes,no)=>{const read=indexedDB.open("r-music-listens-v1",1);read.onupgradeneeded=()=>read.result.createObjectStore("listens",{keyPath:"id"});read.onsuccess=()=>yes(read.result);read.onerror=()=>no(read.error);});
  const value=snapshot();await new Promise<void>((yes,no)=>{const tx=connection.transaction("listens","readwrite");tx.objectStore("listens").put({id:value.id,client_id:"old-device",owner,snapshot:value,ack:value.version});tx.oncomplete=()=>yes();tx.onerror=()=>no(tx.error);});connection.close();
  await begin();expect(await listenJournal.pending(owner)).toEqual([]);
  const rows=await listenTransaction<unknown[]>(["listens"],"readonly",(tx,done)=>{const read=tx.objectStore("listens").getAll();read.onsuccess=()=>done(read.result);});expect(rows).toHaveLength(1);
});
it("saves a cursor and its actual listening evidence together and isolates Core and user",async()=>{
  const cursor=await begin(),value=snapshot();await pcmJournal.checkpoint({...cursor,version:2,heard:[{...boundary,position:1,frames:48000}]},[value]);
  expect((await pcmJournal.load(owner,cursor.endpoint,cursor.zone))?.heard[0].frames).toBe(48000);expect((await listenJournal.pending(owner))[0].snapshot).toEqual(value);
  expect(await pcmJournal.load({...owner,user_id:"other"},cursor.endpoint,cursor.zone)).toBeUndefined();expect(await pcmJournal.load({...owner,core_id:"other"},cursor.endpoint,cursor.zone)).toBeUndefined();
  await expect(listenTransaction(["listens","cursors"],"readwrite",tx=>{tx.objectStore("cursors").delete(cursor.id);tx.objectStore("listens").delete(value.id);throw new Error("interrupted transaction");})).rejects.toThrow("interrupted transaction");
  expect(await pcmJournal.load(owner,cursor.endpoint,cursor.zone)).toBeDefined();expect(await listenJournal.pending(owner)).toHaveLength(1);
});
it("freezes one immutable recovery request before transmission while later listening remains durable",async()=>{
  const cursor=await begin(),frozen=await pcmJournal.freeze(cursor),value=snapshot();
  await pcmJournal.checkpoint({...cursor,version:2,heard:[{...boundary,position:1,frames:48000}]},[value]);
  expect((await pcmJournal.freeze(cursor))?.request).toEqual(frozen?.request);expect((await pcmJournal.load(owner,cursor.endpoint,cursor.zone))?.heard[0].frames).toBe(0);
  expect(await listenJournal.pending(owner)).toHaveLength(1);
});
it("late old workers cannot reactivate retired or replacement cursors",async()=>{
  const old=await begin();await pcmJournal.retire(old);await pcmJournal.checkpoint({...old,version:3},[snapshot()]);expect(await pcmJournal.load(owner,old.endpoint,old.zone)).toBeUndefined();
  const next=await begin();await pcmJournal.retire(old);await pcmJournal.checkpoint({...old,version:4},[]);expect((await pcmJournal.load(owner,next.endpoint,next.zone))?.run).toBe(next.run);
});
it("rejected listening evidence prevents automatic cursor recovery",async()=>{
  const cursor=await begin(),value=snapshot();await pcmJournal.checkpoint({...cursor,version:2},[value]);await listenJournal.acknowledge(owner,{accepted:[],rejected:[{id:value.id,version:value.version,code:"conflicting_snapshot"}],changed:false});
  await expect(pcmJournal.freeze(cursor)).rejects.toThrow();expect((await pcmJournal.load(owner,cursor.endpoint,cursor.zone))?.request).toBeUndefined();
});
