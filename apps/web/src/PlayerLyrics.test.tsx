// @vitest-environment jsdom
import {act} from 'react';
import {createRoot,type Root} from 'react-dom/client';
import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import type {LyricDocument} from '@r-music/sdk';
import {Lyrics} from './PlayerLyrics';
let host:HTMLDivElement,root:Root;
const scrollTo=vi.fn();
const doc:LyricDocument={format:'lrc',text:'[00:01]First\n[00:04]Second\n[00:08]Third',translation:'[00:01]第一句\n[00:04]第二句',offset_ms:200,source:'test'};
const seek=vi.fn();
function render(position=2,document=doc){return act(async()=>root.render(<Lyrics document={document} position={position} offset={.3} font={30} mode="bilingual" seek={seek}/>));}
beforeEach(()=>{vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT',true);vi.stubGlobal('ResizeObserver',class{observe(){}disconnect(){}});vi.stubGlobal('matchMedia',()=>({matches:false}));Object.defineProperty(HTMLElement.prototype,'scrollTo',{configurable:true,value:scrollTo});host=document.createElement('div');document.body.append(host);root=createRoot(host);scrollTo.mockClear();seek.mockClear();});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.restoreAllMocks();vi.unstubAllGlobals();});
it('手动浏览后换句不拉回，点击跟随回到当前句',async()=>{await render();await act(async()=>host.querySelector('.lyrics-scroll')!.dispatchEvent(new WheelEvent('wheel',{bubbles:true,deltaY:200})));scrollTo.mockClear();await render(5);expect(scrollTo).not.toHaveBeenCalled();expect(host.textContent).toContain('回到当前歌词');await act(async()=>host.querySelector<HTMLButtonElement>('.lyrics-resume')!.click());expect(scrollTo).toHaveBeenCalled();expect(host.querySelector('.lyrics-resume')).toBeNull();});
it('点句按文档和用户偏移定位并恢复跟随，双语保持对应',async()=>{await render();expect(host.querySelector('.lyric-line')?.textContent).toBe('First第一句');await act(async()=>host.querySelectorAll<HTMLButtonElement>('.lyric-line')[1].click());expect(seek).toHaveBeenCalledWith(4.5);await render(5);expect(host.querySelector('[aria-current=true]')?.textContent).toContain('Second');});
it('纯文本可读且不编造定位时刻，新曲恢复跟随',async()=>{await render(2,{...doc,text:'没有时间轴\n第二行',translation:undefined,format:'plain'});expect([...host.querySelectorAll<HTMLButtonElement>('.lyric-line')].every(button=>button.disabled)).toBe(true);await act(async()=>host.querySelector('.lyrics-scroll')!.dispatchEvent(new WheelEvent('wheel',{bubbles:true})));expect(host.querySelector('.lyrics-resume')).toBeNull();await render();expect(host.querySelector('.lyrics-reader')?.getAttribute('data-following')).toBe('true');});
it('读取中与加载失败不冒充无歌词，失败可明确重试',async()=>{const retry=vi.fn();await act(async()=>root.render(<Lyrics document={null} position={0} offset={0} font={30} mode="original" seek={seek} loading/>));expect(host.textContent).toContain('正在读取歌词');expect(host.textContent).not.toContain('没有歌词');await act(async()=>root.render(<Lyrics document={null} position={0} offset={0} font={30} mode="original" seek={seek} failed retry={retry}/>));await act(async()=>host.querySelector<HTMLButtonElement>('.lyrics-state button')!.click());expect(retry).toHaveBeenCalledOnce();});
it('真实持续时长推进文字，停顿期间不延伸到下一个字，译文不扫亮',async()=>{
 const document:LyricDocument={...doc,offset_ms:0,text:'[00:01.00]甲乙',translation:'[00:01.00]Translation',word_timing:[{time:1,text:'甲乙',words:[{time:1.2,duration:.2,text:'甲'},{time:3,duration:.6,text:'乙'}]}]};
 await render(1.6,document);expect(host.querySelector<HTMLElement>('.timed-word')!.style.getPropertyValue('--word-progress')).toMatch(/^50/);expect(host.querySelector('small')!.querySelector('.timed-word')).toBeNull();
 await render(2.8,document);expect(host.querySelector<HTMLElement>('.timed-word')!.style.getPropertyValue('--word-progress')).toBe('100%');expect(host.querySelectorAll<HTMLElement>('.timed-word')[1].style.getPropertyValue('--word-progress')).toBe('0%');
});

it('相同歌词资料刷新不打断手动阅读',async()=>{await render();await act(async()=>host.querySelector('.lyrics-scroll')!.dispatchEvent(new WheelEvent('wheel',{bubbles:true})));expect(host.textContent).toContain('回到当前歌词');await render(3,{...doc});expect(host.textContent).toContain('回到当前歌词');});

it('手动阅读时调整字号保留阅读位置，恢复跟随才重新居中',async()=>{
 await render();await act(async()=>host.querySelector('.lyrics-scroll')!.dispatchEvent(new WheelEvent('wheel',{bubbles:true})));scrollTo.mockClear();
 await act(async()=>root.render(<Lyrics document={doc} position={2} offset={.3} font={36} mode="bilingual" seek={seek}/>));
 expect(scrollTo).not.toHaveBeenCalled();await act(async()=>host.querySelector<HTMLButtonElement>('.lyrics-resume')!.click());expect(scrollTo).toHaveBeenCalled();
});

it('安卓从封面下的当前句进入大歌词页，返回保留播放位置与偏好',async()=>{
 const {Player}=await import('./Player');
 const {RMusicClient}=await import('@r-music/sdk');
 vi.stubGlobal('matchMedia',(query:string)=>({matches:query.includes('max-width')}));
 const api=new RMusicClient('https://android-cover-lyrics.test','session');
 const request=vi.spyOn(api,'request').mockResolvedValue(doc);
 const track={id:'song',title:'歌曲',artist:'歌手',album:'专辑',duration:60,format:'flac',source:'local',available:true,bit_depth:16,sample_rate:44100,channels:2,bitrate:0,recording_id:'song'};
 const zone={id:'travel-phone',endpoint_id:'phone',name:'手机',playback_id:'play-1',status:'paused',queue:[track],track,queue_index:0,position:2,updated_at:10000,volume:1,muted:false,repeat:'off',shuffle:false,dsp:{enabled:false,preamp_db:0,balance:0,normalize:false,target_lufs:-14,bands:[]}} as const;
 const close=vi.fn(),savePreferences=vi.fn(async()=>{});
 const renderPlayer=(position:number)=>act(async()=>root.render(
  <Player api={api} zone={{...zone,queue:[track],dsp:{...zone.dsp,bands:[]}}} position={position}
   androidCoverLyrics playing={false} preferences={{player_view:'lyrics'} as React.ComponentProps<typeof Player>['preferences']}
   savePreferences={savePreferences} seek={seek} command={async()=>{}} analyser={{current:null}} adaptive={0}
   nativeState={{}} localOutput sourceInfo={null} sourceError="" contextRate={0} onClose={close} onError={vi.fn()}/>
 ));
 await renderPlayer(2);
 expect(host.querySelector('[aria-label="播放视图"]')).toBeNull();
 const preview=()=>host.querySelector<HTMLButtonElement>('[aria-label="展开歌词"]')!;
 expect(preview().querySelector('.preview-current')?.textContent).toBe('First');
 expect(preview().querySelector('.lyric-preview-lines')?.childElementCount).toBe(3);
 expect(preview().querySelector('svg')).toBeNull();
 expect(host.querySelector('.lyrics-reader')).toBeNull();
 await renderPlayer(5);
 expect(preview().querySelector('.preview-current')?.textContent).toBe('Second');
 await act(async()=>preview().click());
 expect(host.querySelector('.listening-room')?.getAttribute('data-android-lyrics')).toBe('true');
 expect(host.querySelector('.lyric-line.active')?.textContent).toContain('Second');
 expect(host.querySelector('.listening-artwork')?.hasAttribute('inert')).toBe(true);
 await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="返回封面"]')!.click());
 expect(host.querySelector('.listening-room')?.getAttribute('data-android-lyrics')).toBe('false');
 expect(host.querySelector('.listening-lyrics')?.hasAttribute('inert')).toBe(true);
 expect(preview().querySelector('.preview-current')?.textContent).toBe('Second');
 expect(close).not.toHaveBeenCalled();
 expect(savePreferences).not.toHaveBeenCalled();
 expect(request).toHaveBeenCalledTimes(1);
});

it('迷你播放栏优先显示译文，有逐字时间轴才高亮，可切回原文或关闭跟随',async()=>{
 const {MiniLyricTitle}=await import('./PlayerLyrics');
 const {RMusicClient}=await import('@r-music/sdk');
 const api=new RMusicClient('https://android-mini-lyrics.test','session');
 const lyric={...doc,offset_ms:0,translation:'[00:01.00]<00:01.00>晚<00:02.00>风'};
 vi.spyOn(api,'request').mockResolvedValue(lyric);
 const track={id:'mini',title:'歌名',artist:'歌手',album:'专辑',duration:60,format:'flac',source:'local',available:true,bit_depth:16,sample_rate:44100,channels:2,bitrate:0,recording_id:'song'};
 const renderMini=(position:number,mode:'translation'|'original'='translation',follow=true)=>act(async()=>root.render(
  <MiniLyricTitle api={api} track={track} position={position} playing={false} options={{mode,follow,offset:0,remote:false}}/>
 ));
 await renderMini(1.5);
 expect(host.querySelector('.mini-lyric-title')?.textContent).toBe('晚风');
 expect([...host.querySelectorAll<HTMLElement>('.timed-word')].map(word=>word.style.getPropertyValue('--word-progress'))).toEqual(['100%','0%']);
 await renderMini(1.5,'translation',false);
 expect(host.querySelector('.mini-lyric-title')?.textContent).toBe('晚风');
 expect(host.querySelector('.timed-word')).toBeNull();
 await renderMini(1.5,'original');
 expect(host.querySelector('.mini-lyric-title')?.textContent).toBe('First');
 await renderMini(5);
 expect(host.querySelector('.mini-lyric-title')?.textContent).toBe('Second');
 await renderMini(0);
 expect(host.querySelector('.mini-lyric-title')?.textContent).toBe('歌名');
});
