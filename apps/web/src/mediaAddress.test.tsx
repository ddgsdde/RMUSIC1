// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach,describe,expect,it,vi} from "vitest";
import {RMusicClient} from "@r-music/sdk";
import {disposeArtwork} from "./artworkCache";
import {Cover} from "./components";

afterEach(()=>vi.unstubAllGlobals());
describe("反代媒体地址",()=>{
 it("票据使用当前连接的地址和前缀，保留旧核心兼容",async()=>{
  const api=new RMusicClient("https://backup.test/remote/music");
  vi.spyOn(api,"request").mockResolvedValue({url:"/internal/api/v1/media/qqmusic%3Aabc?ticket=a%2Bb",api_path:"api/v1/media/qqmusic%3Aabc?ticket=a%2Bb"});
  expect(await api.mediaUrl("qqmusic:abc")).toBe("https://backup.test/remote/music/api/v1/media/qqmusic%3Aabc?ticket=a%2Bb");
  vi.mocked(api.request).mockResolvedValue({url:"/remote/music/api/v1/media/old?ticket=t"});
  expect(await api.mediaUrl("old")).toBe("https://backup.test/remote/music/api/v1/media/old?ticket=t");
 });
 it.each(["covers","artwork"])("%s 图片通过当前反代前缀加载并携带核心凭据",async(kind)=>{
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
  const fetch=vi.fn(async()=>({ok:true,blob:async()=>new Blob()}));vi.stubGlobal("fetch",fetch);
  const create=vi.fn(()=>"blob:cover"),revoke=vi.fn();URL.createObjectURL=create;URL.revokeObjectURL=revoke;
  const api=new RMusicClient("https://backup.test/remote/music","session");const container=document.createElement("div"),root=createRoot(container);
  try{
   await act(async()=>root.render(<Cover api={api} track={{cover_url:`/api/v1/${kind}/id`,title:"歌曲",album:"专辑"}}/>));
   expect(fetch.mock.calls[0]).toEqual([new URL(`https://backup.test/remote/music/api/v1/${kind}/id`),{headers:{Authorization:"Bearer session"},signal:expect.any(AbortSignal)}]);
   expect(container.querySelector("img")?.src).toBe("blob:cover");
  }finally{await act(async()=>root.unmount());}
  expect(revoke).not.toHaveBeenCalled();disposeArtwork(api);expect(revoke).toHaveBeenCalledWith("blob:cover");
 });
});
it('临时封面错误只重试一次，完成后回收图片与请求',async()=>{
 vi.useFakeTimers();vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT',true);
 const fetch=vi.fn().mockResolvedValueOnce({ok:false,status:503}).mockResolvedValueOnce({ok:true,blob:async()=>new Blob()});vi.stubGlobal('fetch',fetch);
 URL.createObjectURL=vi.fn(()=> 'blob:retry');URL.revokeObjectURL=vi.fn();
 const api=new RMusicClient('http://core','token');const host=document.createElement('div'),root=createRoot(host);
 try {await act(async()=>root.render(<Cover api={api} track={{cover_url:'/api/v1/artwork/key',title:'Song',album:'Album'}}/>));
  await act(async()=>vi.advanceTimersByTimeAsync(600));expect(fetch).toHaveBeenCalledTimes(2);expect(host.querySelector('img')?.src).toBe('blob:retry');
  await act(async()=>root.unmount());expect(URL.revokeObjectURL).not.toHaveBeenCalled();disposeArtwork(api);expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:retry');
 }finally{vi.useRealTimers();vi.unstubAllGlobals();}
});

it('仅可见范围内的封面请求加载，离开页面仍可复用图片',async()=>{
 vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT',true);
 let notify:IntersectionObserverCallback=()=>{};
 const disconnect=vi.fn();
 vi.stubGlobal('IntersectionObserver',class {constructor(cb:IntersectionObserverCallback){notify=cb;}observe(){}disconnect(){disconnect();}unobserve(){}});
 const fetch=vi.fn(async()=>new Response(new Blob(['pixels'])));vi.stubGlobal('fetch',fetch);
 URL.createObjectURL=vi.fn(()=> 'blob:viewport');URL.revokeObjectURL=vi.fn();
 const api=new RMusicClient('http://core','token'),host=document.createElement('div'),root=createRoot(host);
 await act(async()=>root.render(<Cover api={api} track={{cover_url:'/api/v1/artwork/key',title:'Song',album:'Album'}}/>));
 expect(fetch).not.toHaveBeenCalled();
 await act(async()=>notify([{isIntersecting:true} as IntersectionObserverEntry],{} as IntersectionObserver));
 expect(fetch).toHaveBeenCalledTimes(1);expect(host.querySelector('img')?.src).toBe('blob:viewport');
 await act(async()=>root.unmount());expect(disconnect).toHaveBeenCalled();disposeArtwork(api);
});

it('首次失败后可见图片只自动恢复一次，离页会取消等待',async()=>{
 vi.useFakeTimers();vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT',true);
 const fetch=vi.fn().mockResolvedValueOnce({ok:false,status:404}).mockResolvedValueOnce(new Response(new Blob(['pixels'])));vi.stubGlobal('fetch',fetch);URL.createObjectURL=vi.fn(()=> 'blob:recovered');URL.revokeObjectURL=vi.fn();
 const api=new RMusicClient('http://core','token'),host=document.createElement('div'),root=createRoot(host);
 try{await act(async()=>root.render(<Cover api={api} track={{cover_url:'/api/v1/artwork/first-miss',title:'Artist',album:''}}/>));expect(fetch).toHaveBeenCalledTimes(1);await act(async()=>vi.advanceTimersByTimeAsync(16000));expect(fetch).toHaveBeenCalledTimes(2);expect(host.querySelector('img')?.src).toBe('blob:recovered');await act(async()=>vi.advanceTimersByTimeAsync(60000));expect(fetch).toHaveBeenCalledTimes(2);}
 finally{await act(async()=>root.unmount());disposeArtwork(api);vi.useRealTimers();}
});
