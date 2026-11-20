// @vitest-environment jsdom
import {act} from 'react';
import {createRoot,type Root} from 'react-dom/client';
import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import {RMusicClient,type Track} from '@r-music/sdk';
import {PlaybackPolicyEditor} from './PlaybackPolicy';
import {MusicVersions} from './Versions';
import {CatalogSources} from '../CatalogSources';
let host:HTMLDivElement,root:Root;
beforeEach(()=>{vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT',true);host=document.createElement('div');document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.useRealTimers();vi.restoreAllMocks();vi.unstubAllGlobals();});
async function click(text:string){const b=[...host.querySelectorAll('button')].find(b=>b.textContent===text);expect(b,text).toBeTruthy();await act(async()=>b!.click());}
const track={id:'t',title:'Exact',artist:'Artist',album:'Album',source:'fixture',available:true} as Track;
const policy={preferred_track_id:null,apply_to_new_playback:false,automatic_fallback:false,same_release:true,lossless_only:true};
it('播放首选必须保存后生效，自动回退默认关，清除只改变策略',async()=>{
 const api=new RMusicClient('http://fixture'),request=vi.spyOn(api,'request').mockImplementation(async(_,init)=>init?.method==='PUT'?{revision:2,policy:JSON.parse(init.body as string).policy}:{revision:1,policy,identity_resolved:true});
 await act(async()=>root.render(<PlaybackPolicyEditor api={api} id="t" preferred={track}/>));expect(request.mock.calls).toHaveLength(1);
 expect((host.querySelectorAll('input')[1] as HTMLInputElement).checked).toBe(false);
 await click('保存播放策略');expect(JSON.parse(request.mock.calls.at(-1)![1]!.body as string)).toEqual({expected_revision:1,policy:{...policy,preferred_track_id:'t'}});
 await click('清除播放首选');await click('保存播放策略');expect(JSON.parse(request.mock.calls.at(-1)![1]!.body as string).policy.preferred_track_id).toBeNull();
 await act(async()=>root.render(<PlaybackPolicyEditor api={api} id="next"/>));expect(request.mock.calls[0][1]?.signal?.aborted).toBe(true);expect(request.mock.calls.at(-1)![0]).toContain('/next/');
});
it('版本与其他录音独立翻页，精确播放不使用推荐偏好',async()=>{
 const api=new RMusicClient('http://fixture'),edition={revision:0,kind:'original',label:'',notes:''};
 const request=vi.spyOn(api,'request').mockImplementation(async(path)=>path.endsWith('/policy')?{revision:0,policy,identity_resolved:true}:{track_id:'t',identity_resolved:true,items:[{track,edition}],preferred:{track_id:null,revision:0},total:11031,offset:0,edition,other_recordings:[{...track,id:'other'}],other_page:{offset:0,limit:20,total:35,next_offset:20}});
 const play=vi.fn(async()=>{}),exact=vi.fn(async()=>{});
 await act(async()=>root.render(<MusicVersions api={api} track={track} play={play} playExact={exact} enqueue={play} initialOpen/>));
 await click('下一页录音');expect(request.mock.calls.some(([p])=>p.includes('offset=0&other_offset=20'))).toBe(true);
 await click('下一页');expect(request.mock.calls.some(([p])=>p.includes('offset=20&other_offset=20'))).toBe(true);
 await click('播放此版本');expect(exact).toHaveBeenCalledWith(track);expect(play).not.toHaveBeenCalled();
});
it('资料关联先预览后确认，换页面取消请求并清空旧预览',async()=>{
 vi.useFakeTimers();const api=new RMusicClient('http://fixture'),changed=vi.fn();
 const request=vi.spyOn(api,'request').mockImplementation(async(_,init)=>init?.method==='PUT'?{revision:4,preview_only:true}:{revision:4,items:[],candidates:[{id:'other',name:'Exact Alias',source:'two'}]});
 await act(async()=>root.render(<CatalogSources api={api} kind="artist" id="first" canEdit changed={changed}/>));
 await act(async()=>{const d=host.querySelector('details')!;d.open=true;d.dispatchEvent(new Event('toggle'));});await act(async()=>{await vi.advanceTimersByTimeAsync(251);});
 await click('预览关联');expect(changed).not.toHaveBeenCalled();expect(JSON.parse(request.mock.calls.at(-1)![1]!.body as string)).toEqual({other_id:'other',linked:true,expected_revision:4,preview_only:true});
 expect(host.textContent).toContain('确认关联');const signal=request.mock.calls.at(-1)![1]!.signal;
 await act(async()=>root.render(<CatalogSources api={api} kind="artist" id="second" canEdit changed={changed}/>));expect(signal?.aborted).toBe(true);expect([...host.querySelectorAll('button')].some(b=>b.textContent==='确认关联')).toBe(false);
});
