// @vitest-environment jsdom
import {act} from 'react';
import {createRoot} from 'react-dom/client';
import {expect,it,vi} from 'vitest';
import {RMusicClient} from '@r-music/sdk';
import {CollectionBrowser} from './CollectionBrowser';
it('未归属专辑歌曲有独立入口，不再伪造未知专辑卡片',async()=>{
 vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT',true);const host=document.createElement('div'),root=createRoot(host),api=new RMusicClient('http://core','token'),open=vi.fn();
 vi.spyOn(api,'request').mockResolvedValue({items:[],total:0,unassigned_total:2});
 try{await act(async()=>root.render(<CollectionBrowser api={api} kind="albums" source="" revision={0} select={vi.fn()} onError={vi.fn()} selectUnassigned={open}/>));expect(host.textContent).toContain('未归属专辑 · 2 首');expect(host.querySelectorAll('.album-item').length).toBe(0);await act(async()=>host.querySelector<HTMLButtonElement>('.collection-count button')!.click());expect(open).toHaveBeenCalledOnce();await act(async()=>root.unmount());}finally{host.remove();vi.restoreAllMocks();vi.unstubAllGlobals();}
});
