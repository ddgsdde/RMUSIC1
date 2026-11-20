// @vitest-environment jsdom
import {act} from 'react';
import {createRoot} from 'react-dom/client';
import {expect,it,vi} from 'vitest';
import {RMusicClient} from '@r-music/sdk';
import {ArtistAlbums} from './ArtistAlbums';
import {CatalogNavigation} from './CatalogLinks';
it('专辑独立分页、点击明确发行身份且卸载取消读取', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT',true);
  const host=document.createElement('div'),root=createRoot(host),api=new RMusicClient('http://core','token'),navigate=vi.fn();
  const request=vi.spyOn(api,'request').mockResolvedValue({local:{items:[],total:0,page:1},platforms:[{source:'qqmusic',page:1,total:31,next_page:2,items:[{id:'release:qqmusic:Album001',title:'精确专辑',source:'qqmusic'}]}]});
  try {
    await act(async()=>root.render(<CatalogNavigation api={api} location="artist" navigate={navigate} onError={vi.fn()}><ArtistAlbums api={api} artistID="artist-id"/></CatalogNavigation>));
    expect(request.mock.calls[0][0]).toContain('/library/catalog/artist/artist-id/albums');
    await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="打开专辑 精确专辑"]')!.click());
    expect(navigate).toHaveBeenCalledWith('album','release:qqmusic:Album001','精确专辑');
    await act(async()=>[...host.querySelectorAll('button')].find(x=>x.textContent==='下一页')!.click());
    expect(request.mock.calls[1][0]).toContain('page=2&provider=qqmusic');expect(request.mock.calls[1][0]).toContain('refresh=false');
    const signal=request.mock.calls[1][1]!.signal;
    await act(async()=>root.unmount());expect(signal!.aborted).toBe(true);
  } finally {host.remove();vi.restoreAllMocks();vi.unstubAllGlobals();}
});
