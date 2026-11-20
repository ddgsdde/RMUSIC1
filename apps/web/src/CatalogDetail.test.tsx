// @vitest-environment jsdom
import {act} from 'react';
import {createRoot,type Root} from 'react-dom/client';
import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import {RMusicClient,type Track} from '@r-music/sdk';
import {CatalogDetail} from './CatalogDetail';
import {MetadataPriority} from './MetadataPriority';
let host:HTMLDivElement,root:Root;
beforeEach(()=>{vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT',true);host=document.createElement('div');document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.restoreAllMocks();vi.unstubAllGlobals();});
const track={id:'qqmusic:song',title:'Song',artist:'Crush / IU',album:'Album',source:'qqmusic',provider_data:{artist_mids:['crush','iu']},duration:100,format:'unknown',sample_rate:0,bit_depth:0,channels:2,bitrate:0,available:true,recording_id:'rec'} as Track;
const props=(api:RMusicClient)=>({api,id:'iu',kind:'artist' as const,play:vi.fn(async()=>{}),enqueue:vi.fn(async()=>{}),details:vi.fn()});
it('首屏优先曲目，简介和来源从资料入口展开',async()=>{
 const api=new RMusicClient('http://core.test','token');let signal:AbortSignal|undefined;
 vi.spyOn(api,'request').mockImplementation(async(_p,o)=>{signal=o?.signal as AbortSignal;return {name:'IU',profile:{description:'Verified biography',genre:'Pop'},provenance:{description:'qqmusic'},anchor:track,local:{items:[track],total:1,page:1},platforms:[{source:'qqmusic',name:'QQ 音乐',sorts:['default','hot','release_new','release_old'],items:[{...track,id:'qqmusic:other',title:'Cloud song'}],total:64,page:1,next_page:2}]};});
 await act(async()=>root.render(<CatalogDetail {...props(api)}/>));expect(host.textContent).not.toContain('Verified biography');expect(host.textContent).toContain('Cloud song');expect(host.textContent).toContain('本地曲库');expect(host.textContent).not.toContain('Core 曲库');expect([...host.querySelectorAll('h2')][0].textContent).toBe('QQ 音乐');expect(host.textContent).not.toContain('完整专辑');
 await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="查看资料与来源"]')!.click());expect(document.body.textContent).toContain('Verified biography');await act(async()=>document.querySelector<HTMLButtonElement>('[aria-label="关闭"]')!.click());
 const next=[...host.querySelectorAll('button')].find(b=>b.textContent==='下一页')!;await act(async()=>next.click());expect(api.request).toHaveBeenLastCalledWith(expect.stringContaining('page=2'),expect.anything());
 await act(async()=>root.unmount());expect(signal?.aborted).toBe(true);root=createRoot(host);
});
it('切换实体取消旧请求且迟到结果不覆盖新歌手',async()=>{
 const api=new RMusicClient('http://core.test','token');let finish!:(v:unknown)=>void,first:AbortSignal|undefined;
 vi.spyOn(api,'request').mockImplementation((path,o)=>path.includes('/iu?')?new Promise(resolve=>{finish=resolve;first=o?.signal as AbortSignal;}):Promise.resolve({name:'Crush',profile:{},provenance:{},anchor:track,local:{items:[],total:0,page:1},platforms:[]}));
 await act(async()=>root.render(<CatalogDetail key="iu" {...props(api)}/>));await act(async()=>root.render(<CatalogDetail key="crush" {...props(api)} id="crush"/>));expect(first?.aborted).toBe(true);
 await act(async()=>finish({name:'Stale IU',profile:{},local:{items:[],total:0},platforms:[]}));expect(host.textContent).toContain('Crush');expect(host.textContent).not.toContain('Stale IU');
});
it('资料优先级调整后保存显式顺序',async()=>{
 const api=new RMusicClient('http://core.test','token');vi.spyOn(api,'request').mockResolvedValue({metadata_provider_priority:['qqmusic','other'],metadata_providers:[{id:'qqmusic',name:'QQ 音乐'},{id:'other',name:'Other'}]});
 await act(async()=>root.render(<MetadataPriority api={api}/>));await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="提高 other 优先级"]')!.click());await act(async()=>[...host.querySelectorAll('button')].find(b=>b.textContent==='保存优先级')!.click());
 expect(api.request).toHaveBeenLastCalledWith('/settings',{method:'PATCH',body:JSON.stringify({metadata_provider_priority:['other','qqmusic']})});expect(host.textContent).toContain('已保存');
});

it('曲目排序靠近列表，编辑收进资料，切换本地范围不拉平台目录',async()=>{
 const api=new RMusicClient('http://core.test','token');const snapshot={name:'IU',profile:{},provenance:{},anchor:track,local:{items:[track],total:1,page:1},platforms:[{source:'qqmusic',name:'QQ 音乐',sorts:['default','hot','release_new','release_old'],items:[track],total:1,page:1}]};
 const request=vi.spyOn(api,'request').mockResolvedValue(snapshot);
 await act(async()=>root.render(<CatalogDetail {...props(api)} canEdit/>));
 expect(host.textContent).not.toContain('平台目录');expect(host.querySelectorAll('.entity-track')).toHaveLength(1);
 const toolbar=host.querySelector('.entity-actions')!;expect(toolbar.textContent).not.toContain('编辑资料');const select=toolbar.querySelector('select')!;select.value='release_new';await act(async()=>select.dispatchEvent(new Event('change',{bubbles:true})));
 expect(request).toHaveBeenLastCalledWith(expect.stringContaining('platform_sort=release_new'),expect.anything());
 await act(async()=>[...host.querySelectorAll<HTMLButtonElement>('nav button')].find(b=>b.textContent?.startsWith('本地曲库'))!.click());
 expect(request).toHaveBeenLastCalledWith(expect.stringContaining('remote=false'),expect.anything());
 await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="查看资料与来源"]')!.click());expect(document.body.textContent).toContain('编辑资料');await act(async()=>[...document.querySelectorAll<HTMLButtonElement>('button')].find(b=>b.textContent==='更新资料')!.click());expect(request).toHaveBeenLastCalledWith(expect.stringContaining('remote=true'),expect.anything());expect(request).toHaveBeenLastCalledWith(expect.stringContaining('refresh=true'),expect.anything());
});
