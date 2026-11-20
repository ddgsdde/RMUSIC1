// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach,expect,it,vi} from "vitest";
import {RMusicClient,type Track} from "@r-music/sdk";
import {TrackContextMenuProvider} from "./TrackContextMenu";
import {SongButtons,SongCollectionProvider} from "./SongCollection";
afterEach(()=>vi.unstubAllGlobals());
it("喜欢可切换保存，并能从歌曲入口添加到已有歌单",async()=>{
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
 const api=new RMusicClient("https://core.test","session"),favorite={id:"favorites",name:"我喜欢",track_ids:[] as string[]};
 const track={id:"local-song",title:"验收曲",artist:"艺人",album:"专辑",source:"local",available:true,duration:180,sample_rate:44100,bit_depth:16,format:"flac",channels:2} as Track;
 vi.spyOn(api,"get").mockImplementation(async path=>path==="/favorites"?favorite:path==="/playlists"?{items:[{id:"my-list",name:"旅行歌单",track_ids:[]}]}:{items:[]});
 const write=vi.spyOn(api,"request").mockImplementation(async(_path,options)=>{favorite.track_ids=JSON.parse(options?.body as string).liked?[track.id]:[];return {...favorite};});
 const add=vi.spyOn(api,"post").mockResolvedValue({}),notice=vi.fn(),error=vi.fn();
 const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 try{
  await act(async()=>root.render(<SongCollectionProvider api={api} plugins={[]} notify={notice} onError={error}><TrackContextMenuProvider api={api} location="library" play={vi.fn()} enqueue={vi.fn()} details={vi.fn()} onError={error}><SongButtons track={track}/></TrackContextMenuProvider></SongCollectionProvider>));
  await act(async()=>host.querySelector<HTMLButtonElement>('button[aria-label="喜欢 验收曲"]')!.click());
  expect(write).toHaveBeenCalledWith("/favorites/local-song",expect.objectContaining({method:"PUT"}));expect(host.querySelector('button[aria-label="取消喜欢 验收曲"]')?.getAttribute("aria-pressed")).toBe("true");
  await act(async()=>host.querySelector<HTMLButtonElement>('button[aria-label="更多歌曲操作 验收曲"]')!.click());
  await act(async()=>[...host.querySelectorAll("[role=menuitem]")].find(button=>button.textContent==="添加到歌单…")!.dispatchEvent(new MouseEvent("click",{bubbles:true})));
  expect(host.textContent).toContain("旅行歌单");
  await act(async()=>[...host.querySelectorAll(".collection-targets button")].find(button=>button.textContent?.includes("旅行歌单"))!.dispatchEvent(new MouseEvent("click",{bubbles:true})));
  expect(add).toHaveBeenCalledWith("/playlists/my-list/tracks",{track_id:"local-song",remove:false});expect(error).not.toHaveBeenCalled();
 }finally{await act(async()=>root.unmount());host.remove();}
});

it("只有显式绑定的本地我喜欢参与同步，不自动绑定平台收藏",async()=>{
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);const api=new RMusicClient("https://core.test","session"),track={id:"qqmusic:song",source:"qqmusic",title:"云端曲"} as Track;
 let favorite={id:"favorites-user",track_ids:[track.id],sync:{provider:"qqmusic",playlist_id:"chosen",account_context:"account",capabilities:["add","remove"]}};
 vi.spyOn(api,"get").mockImplementation(async path=>path==="/favorites"?favorite:{items:[{id:"liked",source:"qqmusic",account_context:"account",writable:true}]});
 const request=vi.spyOn(api,"request").mockImplementation(async()=>{favorite={...favorite,track_ids:[]};return favorite;}),post=vi.spyOn(api,"post").mockResolvedValue({});
 const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 try{await act(async()=>root.render(<SongCollectionProvider api={api} plugins={[{id:"qqmusic",name:"QQ",enabled:true,favorites_playlist_id:"liked"} as any]} notify={vi.fn()} onError={vi.fn()}><SongButtons track={track}/></SongCollectionProvider>));
 const unlike=host.querySelector<HTMLButtonElement>('button[aria-label="取消喜欢 云端曲"]');expect(unlike).not.toBeNull();await act(async()=>unlike!.click());expect(request).toHaveBeenCalledWith("/favorites/qqmusic%3Asong",expect.objectContaining({body:JSON.stringify({liked:false})}));expect(post).not.toHaveBeenCalled();expect(host.querySelector('button[aria-label="喜欢 云端曲"]')).not.toBeNull();
 }finally{await act(async()=>root.unmount());host.remove();}
});

it("完整缓存的本地副本仍向原音源同步喜欢",async()=>{
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);const api=new RMusicClient("http://core","session");const track={id:"local-copy",source:"local",title:"缓存歌曲",provider_data:{cached_from:{track_id:"qqmusic:original",source:"qqmusic"}}} as unknown as Track;
 vi.spyOn(api,"get").mockImplementation(async path=>path==="/favorites"?{track_ids:[]}:path==="/playlists"?{items:[{id:"mirror",track_ids:[],sync:{provider:"qqmusic",playlist_id:"liked",account_context:"account",capabilities:["add","remove"]}}]}:{items:[{id:"liked",source:"qqmusic",account_context:"account",writable:true}]});
 const request=vi.spyOn(api,"request").mockResolvedValue({track_ids:["qqmusic:original"]}),post=vi.spyOn(api,"post").mockResolvedValue({});const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 try{await act(async()=>root.render(<SongCollectionProvider api={api} plugins={[{id:"qqmusic",enabled:true,favorites_playlist_id:"liked"} as any]} notify={vi.fn()} onError={vi.fn()}><SongButtons track={track}/></SongCollectionProvider>));await act(async()=>host.querySelector<HTMLButtonElement>('button[aria-label="喜欢 缓存歌曲"]')!.click());expect(request).toHaveBeenCalledWith("/favorites/qqmusic%3Aoriginal",expect.anything());expect(post).not.toHaveBeenCalled();}finally{await act(async()=>root.unmount());host.remove();}
});
