// @vitest-environment jsdom
import {act} from 'react';
import {createRoot,type Root} from 'react-dom/client';
import {afterEach,beforeEach,describe,it,expect,vi} from 'vitest';
import type {RMusicClient,Playlist,Track} from '@r-music/sdk';
import {PlaylistEditor} from './PlaylistEditor';
let host:HTMLDivElement,root:Root;
beforeEach(()=>{vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT',true);host=document.createElement('div');document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();});
const track=(id:string)=>({id,title:'Title '+id,artist:'QA'}) as Track;
const playlist={id:'p',name:'Original',track_ids:['a','b','a']} as Playlist;
const buttons=()=>Array.from(document.querySelectorAll('button'));
describe('歌单编辑事务与生命周期',()=>{
 it('按位置移动重复曲目，冲突保留草稿并发送原始比较条件',async()=>{
  const writes:unknown[]=[],onError=vi.fn(),onSaved=vi.fn();
  const api={request:vi.fn(async(_path:string,options?:RequestInit)=>{
   if(options?.method==='PUT'){writes.push(JSON.parse(options.body as string));throw Object.assign(new Error('playlist_edit_conflict'),{status:409});}
   return {items:[track('a'),track('b')]};
  })} as unknown as RMusicClient;
  await act(async()=>root.render(<PlaylistEditor api={api} playlist={playlist} onClose={()=>{}} onSaved={onSaved} onError={onError}/>));
  await act(async()=>buttons().find(button=>button.getAttribute('aria-label')==='上移第 3 首')!.click());
  await act(async()=>document.querySelector('form')!.dispatchEvent(new Event('submit',{bubbles:true,cancelable:true})));
  expect(writes).toEqual([{name:'Original',track_ids:['a','a','b'],expected:{name:'Original',track_ids:['a','b','a']}}]);
  expect(onSaved).not.toHaveBeenCalled();expect(onError).toHaveBeenCalled();expect(document.querySelector('form')).not.toBeNull();
 });
 it('关闭编辑页取消读取和保存，迟到结果不提交页面副作用',async()=>{
  let readSignal:AbortSignal|undefined,saveSignal:AbortSignal|undefined;let resolveSave:()=>void=()=>{};const onSaved=vi.fn(),onError=vi.fn();
  const api={request:vi.fn((_path:string,options?:RequestInit)=>{
   if(options?.method==='PUT'){saveSignal=options.signal as AbortSignal;return new Promise<void>(resolve=>{resolveSave=resolve;});}
   readSignal=options?.signal as AbortSignal;return new Promise(()=>{});
  })} as unknown as RMusicClient;
  await act(async()=>root.render(<PlaylistEditor api={api} playlist={playlist} onClose={()=>{}} onSaved={onSaved} onError={onError}/>));
  await act(async()=>document.querySelector('form')!.dispatchEvent(new Event('submit',{bubbles:true,cancelable:true})));
  await act(async()=>root.render(null));expect(readSignal?.aborted).toBe(true);expect(saveSignal?.aborted).toBe(true);
  await act(async()=>resolveSave());expect(onSaved).not.toHaveBeenCalled();expect(onError).not.toHaveBeenCalled();
 });
});
