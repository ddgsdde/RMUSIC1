// @vitest-environment jsdom
import {act} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,describe,expect,it,vi} from "vitest";
import {RMusicClient,type Zone} from "@r-music/sdk";
import {SaveSmartPlaylist} from "./SaveSmartPlaylist";
import {PlaylistsView} from "./Playlists";

let host:HTMLDivElement,root:Root;
const onError=vi.fn(),notify=vi.fn(),saved=vi.fn(),refresh=vi.fn(async()=>{});
const smart={id:"smart",name:"动态",track_ids:[],smart_query:{q:"夜航",source:"local"}};
const zone={id:"z",name:"QA",queue:[]} as unknown as Zone;
beforeEach(()=>{vi.clearAllMocks();vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();});
async function click(label:string){const button=[...host.querySelectorAll("button")].find(b=>b.getAttribute("aria-label")===label||b.textContent===label)!;expect(button).toBeDefined();await act(async()=>button.click());}
async function submit(){await click("保存为智能歌单");await act(async()=>{const input=host.querySelector("input")!;Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(input,"规则");input.dispatchEvent(new Event("input",{bubbles:true}));});await act(async()=>{host.querySelector("form")!.dispatchEvent(new Event("submit",{bubbles:true,cancelable:true}));});}
function playlists(api:RMusicClient){return <PlaylistsView api={api} zone={zone} refreshZones={refresh} onError={onError} notify={notify} plugins={[]} play={refresh} enqueue={refresh} goSettings={()=>{}}/>;}
describe("智能歌单",()=>{
 it("编辑保存到原歌单，保持完整规则而不创建副本",async()=>{
  const api=new RMusicClient("http://qa"),request=vi.spyOn(api,"request").mockResolvedValue({id:"smart/id"});
  const query={q:"更新",source:"qqmusic",album_id:"album",artist_id:"artist"};
  await act(async()=>root.render(<SaveSmartPlaylist api={api} playlist={{id:"smart/id",name:"原名"}} query={query} onError={onError} onSaved={saved}/>));
  await click("更新智能歌单");expect(host.querySelector("input")!.value).toBe("原名");
  await act(async()=>host.querySelector("form")!.dispatchEvent(new Event("submit",{bubbles:true,cancelable:true})));
  expect(request.mock.calls[0][0]).toBe("/playlists/smart%2Fid");expect(request.mock.calls[0][1]!.method).toBe("PUT");
  expect(JSON.parse(request.mock.calls[0][1]!.body as string)).toEqual({name:"原名",smart_query:query});expect(saved).toHaveBeenCalledOnce();
 });
 it("切换核心取消未完成的编辑保存，不提交迟到通知",async()=>{
  const api=new RMusicClient("http://old"),next=new RMusicClient("http://new");let resolve!:(v:unknown)=>void;
  const request=vi.spyOn(api,"request").mockImplementation(()=>new Promise(r=>{resolve=r;})),newRequest=vi.spyOn(next,"request");
  const render=(client:RMusicClient)=><SaveSmartPlaylist api={client} playlist={{id:"same-id",name:"原名"}} query={{source:"local"}} onError={onError} onSaved={saved}/>;
  await act(async()=>root.render(render(api)));await click("更新智能歌单");
  await act(async()=>host.querySelector("form")!.dispatchEvent(new Event("submit",{bubbles:true,cancelable:true})));
  await act(async()=>root.render(render(next)));expect(request.mock.calls[0][1]!.signal!.aborted).toBe(true);
  await act(async()=>resolve({id:"same-id"}));expect(saved).not.toHaveBeenCalled();expect(newRequest).not.toHaveBeenCalled();expect(onError).not.toHaveBeenCalled();
 });
 it("保存筛选规则而不保存当前分页的曲目",async()=>{
  const api=new RMusicClient("http://qa"),request=vi.spyOn(api,"request").mockResolvedValue({id:"new"});
  await act(async()=>root.render(<SaveSmartPlaylist api={api} query={smart.smart_query} onError={onError} onSaved={saved}/>));await submit();
  expect(JSON.parse(request.mock.calls[0][1]!.body as string)).toEqual({name:"规则",smart_query:smart.smart_query});expect(saved).toHaveBeenCalledOnce();expect(onError).not.toHaveBeenCalled();
 });
 it("关闭保存弹窗取消旧请求，迟到成功不通知",async()=>{
  const api=new RMusicClient("http://qa");let resolve!:(v:unknown)=>void;const request=vi.spyOn(api,"request").mockImplementation(()=>new Promise(r=>{resolve=r;}));
  await act(async()=>root.render(<SaveSmartPlaylist api={api} query={{}} onError={onError} onSaved={saved}/>));await submit();await click("关闭");
  expect(request.mock.calls[0][1]!.signal!.aborted).toBe(true);await act(async()=>resolve({id:"late"}));expect(saved).not.toHaveBeenCalled();expect(onError).not.toHaveBeenCalled();
 });
 it("播放前重新解析当前结果，不使用列表页的空快照",async()=>{
  const api=new RMusicClient("http://qa"),request=vi.spyOn(api,"request").mockImplementation(async path=>path==="/playlist-directory"?Promise.resolve({items:[]}):path==="/playlist-directory"?{items:[]}:path==="/playlists"?{items:[smart]}:path==="/playlists/smart"?{...smart,track_ids:["latest"]}:{});
  await act(async()=>root.render(playlists(api)));await click("播放 动态");
  const queued=request.mock.calls.find(([path])=>path==="/zones/z/queue")!;
  expect(JSON.parse(queued[1]!.body as string)).toEqual({track_ids:["latest"]});expect(request.mock.calls.some(([path])=>path==="/zones/z/play")).toBe(true);expect(refresh).toHaveBeenCalledOnce();
 });
 it("空匹配提示且保留当前音区队列",async()=>{
  const api=new RMusicClient("http://qa"),request=vi.spyOn(api,"request").mockImplementation(async path=>path==="/playlist-directory"?Promise.resolve({items:[]}):path==="/playlist-directory"?{items:[]}:path==="/playlists"?{items:[smart]}:smart);
  await act(async()=>root.render(playlists(api)));await click("播放 动态");expect(notify).toHaveBeenCalledWith("智能歌单当前没有匹配的歌曲");expect(request.mock.calls.some(([path])=>path.startsWith("/zones/"))).toBe(false);
 });
 it("切换核心取消旧解析，迟到结果不写队列或启动播放",async()=>{
  const api=new RMusicClient("http://old"),next=new RMusicClient("http://new");let resolve!:(v:unknown)=>void;
  const request=vi.spyOn(api,"request").mockImplementation(path=>path==="/playlist-directory"?Promise.resolve({items:[]}):path==="/playlists"?Promise.resolve({items:[smart]}):new Promise(r=>{resolve=r;}));vi.spyOn(next,"request").mockResolvedValue({items:[]});
  await act(async()=>root.render(playlists(api)));await click("播放 动态");const signal=request.mock.calls.find(([path])=>path==="/playlists/smart")![1]!.signal!;
  await act(async()=>root.render(playlists(next)));expect(signal.aborted).toBe(true);await act(async()=>resolve({...smart,track_ids:["late"]}));expect(request.mock.calls.some(([path])=>path.startsWith("/zones/"))).toBe(false);expect(onError).not.toHaveBeenCalled();
 });
});
