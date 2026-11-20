// @vitest-environment jsdom
import {act} from 'react';
import {createRoot,type Root} from 'react-dom/client';
import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import {ApiError,RMusicClient,type Track} from '@r-music/sdk';
import {PlaylistImport} from './PlaylistExchange';
import {StatsView} from './Stats';
import {CoreStatisticsSettings} from './CoreStatisticsSettings';
import {PluginConfiguration,CoreExtensions} from './CoreExtensions';
import {MusicIdentityPanel} from './MusicIdentity';
let host:HTMLDivElement,root:Root;
beforeEach(()=>{vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT',true);localStorage.clear();host=document.createElement('div');document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.restoreAllMocks();vi.unstubAllGlobals();});
async function click(text:string){const button=[...document.querySelectorAll('button')].find(b=>b.textContent===text);expect(button, text).toBeTruthy();await act(async()=>button!.click());}
async function change(field:HTMLInputElement|HTMLSelectElement,value:string){await act(async()=>{Object.getOwnPropertyDescriptor(field instanceof HTMLInputElement?HTMLInputElement.prototype:HTMLSelectElement.prototype,'value')!.set!.call(field,value);field.dispatchEvent(new Event(field instanceof HTMLSelectElement?'change':'input',{bubbles:true}));});}
const stats={period_days:1,start_day:'2026-10-22',end_day:'2026-10-22',timezone:'Asia/Shanghai',legacy_utc_seconds:7,summary:{listened_seconds:60,play_count:1,unique_tracks:1,skip_count:0},days:[],tracks:[],artists:[]};
it('统计跟随核心时区，忽略旧的本机偏好并隐藏时区选择，周期切换取消旧请求',async()=>{
 localStorage.setItem('rmusic.stats.timezone','America/New_York');const api=new RMusicClient('http://stats-core-zone');const request=vi.spyOn(api,'request').mockResolvedValue(stats);
 await act(async()=>root.render(<StatsView api={api} revision={0} play={async()=>{}} onError={()=>{}}/>));expect(request.mock.calls[0][0]).toBe('/stats?period_days=1');
 expect(host.querySelector('select[aria-label="统计时区"]')).toBeNull();expect(host.textContent).not.toContain('Asia/Shanghai');expect(host.textContent).not.toContain('统计时区');expect(host.textContent).toContain('旧记录按原有日期保留');
 const signal=request.mock.calls[0][1]!.signal!;await click('7天');expect(signal.aborted).toBe(true);expect(request.mock.calls.at(-1)![0]).toBe('/stats?period_days=7');
});
it('核心统计设置默认上海，可保存为有效的其他时区',async()=>{
 const api=new RMusicClient('http://core-statistics-settings'),notify=vi.fn();const request=vi.spyOn(api,'request').mockResolvedValue({statistics_timezone:'Asia/Shanghai'});
 await act(async()=>root.render(<CoreStatisticsSettings api={api} notify={notify} onError={()=>{}}/>));expect(host.querySelector('select')!.value).toBe('Asia/Shanghai');
 await change(host.querySelector('select')!,'America/New_York');await click('保存统计设置');
 const saved=request.mock.calls.find(([,options])=>options?.method==='PATCH')!;expect(JSON.parse(saved[1]!.body as string)).toEqual({statistics_timezone:'America/New_York'});expect(notify).toHaveBeenCalled();
});
it('导入只自动选择准确身份，候选需手动确认，失败可保留选择重试',async()=>{
 const api=new RMusicClient('http://playlist-qa');const saved=vi.fn();let fail=true;
 const track={id:'exact',title:'准确歌曲',artist:'作者',album:'发行',duration:30,source:'local',available:true};
 const request=vi.spyOn(api,'request').mockImplementation(async(path)=>{if(path.endsWith('preview'))return {total:3,items:[{entry:{title:'准确歌曲'},status:'matched',matched:track,candidates:[]},{entry:{title:'同名歌曲'},status:'review',matched:null,candidates:[{...track,id:'candidate'}]},{entry:{title:'缺失歌曲'},status:'missing',matched:null,candidates:[]}]};if(fail)throw new Error('核心存储已满');return {id:'created'};});
 await act(async()=>root.render(<PlaylistImport api={api} close={()=>{}} saved={saved}/>));const field=host.querySelector<HTMLInputElement>('input[type=file]')!;const file=new File(['synthetic'],'songs.csv');Object.defineProperty(file,'text',{value:async()=> 'title,artist\n准确歌曲,作者'});Object.defineProperty(field,'files',{value:[file]});await act(async()=>field.dispatchEvent(new Event('change',{bubbles:true})));await click('预览与匹配');expect(host.textContent).toContain('1 / 3 首已选择');
 await change(host.querySelector('select[aria-label="确认第 2 首"]')!,'candidate');await click('创建播放列表（2 首）');expect(host.textContent).toContain('核心存储已满');expect(saved).not.toHaveBeenCalled();fail=false;await click('创建播放列表（2 首）');expect(saved).toHaveBeenCalledOnce();expect(JSON.parse(request.mock.calls.at(-1)![1]!.body as string).track_ids).toEqual(['exact','candidate']);const calls=request.mock.calls.filter(([path])=>path==='/playlists/import');expect(JSON.parse(calls[0][1]!.body as string).request_id).toBe(JSON.parse(calls[1][1]!.body as string).request_id);
});
it('插件配置冲突必须重新读取修订号，保留草稿后再提交',async()=>{
 const api=new RMusicClient('http://plugin-qa');const config=vi.spyOn(api,'pluginConfiguration').mockResolvedValue({schema:{type:'object',properties:{label:{type:'string'}}},values:{label:'old'},revision:'1'});const write=vi.spyOn(api,'configurePlugin').mockRejectedValueOnce(new ApiError(409,'plugin_configuration_conflict','配置已改变')).mockResolvedValue({saved:true,revision:'3'});
 await act(async()=>root.render(<PluginConfiguration api={api} id="report" close={()=>{}}/>));await change(host.querySelector('input')!,'draft');await click('保存配置');expect((host.querySelector('button.primary') as HTMLButtonElement).disabled).toBe(true);config.mockResolvedValue({schema:{type:'object',properties:{label:{type:'string'}}},values:{label:'other'},revision:'2'});await click('读取最新配置（保留填写内容）');expect(host.querySelector('input')!.value).toBe('draft');await click('保存配置');expect(write.mock.calls.at(-1)![1]).toEqual({expected_revision:'2',values:{label:'draft'}});
});
it('扩展读取失败可以重试，结果按字段显示而不是只显示 JSON',async()=>{
 const api=new RMusicClient('http://extension-qa');vi.spyOn(api,'extensions').mockRejectedValueOnce(new Error('核心离线')).mockResolvedValue({items:[{id:'p',name:'统计插件',version:'1',extension_api:'1',enabled:true,extensions:[{method:'summary',title:'曲库概览',kind:'statistics',scope:'read',effect:'none',input:{type:'object',properties:{}},output:{type:'object',properties:{count:{type:'integer',description:'可用歌曲'}}}}]}]});vi.spyOn(api,'invokeExtension').mockResolvedValue({plugin_id:'p',method:'summary',version:1,effect:'none',host_actions_applied:false,output:{count:100000}});
 await act(async()=>root.render(<CoreExtensions api={api}/>));await click('重试');await click('打开曲库概览');await click('运行扩展');expect(document.querySelector('.extension-output')?.textContent).toContain('可用歌曲100,000');
});
it('身份修订展示前后差异，并载入草稿而不直接覆盖当前关系',async()=>{
 const api=new RMusicClient('http://identity-qa');const entity={kind:'work' as const,namespace:'rmusic-manual',external_id:'local',title:'原作品',role:'work',id:'e',basis:'manual'};const before={track_id:'t',revision:0,automatic_merge:false as const,entities:[],relations:[]},after={...before,revision:1,entities:[entity]};vi.spyOn(api,'trackIdentity').mockResolvedValue({...after,revision:3});vi.spyOn(api,'fingerprint').mockResolvedValue({available:false,automatic_merge:false,reason:'尚未分析'});vi.spyOn(api,'identityHistory').mockResolvedValue({total:1,items:[{revision:1,created_at:1,before,after}]});const save=vi.spyOn(api,'editTrackIdentity');
 await act(async()=>root.render(<MusicIdentityPanel api={api} track={{id:'t',title:'测试'} as Track} play={async()=>{}} enqueue={async()=>{}}/>));await click('修订记录');expect(host.textContent).toContain('确认 · 作品：原作品');await click('预览恢复此修订');expect(save).not.toHaveBeenCalled();expect(host.querySelector<HTMLInputElement>('input[aria-label="名称 1"]')!.value).toBe('原作品');
});
it('全库身份索引由用户明确启动，显示后台管理入口而不阻塞当前编辑',async()=>{
 const api=new RMusicClient('http://identity-index-qa');vi.spyOn(api,'trackIdentity').mockResolvedValue({track_id:'t',revision:0,automatic_merge:false,relations:[],entities:[]});vi.spyOn(api,'fingerprint').mockResolvedValue({available:false,automatic_merge:false,reason:'尚未分析'});const index=vi.spyOn(api,'indexMusicIdentity').mockResolvedValue({id:'job',status:'pending'});
 await act(async()=>root.render(<MusicIdentityPanel api={api} track={{id:'t',title:'测试'} as Track} play={async()=>{}} enqueue={async()=>{}}/>));expect(index).not.toHaveBeenCalled();await click('更新全曲库身份索引');expect(index).toHaveBeenCalledOnce();expect(host.textContent).toContain('可在设置的后台任务中查看、暂停或取消');
});
it('详细聆听记录读取失败可以就地重试，不需要关闭页面',async()=>{
 const api=new RMusicClient('http://history-retry-qa');let failed=true;const request=vi.spyOn(api,'request').mockImplementation(async path=>{if(path.startsWith('/stats/history')){if(failed)throw new TypeError('Failed to fetch');return {items:[],next_cursor:null};}return stats;});
 await act(async()=>root.render(<StatsView api={api} revision={0} play={async()=>{}} onError={()=>{}}/>));await click('聆听历史');expect(host.textContent).toContain('无法连接核心');failed=false;await click('重试记录');expect(host.textContent).toContain('还没有记录');expect(request.mock.calls.filter(([path])=>path.startsWith('/stats/history'))).toHaveLength(2);
});
