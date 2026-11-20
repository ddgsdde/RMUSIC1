// @vitest-environment jsdom
import {afterEach,expect,it,vi} from 'vitest';
import {RMusicClient} from '@r-music/sdk';
import {acquireArtwork,disposeArtwork} from './artworkCache';
const native=vi.hoisted(()=>({enabled:false,cover:vi.fn()}));
vi.mock('./native',()=>({get isNative(){return native.enabled;},nativeCover:native.cover}));
afterEach(()=>{native.enabled=false;native.cover.mockReset();vi.useRealTimers();vi.unstubAllGlobals();});
const path='/api/v1/artwork/'+'a'.repeat(64);
it('多页面共享一次正文，返回列表和缩放只复用，退出账号后释放',async()=>{
 const api=new RMusicClient('http://core','token');const fetch=vi.fn(async()=>new Response(new Blob(['pixels'],{type:'image/jpeg'}),{headers:{ETag:'"v1"'}}));vi.stubGlobal('fetch',fetch);
 URL.createObjectURL=vi.fn(()=> 'blob:shared');URL.revokeObjectURL=vi.fn();
 const leases=await Promise.all(Array.from({length:10},()=>acquireArtwork(api,path,new AbortController().signal)));
 expect(fetch).toHaveBeenCalledTimes(1);expect(new Set(leases.map(l=>l.src)).size).toBe(1);leases.forEach(l=>l.release());
 const again=await acquireArtwork(api,path,new AbortController().signal);expect(fetch).toHaveBeenCalledTimes(1);again.release();expect(URL.revokeObjectURL).not.toHaveBeenCalled();disposeArtwork(api);expect(URL.revokeObjectURL).toHaveBeenCalledTimes(1);
});
it('重验只传ETag，304保留原Blob，不同账号不共用正文',async()=>{
 const api=new RMusicClient('http://core','A');URL.createObjectURL=vi.fn(()=> 'blob:image');URL.revokeObjectURL=vi.fn();
 const fetch=vi.fn().mockResolvedValueOnce(new Response(new Blob(['pixels']),{headers:{ETag:'"v1"'}})).mockResolvedValueOnce(new Response(null,{status:304})).mockResolvedValueOnce(new Response(new Blob(['B'])));vi.stubGlobal('fetch',fetch);
 const first=await acquireArtwork(api,path,new AbortController().signal);first.release();api.artworkRevision++;
 const second=await acquireArtwork(api,path,new AbortController().signal);expect(second.src).toBe(first.src);expect(fetch.mock.calls[1][1].headers['If-None-Match']).toBe('"v1"');second.release();expect(URL.createObjectURL).toHaveBeenCalledTimes(1);
 api.token='B';const third=await acquireArtwork(api,path,new AbortController().signal);expect(fetch.mock.calls[2][1].headers.Authorization).toBe('Bearer B');expect(fetch.mock.calls[2][1].headers['If-None-Match']).toBeUndefined();third.release();disposeArtwork(api);
});
it('一个消费者取消不会取消其他消费者，最后一个离开才中止',async()=>{
 const api=new RMusicClient('http://core','token');const fetch=vi.fn((_url,_options)=>new Promise((_resolve,reject)=>_options.signal.addEventListener('abort',()=>reject(new DOMException('Cancelled','AbortError')))));vi.stubGlobal('fetch',fetch);
 const a=new AbortController(),b=new AbortController();const one=acquireArtwork(api,path,a.signal).catch(()=>null),two=acquireArtwork(api,path,b.signal).catch(()=>null);
 await Promise.resolve();await Promise.resolve();a.abort();expect(fetch.mock.calls[0][1].signal.aborted).toBe(false);b.abort();await Promise.all([one,two]);expect(fetch.mock.calls[0][1].signal.aborted).toBe(true);disposeArtwork(api);
});
it('损坏图片可有界重新取回',async()=>{
 const api=new RMusicClient('http://core','token');vi.stubGlobal('fetch',vi.fn(async()=>new Response(new Blob(['pixels']),{headers:{ETag:'"v1"'}})));URL.createObjectURL=vi.fn(()=> 'blob:image');URL.revokeObjectURL=vi.fn();
 const lease=await acquireArtwork(api,path,new AbortController().signal);lease.invalidate();const retry=await acquireArtwork(api,path,new AbortController().signal,true);expect(fetch).toHaveBeenCalledTimes(2);expect(vi.mocked(fetch).mock.calls[1][1]?.cache).toBe('reload');retry.release();disposeArtwork(api);
});

it('取消重验会释放已移出缓存的旧Blob，不能留下孤立图片',async()=>{
 const api=new RMusicClient('http://core','token');URL.createObjectURL=vi.fn(()=> 'blob:old');URL.revokeObjectURL=vi.fn();
 const fetch=vi.fn().mockResolvedValueOnce(new Response(new Blob(['pixels']))).mockImplementationOnce((_url,options)=>new Promise((_resolve,reject)=>options.signal.addEventListener('abort',()=>reject(new DOMException('Cancelled','AbortError')))));vi.stubGlobal('fetch',fetch);
 const first=await acquireArtwork(api,path,new AbortController().signal);first.release();api.artworkRevision++;
 const stop=new AbortController();const next=acquireArtwork(api,path,stop.signal).catch(()=>null);await Promise.resolve();await Promise.resolve();stop.abort();await next;
 expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:old');disposeArtwork(api);
});

it('安卓切页后接回同一封面请求，短暂的链路错误只重试一次',async()=>{
 native.enabled=true;vi.useFakeTimers();
 const api=new RMusicClient('http://core','token');
 const image='data:image/jpeg;base64,pixels';
 native.cover.mockRejectedValueOnce(Object.assign(new Error('relay unavailable'),{status:503})).mockResolvedValueOnce(image);
 const first=new AbortController(), second=new AbortController();
 const a=acquireArtwork(api,path,first.signal).catch(()=>null);
 await vi.advanceTimersByTimeAsync(0);
 first.abort();
 await vi.advanceTimersByTimeAsync(100);
 const b=acquireArtwork(api,path,second.signal);
 expect(native.cover).toHaveBeenCalledTimes(1);
 expect(native.cover.mock.calls[0][2].aborted).toBe(false);
 await vi.advanceTimersByTimeAsync(500);
 expect(await a).toBeNull();
 const lease=await b;
 expect(lease.src).toBe(image);
 expect(native.cover).toHaveBeenCalledTimes(2);
 lease.release();disposeArtwork(api);
});
