// @vitest-environment jsdom
import {act} from 'react';
import {createRoot} from 'react-dom/client';
import {expect,it,vi} from 'vitest';
import {AndroidQuality} from './AndroidQuality';
const {invoke}=vi.hoisted(()=>({invoke:vi.fn()}));
vi.mock('./native',()=>({invokeNative:invoke}));
it('keeps Wi-Fi and mobile choices independent and saves both explicitly',async()=>{
 vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT',true);
 invoke.mockResolvedValueOnce({wifi:'hires',mobile:'original',network:'wifi'}).mockResolvedValueOnce({wifi:'hires',mobile:'lossless',network:'wifi'});
 const host=document.createElement('div');document.body.append(host);const root=createRoot(host),notify=vi.fn(),error=vi.fn();
 try{
  await act(async()=>root.render(<AndroidQuality notify={notify} onError={error}/>));
  const wifi=host.querySelector<HTMLSelectElement>('[aria-label="Wi-Fi音质"]')!,mobile=host.querySelector<HTMLSelectElement>('[aria-label="移动流量音质"]')!;
  expect(wifi.value).toBe('hires');expect(mobile.value).toBe('original');
  await act(async()=>{mobile.value='lossless';mobile.dispatchEvent(new Event('change',{bubbles:true}));});
  expect(wifi.value).toBe('hires');expect(invoke).toHaveBeenCalledTimes(1);
  await act(async()=>[...host.querySelectorAll('button')].find(b=>b.textContent==='保存音质选择')!.click());
  expect(invoke).toHaveBeenLastCalledWith('native_android_quality',{values:{wifi:'hires',mobile:'lossless'}});
  expect(error).not.toHaveBeenCalled();expect(notify).toHaveBeenCalled();
 }finally{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();}
});
