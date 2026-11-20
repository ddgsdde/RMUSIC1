// @vitest-environment jsdom
import {act} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {RMusicClient,type Track} from "@r-music/sdk";
import {MusicIdentityPanel} from "./MusicIdentity";
import {CoreExtensions,PluginConfiguration} from "./CoreExtensions";
let host:HTMLDivElement,root:Root;
beforeEach(()=>{const current=globalThis.crypto;vi.stubGlobal("crypto",{getRandomValues:current.getRandomValues.bind(current)});vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();});
async function click(text:string){await act(async()=>[...document.querySelectorAll('button')].find(b=>b.textContent===text)!.click());}
async function input(label:string,value:string){await act(async()=>{const field=document.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`)!;Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value')!.set!.call(field,value);field.dispatchEvent(new Event('input',{bubbles:true}));});}
it("身份关系需手动确认、修订冲突保留编辑，卸载取消读取",async()=>{
 const api=new RMusicClient('http://identity-qa','synthetic'),read=vi.spyOn(api,'trackIdentity').mockResolvedValue({track_id:'qa',revision:4,automatic_merge:false,relations:[],entities:[]});vi.spyOn(api,'fingerprint').mockResolvedValue({available:false,automatic_merge:false,reason:'尚未分析'});
 vi.spyOn(api,'previewTrackIdentity').mockResolvedValue({before:{track_id:'qa',revision:4,entities:[],relations:[],automatic_merge:false},after:{track_id:'qa',revision:4,entities:[],relations:[],automatic_merge:false},resolution:{},affected_candidates:1,preview_only:true,playback_ids_unchanged:true});
 const save=vi.spyOn(api,'editTrackIdentity').mockRejectedValue(new Error('音乐身份已被修改，请重新读取后确认'));const analyze=vi.spyOn(api,'analyzeFingerprint').mockResolvedValue({id:'job',status:'pending'});
 await act(async()=>root.render(<MusicIdentityPanel api={api} track={{id:'qa',title:'测试'} as Track} play={async()=>{}} enqueue={async()=>{}}/>));expect(analyze).not.toHaveBeenCalled();await click('编辑确认关系');await click('添加关系');await input('稳定 ID 1','11111111-1111-4111-8111-111111111111');expect(host.querySelector<HTMLButtonElement>('.identity-editor .primary')?.disabled).toBe(true);await click('预览变更');await click('保存人工确认');expect(save.mock.calls[0][1]).toMatchObject({expected_revision:4,assertions:[{kind:'work',namespace:'rmusic-manual',title:'测试'}]});expect(host.textContent).toContain('音乐身份已被修改');expect(host.querySelector('.identity-editor')).not.toBeNull();
 read.mockResolvedValue({track_id:'qa',revision:5,automatic_merge:false,relations:[],entities:[]});await click('刷新身份与指纹');expect(host.querySelector<HTMLButtonElement>('.identity-editor .primary')?.disabled).toBe(true);
 const signal=read.mock.calls[0][1]!;await act(async()=>root.unmount());root=createRoot(host);expect(signal.aborted).toBe(true);
});
it("配置保存保留嵌套密码，不把脱敏占位写回，使用读取的修订号",async()=>{
 const api=new RMusicClient('http://config-qa','synthetic');vi.spyOn(api,'pluginConfiguration').mockResolvedValue({revision:'revision-2',schema:{type:'object',properties:{connection:{type:'object',properties:{password:{type:'string',secret:true},label:{type:'string'}}}}},values:{connection:{password:{configured:true},label:'first'}}});const write=vi.spyOn(api,'configurePlugin').mockResolvedValue({saved:true,revision:'next'}),close=vi.fn();
 await act(async()=>root.render(<PluginConfiguration api={api} id="example" close={close}/>));expect(document.querySelector<HTMLInputElement>('input[type=password]')!.value).toBe('');await input('connection.label','second');await click('保存配置');expect(write).toHaveBeenCalledWith('example',{expected_revision:'revision-2',values:{connection:{label:'second'}}},expect.any(AbortSignal));expect(close).toHaveBeenCalled();
});
it("扩展只有显式点击才调用，建议结果不自动应用",async()=>{
 const api=new RMusicClient('http://extension-qa','synthetic');vi.spyOn(api,'extensions').mockResolvedValue({items:[{id:'report',name:'报告',version:'1',extension_api:'1',enabled:true,extensions:[{method:'summarize',kind:'automation',scope:'control',effect:'proposal',input:{type:'object',properties:{}},output:{type:'boolean'}}]}]});const invoke=vi.spyOn(api,'invokeExtension').mockResolvedValue({plugin_id:'report',method:'summarize',version:1,effect:'proposal',host_actions_applied:false,output:false});
 await act(async()=>root.render(<CoreExtensions api={api}/>));expect(invoke).not.toHaveBeenCalled();await click('打开');expect(invoke).not.toHaveBeenCalled();await click('运行扩展');expect(invoke).toHaveBeenCalledWith('report','summarize',{},expect.any(AbortSignal));expect(document.body.textContent).toContain('结果为建议，尚未应用到 Core');
});
it("结构化敏感配置只发送显式输入的值，不回写脱敏标记",async()=>{
 const api=new RMusicClient('http://secret-qa','synthetic');vi.spyOn(api,'pluginConfiguration').mockResolvedValue({revision:'r1',schema:{type:'object',properties:{keys:{type:'array',secret:true,items:{type:'string'}}}},values:{keys:{configured:true}}});const write=vi.spyOn(api,'configurePlugin').mockResolvedValue({saved:true,revision:'r2'});
 await act(async()=>root.render(<PluginConfiguration api={api} id="secret-example" close={()=>{}}/>));await click('更改keys');await act(async()=>{const details=document.querySelector('details')!;details.open=true;details.dispatchEvent(new Event('toggle'));});const field=document.querySelector<HTMLTextAreaElement>('textarea[aria-label=keys]')!;expect(field.value).toBe('');await act(async()=>{Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value')!.set!.call(field,'["synthetic-secret"]');field.dispatchEvent(new Event('input',{bubbles:true}));});await click('保存配置');expect(write).toHaveBeenCalledWith('secret-example',{expected_revision:'r1',values:{keys:['synthetic-secret']}},expect.any(AbortSignal));
});
