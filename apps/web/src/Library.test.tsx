// @vitest-environment jsdom
import {act} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,describe,expect,it,vi} from "vitest";
import {RMusicClient,type Track} from "@r-music/sdk";
import {Library} from "./Library";

// DOM measurement is unavailable in jsdom; put the viewport at the last row.
vi.mock("@tanstack/react-virtual",()=>({useVirtualizer:({count}:{count:number})=>({
 getVirtualItems:()=>count?[{index:0,start:0,size:58},...(count>1?[{index:count-1,start:(count-1)*58,size:58}]:[])]:[],
 getTotalSize:()=>count*58,
})}));
interface Pending {url:URL;signal:AbortSignal|null|undefined;reply:(status:number,items?:Track[],total?:number)=>void}
let root:Root,host:HTMLDivElement,requests:Pending[];
const onError=vi.fn(),noop=vi.fn(async()=>{});
const song=(id:string)=>({id,title:id,artist:"测试歌手",album:"测试专辑",duration:30,available:true,source:"local",format:"flac",sample_rate:96000,bit_depth:24,channels:2,cover_url:null}) as Track;
const page=(prefix:string)=>Array.from({length:50},(_,i)=>song(`${prefix}-${i}`));
const api=new RMusicClient("http://qa-core");
const scrollTo=Object.getOwnPropertyDescriptor(HTMLElement.prototype,"scrollTo");
async function render(query:string,client=api){
 await act(async()=>root.render(<Library api={client} query={query} source="local" albumId="" artistId="" revision={0} play={noop} enqueue={noop} onError={onError} goSettings={()=>{}} showDetails={()=>{}} browseAlbum={()=>{}}/>));
 await act(async()=>{await vi.advanceTimersByTimeAsync(300);});
}
async function nextPage(){await act(async()=>{[...host.querySelectorAll('button')].find(button=>button.textContent==='下一页')!.click();});await act(async()=>vi.advanceTimersByTimeAsync(300));}
async function reply(request:Pending,status:number,items?:Track[],total?:number){await act(async()=>{request.reply(status,items,total);await vi.advanceTimersByTimeAsync(0);});}
function request(query:string,offset:string){return requests.find(r=>r.url.searchParams.get("q")===query&&r.url.searchParams.get("offset")===offset)!;}
beforeEach(()=>{
 vi.useFakeTimers();vi.clearAllMocks();sessionStorage.clear();requests=[];
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
 Object.defineProperty(HTMLElement.prototype,"scrollTo",{configurable:true,value:vi.fn()});
 vi.stubGlobal("fetch",vi.fn((input:string,init:RequestInit)=>new Promise<Response>(resolve=>{
  requests.push({url:new URL(input),signal:init.signal,reply:(status,items=[],total=items.length)=>resolve(new Response(JSON.stringify(status===200?{items,total}:{error:{code:"unauthorized",message:"旧核心登录已过期"}}),{status,headers:{"Content-Type":"application/json"}}))});
 })));
 host=document.createElement("div");document.body.append(host);root=createRoot(host);
});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();if(scrollTo)Object.defineProperty(HTMLElement.prototype,"scrollTo",scrollTo);else Reflect.deleteProperty(HTMLElement.prototype,"scrollTo");vi.useRealTimers();vi.unstubAllGlobals();});
describe("曲目分页的请求生命周期",()=>{
 it("后台刷新保留曲目和位置，失败不清空当前页",async()=>{
  const props={api,query:"",source:"local",albumId:"",artistId:"",play:noop,enqueue:noop,onError,goSettings:()=>{},showDetails:()=>{},browseAlbum:()=>{}};
  await act(async()=>root.render(<Library {...props} revision={0}/>));await act(async()=>vi.advanceTimersByTimeAsync(0));
  await reply(request("","0"),200,page("保留"),100);
  const scrollCalls=vi.mocked(HTMLElement.prototype.scrollTo).mock.calls.length;
  await act(async()=>root.render(<Library {...props} revision={1}/>));await act(async()=>vi.advanceTimersByTimeAsync(0));
  expect(host.textContent).toContain("保留-0");
  const pending=requests.filter(r=>r.url.pathname.endsWith("/tracks")).at(-1)!;
  await reply(pending,500);
  expect(host.textContent).toContain("保留-0");expect(HTMLElement.prototype.scrollTo).toHaveBeenCalledTimes(scrollCalls);
 });
 it("编辑智能规则时只预览入库匹配，重扫会刷新搜索结果",async()=>{
  const props={api,query:"夜航",source:"qqmusic",albumId:"",artistId:"",play:noop,enqueue:noop,onError,goSettings:()=>{},showDetails:()=>{},browseAlbum:()=>{},smartPlaylists:true,editingPlaylist:{id:"smart",name:"规则",track_ids:[],smart_query:{q:"夜航",source:"qqmusic"}}};
  await act(async()=>root.render(<Library {...props} revision={1}/>));await act(async()=>{await vi.advanceTimersByTimeAsync(300);});
  const first=request("夜航","0");expect(first.url.pathname).toBe("/api/v1/library/tracks");expect(requests.some(r=>r.url.pathname.includes("/search"))).toBe(false);
  expect(host.textContent).toContain("更新智能歌单");
  await act(async()=>root.render(<Library {...props} revision={2}/>));expect(first.signal?.aborted).toBe(true);await act(async()=>{await vi.advanceTimersByTimeAsync(300);});
  expect(requests.filter(r=>r.url.pathname==="/api/v1/library/tracks")).toHaveLength(2);
 });
 it("切页由用户触发，查询切换后忽略旧分页的401",async()=>{
  await render("旧查询");await reply(request("旧查询","0"),200,page("旧曲"),200);await nextPage();
  const old=request("旧查询","50");expect(old).toBeDefined();
  await render("新查询");expect(host.textContent).not.toContain("旧曲-49");await reply(request("新查询","0"),200,page("新曲"),200);await nextPage();
  expect(request("新查询","50")).toBeDefined();
  await reply(old,401);expect(onError).not.toHaveBeenCalled();
  expect(host.textContent).not.toContain("旧曲-49");
 });
 it("切换查询会取消旧分页，迟到的 finally 不清除新页加载状态",async()=>{
  await render("旧查询");await reply(request("旧查询","0"),200,page("旧曲"),200);await nextPage();const old=request("旧查询","50");
  await render("新查询");await reply(request("新查询","0"),200,page("新曲"),200);await nextPage();const current=request("新查询","50");
  expect(old.signal?.aborted).toBe(true);expect(current).toBeDefined();
  await reply(old,200,[song("迟到旧曲")],200);
  expect([...host.querySelectorAll("button")].find(button=>button.textContent==="下一页")?.disabled).toBe(true);expect(host.textContent).not.toContain("迟到旧曲");
  await reply(current,200,[song("新分页")],101);expect(host.textContent).toContain("新分页");expect(host.textContent).not.toContain("加载更多");
 });
 it("卸载后取消在途分页并忽略错误",async()=>{
  await render("查询");await reply(request("查询","0"),200,page("歌曲"),200);await nextPage();const pending=request("查询","50");
  await act(async()=>root.render(null));expect(pending.signal?.aborted).toBe(true);
  await reply(pending,401);expect(onError).not.toHaveBeenCalled();
 });
 it("切换核心后不接受旧核心的列表与分页错误",async()=>{
  await render("查询");await reply(request("查询","0"),200,page("旧核心"),200);await nextPage();const old=request("查询","50");
  await render("查询",new RMusicClient("http://new-core"));
  const current=requests.find(r=>r.url.host==="new-core")!;await reply(current,200,[song("新核心歌曲")],1);
  await reply(old,401);expect(onError).not.toHaveBeenCalled();expect(host.textContent).toContain("新核心歌曲");
 });
});
