// @vitest-environment jsdom
import {act} from 'react';
import {hasAlbum} from './CatalogLinks';
import {createRoot} from 'react-dom/client';
import {expect,it,vi} from 'vitest';
import {isNewRelease,LibraryFilterBar,filterQuery} from './CatalogControls';
it('新歌只采用真实发行日，未来日期、无效日期及满30天不标记',()=>{
 const now=Date.parse('2026-10-27T10:00:00Z');expect(isNewRelease('2026-10-20',now)).toBe(true);expect(isNewRelease('2026-10-28',now)).toBe(false);expect(isNewRelease('2026-09-27',now)).toBe(false);expect(isNewRelease('',now)).toBe(false);expect(isNewRelease('2026-02-30',now)).toBe(false);
 expect(filterQuery({format:'flac',min_rate:'96000',artwork:''})).toBe('format=flac&min_rate=96000');
});
it('筛选和排序写入受控参数，不下载全库到浏览器排序',async()=>{
 vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT',true);const host=document.createElement('div'),root=createRoot(host),change=vi.fn();
 try{await act(async()=>root.render(<LibraryFilterBar value={{format:'flac'}} change={change}/>));const sort=host.querySelector<HTMLSelectElement>('[aria-label="排序"]')!;sort.value='release_old';await act(async()=>sort.dispatchEvent(new Event('change',{bubbles:true})));expect(change).toHaveBeenCalledWith({format:'flac',sort:'release_old'});expect(host.textContent).toContain('播放列表');expect(host.textContent).toContain('发行起始');await act(async()=>root.unmount());}finally{host.remove();vi.unstubAllGlobals();}
});

it('缺少专辑资料不生成假链接，真实名为None的发行仍可跳转',()=>{
 expect(hasAlbum({artist:'A',album:''})).toBe(false);expect(hasAlbum({artist:'A',album:'None'})).toBe(false);expect(hasAlbum({artist:'A',album:'None',provider_data:{album_mid:'VerifiedAlbum'}})).toBe(true);
});
