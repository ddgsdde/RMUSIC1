// @vitest-environment jsdom
import {act} from 'react';
import {createRoot, type Root} from 'react-dom/client';
import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import {RMusicClient,type Track} from '@r-music/sdk';
import {CatalogLink,CatalogNavigation,artistNames} from './CatalogLinks';
import {SearchPage} from './Search';
let host:HTMLDivElement,root:Root;
beforeEach(()=>{(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT=true;host=document.createElement('div');document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.restoreAllMocks();vi.useRealTimers();});
const track={id:'qqmusic:song',title:'Song',artist:'Crush / IU',album:'Album',source:'qqmusic',provider_data:{artist_mids:['crush','iu']},duration:100,format:"unknown",sample_rate:0,bit_depth:0,channels:2,bitrate:0,available:true,recording_id:"recording"} as Track;
it('只有独立身份依据才拆分合作歌手，保留乐队名与缓存身份',()=>{
 expect(artistNames(track)).toEqual(['Crush','IU']);expect(artistNames({...track,source:'local',provider_data:{}})).toEqual(['Crush / IU']);expect(artistNames({...track,artist:'AC/DC',provider_data:{}})).toEqual(['AC/DC']);
 expect(artistNames({...track,source:'local',provider_data:{...track.provider_data,cached_from:{source:'qqmusic'}}})).toEqual(['Crush','IU']);
});
it('点击参与歌手打开独立身份且不触发父级播放',async()=>{
 const api=new RMusicClient('http://core.test','token'),navigate=vi.fn(),play=vi.fn();vi.spyOn(api,'request').mockResolvedValue({resolved:true,artists:[{id:'a',name:'Crush'},{id:'b',name:'IU'}]});
 await act(async()=>root.render(<CatalogNavigation api={api} location="library" navigate={navigate} onError={vi.fn()}><button onClick={play}><CatalogLink track={track}/></button></CatalogNavigation>));
 await act(async()=>host.querySelectorAll<HTMLElement>('[role=link]')[1].click());expect(navigate).toHaveBeenCalledWith('artist','b','IU');expect(play).not.toHaveBeenCalled();
});
it('查询摘要先解析独立名字，直接点击哪个歌手就打开哪个歌手',async()=>{
 const api=new RMusicClient('http://core.test','token'),navigate=vi.fn();vi.spyOn(api,'request').mockResolvedValue({resolved:true,artists:[{id:'a',name:'Crush'},{id:'b',name:'IU'}]});
 await act(async()=>root.render(<CatalogNavigation api={api} location="stats" navigate={navigate} onError={vi.fn()}><CatalogLink track={{id:track.id,artist:track.artist,album:''}}/></CatalogNavigation>));
 expect(host.querySelectorAll('[role=link]').length).toBe(2);await act(async()=>host.querySelectorAll<HTMLElement>('[role=link]')[1].click());expect(navigate).toHaveBeenCalledWith('artist','b','IU');expect(document.body.textContent).not.toContain('选择对应资料');
});
it('分类切换复用结果且离开页面取消搜索',async()=>{
 vi.useFakeTimers();const api=new RMusicClient('http://core.test','token');const signals:AbortSignal[]=[];
 const request=vi.spyOn(api,'request').mockImplementation(async(_path,options)=>{signals.push(options?.signal as AbortSignal);return {items:[track],artists:[{id:'iu',name:'IU',source:'qqmusic',track}],albums:[]};});
 const props={api,query:'IU',play:vi.fn(),browse:vi.fn(),details:vi.fn()};
 await act(async()=>root.render(<SearchPage {...props}/>));await act(async()=>vi.advanceTimersByTimeAsync(301));
 expect(host.textContent).toContain('Song');await act(async()=>Array.from(host.querySelectorAll('nav button')).find(x=>x.textContent==='艺术家')!.dispatchEvent(new MouseEvent('click',{bubbles:true})));
 expect(request.mock.calls.map(([path])=>path)).toEqual(['/library/search?q=IU','/search?q=IU']);
 expect(host.textContent).not.toContain('Song');
 await act(async()=>root.unmount());expect(signals.map(signal=>signal.aborted)).toEqual([true,true]);root=createRoot(host);
});
it('快速输入丢弃前一个请求的迟到结果',async()=>{
 vi.useFakeTimers();const api=new RMusicClient('http://core.test','token');let finish!:(v:unknown)=>void;
 vi.spyOn(api,'request').mockImplementation((path)=>path.includes('first')?new Promise(resolve=>{finish=resolve;}):Promise.resolve({items:[{...track,title:'Current'}],artists:[],albums:[]}));
 const props={api,play:vi.fn(),browse:vi.fn(),details:vi.fn()};
 await act(async()=>root.render(<SearchPage {...props} query="first"/>));await act(async()=>vi.advanceTimersByTimeAsync(301));
 await act(async()=>root.render(<SearchPage {...props} query="second"/>));await act(async()=>vi.advanceTimersByTimeAsync(301));
 await act(async()=>finish({items:[{...track,title:'Stale'}],artists:[],albums:[]}));expect(host.textContent).toContain('Current');expect(host.textContent).not.toContain('Stale');
});
