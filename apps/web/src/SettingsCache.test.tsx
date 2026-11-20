// @vitest-environment jsdom
import {act} from 'react';
import {createRoot} from 'react-dom/client';
import {it,expect,vi} from 'vitest';
import type {RMusicClient,User} from '@r-music/sdk';
import {SettingsView} from './Settings';
vi.mock('./BackgroundTasks',()=>({BackgroundTasks:()=>null,CoreResourceSettings:()=>null}));
it('缓慢首读不会被轮询作废，初始化真实预算，离页取消请求',async()=>{
 vi.useFakeTimers();vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT',true);let resolve:(value:unknown)=>void=()=>{};let signal:AbortSignal|undefined;
 const request=vi.fn((_path:string,options?:RequestInit)=>{signal=options?.signal as AbortSignal;return new Promise(done=>{resolve=done;});});
 const api={get:vi.fn(async()=>({items:[]})),request:(path:string,options?:RequestInit)=>path.startsWith('/cache?')?request(path,options):Promise.resolve({})} as unknown as RMusicClient;
 const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
 try{
  await act(async()=>root.render(<SettingsView api={api} preferences={{theme:'graphite',mode:'dark',low_performance:false,custom_themes:[]}} savePreferences={async()=>{}} plugins={[]} user={{id:'qa',username:'qa',role:'admin'} as User} notify={()=>{}} onError={()=>{}} refreshLibrary={()=>{}} profiles={[]} connect={()=>{}} refreshZones={async()=>{}}/>));
  await act(async()=>Array.from(host.querySelectorAll('button')).find(button=>button.textContent==='服务端缓存')!.click());
  expect(request).toHaveBeenCalledTimes(1);expect(request.mock.calls[0][0]).toBe('/cache?limit=50&offset=0');
  await act(async()=>vi.advanceTimersByTimeAsync(9000));expect(request).toHaveBeenCalledTimes(1);
  await act(async()=>resolve({limit_bytes:7*1024**3,used_bytes:0,entries:[],total:123,directory:'/qa/cache',portable_directory:'/qa/files'}));
  expect((host.querySelector('input[type=number]') as HTMLInputElement).value).toBe('7');
  expect(host.textContent).toContain('123');
  await act(async()=>root.render(null));expect(signal?.aborted).toBe(true);
 }finally{await act(async()=>root.unmount());host.remove();vi.useRealTimers();vi.unstubAllGlobals();}
});
