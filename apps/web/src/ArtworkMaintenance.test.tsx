// @vitest-environment jsdom
import {act} from 'react';
import {createRoot} from 'react-dom/client';
import {expect,it,vi,afterEach} from 'vitest';
import {RMusicClient} from '@r-music/sdk';
import {ArtworkMaintenance} from './ArtworkMaintenance';
import {CatalogScrapeSettings} from './CatalogScrapeSettings';
afterEach(()=>vi.unstubAllGlobals());
it('图片维护分页读取，检查不自动刷新，单项修复保留作用范围',async()=>{
 vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT',true);const api=new RMusicClient('http://fixture','token');const id='a'.repeat(64);const read=vi.spyOn(api,'artworkCache').mockResolvedValue({items:[{id,name:'Fixture',host:'example.test',cover_id:null,state:'missing'}],total:1,offset:0,limit:50,used_bytes:123,limit_bytes:536870912,file_count:1,protected_base_images:true,derived_count:0,metrics:{scope:'since_start',requests:2,not_modified:1,response_bytes:123,upstream_bytes:123,upstream_reads:1},jobs:[]});const inspect=vi.spyOn(api,'inspectArtwork').mockResolvedValue({id:'check',status:'pending'}),refresh=vi.spyOn(api,'refreshArtwork').mockResolvedValue({id:'refresh',status:'pending'});
 const host=document.createElement('div'),root=createRoot(host);try{await act(async()=>root.render(<ArtworkMaintenance api={api}/>));expect(read.mock.calls[0].slice(0,2)).toEqual([0,false]);expect(refresh).not.toHaveBeenCalled();await act(async()=>[...host.querySelectorAll('button')].find(b=>b.textContent==='检查并标记')!.click());expect(inspect).toHaveBeenCalledOnce();expect(refresh).not.toHaveBeenCalled();await act(async()=>[...host.querySelectorAll('button')].find(b=>b.textContent==='刷新图片')!.click());expect(refresh.mock.calls[0][0]).toEqual([id]);}finally{await act(async()=>root.unmount());}expect(read.mock.calls[0][2]?.aborted).toBe(true);
});
it('刮削默认关闭且并发为一，保存与卸载都使用可取消请求',async()=>{
 vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT',true);const api=new RMusicClient('http://fixture','token');const request=vi.spyOn(api,'request').mockResolvedValue({});const host=document.createElement('div'),root=createRoot(host);
 try{await act(async()=>root.render(<CatalogScrapeSettings api={api}/>));expect((host.querySelector('input[type=checkbox]')as HTMLInputElement).checked).toBe(false);expect((host.querySelector('input[type=number]')as HTMLInputElement).value).toBe('1');await act(async()=>host.querySelector('form')!.dispatchEvent(new Event('submit',{bubbles:true,cancelable:true})));expect(JSON.parse(request.mock.calls.at(-1)![1]!.body as string)).toEqual({catalog_scrape_enabled:false,catalog_scrape_concurrency:1});}finally{await act(async()=>root.unmount());}expect(request.mock.calls.at(-1)![1]!.signal?.aborted).toBe(true);
});
it('全选只覆盖当前分页，取消全选与换页清除选择，批量刷新不扩大范围',async()=>{
 vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT',true);
 const api=new RMusicClient('http://fixture','token');
 const ids=['a'.repeat(64),'b'.repeat(64),'c'.repeat(64)];
 vi.spyOn(api,'artworkCache').mockImplementation(async offset=>({items:(offset===0?ids.slice(0,2):ids.slice(2)).map(id=>({id,name:id[0],host:'example.test',cover_id:null,state:'missing'})),total:51,offset:offset||0,limit:50,used_bytes:123,limit_bytes:536870912,file_count:3,protected_base_images:true,derived_count:0,metrics:{scope:'since_start',requests:0,not_modified:0,response_bytes:0,upstream_bytes:0,upstream_reads:0},jobs:[]}));
 const refresh=vi.spyOn(api,'refreshArtwork').mockResolvedValue({id:'refresh',status:'pending'});
 const host=document.createElement('div'),root=createRoot(host);
 const button=(label:string)=>[...host.querySelectorAll('button')].find(b=>b.textContent===label)!;
 const choices=()=>[...host.querySelectorAll<HTMLInputElement>('input[aria-label^="选择 "]')];
 try{
  await act(async()=>root.render(<ArtworkMaintenance api={api}/>));
  await act(async()=>button('全选本页').click());expect(choices().map(c=>c.checked)).toEqual([true,true]);expect(button('刷新所选 · 2').disabled).toBe(false);
  await act(async()=>button('取消全选').click());expect(choices().every(c=>!c.checked)).toBe(true);
  await act(async()=>button('全选本页').click());
  await act(async()=>button('下一页').click());expect(choices().map(c=>c.checked)).toEqual([false]);expect(button('刷新所选 · 0').disabled).toBe(true);
  await act(async()=>button('全选本页').click());
  await act(async()=>button('刷新所选 · 1').click());expect(refresh.mock.calls[0][0]).toEqual([ids[2]]);
 }finally{await act(async()=>root.unmount());}
});
