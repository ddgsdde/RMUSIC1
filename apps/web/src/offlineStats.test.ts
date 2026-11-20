// @vitest-environment jsdom
import "fake-indexeddb/auto";
import {beforeEach,expect,it} from "vitest";
import {ListenSampler,listenJournal,listenOwner,rememberListenOwner,forgetListenOwner,observeListenCore,type ListenSnapshot} from "./offlineStats";
const start=Date.parse("2026-10-12T23:59:50Z"),owner={core_id:"core-a",user_id:"user-a"};
beforeEach(async()=>{localStorage.clear();await new Promise<void>((yes,no)=>{const r=indexedDB.deleteDatabase("r-music-listens-v1");r.onsuccess=()=>yes();r.onerror=()=>no(r.error);});});
function sampler(){return new ListenSampler({track_id:"a",title:"缓存音乐"},120,start);}
function total(s:ListenSnapshot){return Object.values(s.days).reduce((sum,day)=>sum+day.seconds,0);}
it("累计实际进度并跨 UTC 午夜分配，不累计暂停、定位、长时间挂起和系统时钟跳变",()=>{
 const s=sampler();s.sample(0,true,false,start,0);s.sample(20,false,false,start+20000,20000);
 expect(s.snapshot.days["2026-10-12"].seconds).toBe(10);expect(s.snapshot.days["2026-10-13"].seconds).toBe(10);
 s.sample(20,true,false,start+25000,25000);s.sample(21,true,false,start+26000,26000);
 s.sample(21,true,false,start+27000,27000);s.sample(90,true,true,start+27000,27000);s.sample(91,true,false,start+28000,28000);
 s.sample(120,true,false,start+90000,90000);s.sample(119,true,false,start+91000,91000);s.sample(120,true,false,start+92000,92000);
 s.sample(121,true,false,start+3600000,93000);expect(total(s.snapshot)).toBe(23);
});
it("持久记录按核心和账号隔离，重登不会改写原缓存归属，显式退出清除当前统计身份",()=>{
 rememberListenOwner("http://localhost",owner);expect(listenOwner("core-a")).toEqual(owner);
 observeListenCore("http://localhost","core-b");expect(listenOwner("core-b")).toBeUndefined();
 rememberListenOwner("http://localhost",{core_id:"core-b",user_id:"user-b"});forgetListenOwner("http://localhost");expect(listenOwner("core-b")).toBeUndefined();expect(listenOwner("core-a")).toEqual(owner);
});
it("迟到应答不确认新版本，确认后旧写入不复活或降低版本",async()=>{
 const s=sampler();s.sample(0,true,false,start,0);s.sample(10,true,false,start+10000,10000);const first=s.flush(true)!;await listenJournal.save(owner,first);
 s.sample(15,false,false,start+15000,15000);const final=s.finish("stop");await listenJournal.save(owner,final);
 await listenJournal.acknowledge(owner,{accepted:[{id:first.id,version:first.version}],rejected:[],changed:true});
 expect((await listenJournal.pending(owner))[0].snapshot.version).toBe(final.version);
 expect(await listenJournal.pending({...owner,user_id:"other"})).toEqual([]);
 await listenJournal.acknowledge({...owner,core_id:"other"},{accepted:[{id:final.id,version:final.version}],rejected:[],changed:true});expect(await listenJournal.pending(owner)).toHaveLength(1);
 await listenJournal.acknowledge(owner,{accepted:[{id:final.id,version:final.version}],rejected:[],changed:true});await listenJournal.save(owner,first);expect(await listenJournal.pending(owner)).toEqual([]);
});
it("隔离被拒绝的版本，其他会话继续补传，已返回的快照不受后续采样修改",async()=>{
 const s=sampler();s.sample(0,true,false,start,0);s.sample(10,true,false,start+10000,10000);const first=s.flush(true)!;await listenJournal.save(owner,first);s.sample(15,true,false,start+15000,15000);expect(total(first)).toBe(10);
 const other=sampler().finish("ended");await listenJournal.save(owner,other);
 await listenJournal.acknowledge(owner,{accepted:[],rejected:[{id:first.id,version:first.version,code:"invalid_snapshot"}],changed:false});
 expect((await listenJournal.pending(owner)).map(row=>row.id)).toEqual([other.id]);
});
it("旧核心跳过测量记录仍能补传后面的普通离线日志，不删除测量归属",async()=>{
 for(let i=0;i<33;i++){const value=sampler().finish("interrupted");value.measured={playback_id:`source-${i}`,listen_session_id:`session-${i}`,segments:[]};await listenJournal.save(owner,value);}
 const legacy=sampler().finish("ended");await listenJournal.save(owner,legacy);
 expect((await listenJournal.pending(owner,false)).map(row=>row.id)).toEqual([legacy.id]);expect(await listenJournal.pending(owner)).toHaveLength(32);
 expect(await listenJournal.pending({...owner,user_id:"another"},false)).toEqual([]);
});
it("相同记录 ID 的另一个账号不能覆盖原归属及其已保存版本",async()=>{
 const value=sampler().finish("interrupted");await listenJournal.save(owner,value);
 await listenJournal.save({...owner,user_id:"other"},{...value,version:value.version+1});
 expect((await listenJournal.pending(owner))[0].snapshot.version).toBe(value.version);expect(await listenJournal.pending({...owner,user_id:"other"})).toEqual([]);
});
