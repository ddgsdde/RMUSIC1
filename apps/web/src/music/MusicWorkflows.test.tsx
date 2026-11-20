// @vitest-environment jsdom
import {LocalPlaylistView} from "../LocalPlaylistView";
import {DiscoveryCollections} from "./DiscoveryCollections";
import {act} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {RMusicClient,type Track} from "@r-music/sdk";
import {LlmSettings} from "./LlmSettings";
import {LlmPicks} from "./LlmPicks";
import {MusicVersions} from "./Versions";
import {SmartPlaylistEditor} from "./SmartPlaylistEditor";
import {PluginWorkbench} from "./PluginWorkbench";

let host:HTMLDivElement,root:Root;
beforeEach(()=>{vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.restoreAllMocks();vi.unstubAllGlobals();});
async function click(text:string){const b=[...document.querySelectorAll("button")].find(b=>b.textContent===text);expect(b,text).toBeTruthy();await act(async()=>b!.click());}
async function input(field:HTMLInputElement|HTMLSelectElement,value:string){await act(async()=>{Object.getOwnPropertyDescriptor(field instanceof HTMLInputElement?HTMLInputElement.prototype:HTMLSelectElement.prototype,"value")!.set!.call(field,value);field.dispatchEvent(new Event(field instanceof HTMLInputElement?"input":"change",{bubbles:true}));});}
async function submit(form:HTMLFormElement){await act(async()=>form.dispatchEvent(new Event("submit",{bubbles:true,cancelable:true})));}
const track={id:"t",title:"测试歌曲",artist:"林间",album:"夜航",source:"local",available:true,format:"flac",sample_rate:96000,bit_depth:24} as Track;

it("LLM 默认不请求推荐，保存密钥后不回填，模型列表由接口读取",async()=>{
  const api=new RMusicClient("http://llm-config"),request=vi.spyOn(api,"request").mockImplementation(async(path,init)=>{
    if(path==="/llm/models")return {items:["qa-model"]};
    if(path==="/recommendations/settings")return {advanced:init?.method==="PUT"};
    if(init?.method==="PUT")return {enabled:false,base_url:"https://compatible.example",model:"qa-model",revision:"2",key_configured:true};
    return {enabled:false,base_url:"https://compatible.example",model:"",revision:"1",key_configured:false};
  });
  await act(async()=>root.render(<LlmSettings api={api} admin/>));expect(request.mock.calls.map(([path])=>path)).toEqual(["/llm/settings","/recommendations/settings"]);
  await input(host.querySelector('input[type="password"]')!,"synthetic-credential");await click("获取可用模型");expect(host.querySelector<HTMLInputElement>('input[list="llm-models"]')!.value).toBe("qa-model");
  await submit(host.querySelector("form")!);expect(host.querySelector<HTMLInputElement>('input[type="password"]')!.value).toBe("");expect(host.textContent).not.toContain("synthetic-credential");
  await input(host.querySelector("select")!,"advanced");
  expect(request).toHaveBeenLastCalledWith("/recommendations/settings",expect.objectContaining({method:"PUT",body:JSON.stringify({advanced:true})}));
  const saved=request.mock.calls.find(([path,options])=>path==="/llm/settings"&&options?.method==="PUT")!;expect(JSON.parse(saved[1]!.body as string)).toMatchObject({expected_revision:"1",model:"qa-model",key:"synthetic-credential"});
});

it("LLM 取消会中止请求，失败保留候选，不启动播放",async()=>{
  const api=new RMusicClient("http://llm-cancel"),play=vi.fn();let signal:AbortSignal|undefined;
  vi.spyOn(api,"request").mockImplementation((_path,options)=>{signal=options?.signal as AbortSignal;return new Promise(()=>{});});
  await act(async()=>root.render(<LlmPicks api={api} candidates={[track]} play={play} goSettings={()=>{}}/>));await click("可选 LLM 推荐");await submit(host.querySelector("form")!);expect(signal?.aborted).toBe(false);await click("取消请求");expect(signal?.aborted).toBe(true);expect(play).not.toHaveBeenCalled();
});

it("LLM 精选保存使用稳定幂等键，错误回复不清空结果",async()=>{
  const api=new RMusicClient("http://llm-save"),bodies:string[]=[];let fail=true;
  vi.spyOn(api,"request").mockImplementation(async(path,options)=>{if(path==="/llm/recommend")return {items:[track],reasons:{t:"安静器乐"},model:"qa-model",ignored:1};bodies.push(options!.body as string);if(fail)throw new TypeError("Failed to fetch");return {id:"saved"};});
  await act(async()=>root.render(<LlmPicks api={api} candidates={[track]} play={async()=>{}} goSettings={()=>{}}/>));await click("可选 LLM 推荐");await submit(host.querySelector("form")!);expect(host.textContent).toContain("忽略 1 个");await submit(host.querySelectorAll("form")[1]);expect(host.textContent).toContain("测试歌曲");fail=false;await submit(host.querySelectorAll("form")[1]);expect(JSON.parse(bodies[0]).request_id).toBe(JSON.parse(bodies[1]).request_id);expect(host.textContent).toContain("已保存为本地播放列表");
});

it("智能规则模板可预览，收藏条件和范围完整保存，卸载中止请求",async()=>{
  const api=new RMusicClient("http://rules"),saved=vi.fn();const request=vi.spyOn(api,"request").mockResolvedValue({items:[track],total:1,truncated:false,offset:0});
  await act(async()=>root.render(<SmartPlaylistEditor api={api} providers={[]} close={()=>{}} saved={saved}/>));
  await input(host.querySelector("input")!,"高解析");await input(host.querySelector("select")!,"hires");await click("预览匹配");expect(host.textContent).toContain("测试歌曲");
  await submit(host.querySelector("form")!);const body=JSON.parse(request.mock.calls.at(-1)![1]!.body as string);expect(body.smart_query).toMatchObject({min_sample_rate:88200,min_bit_depth:24,max_results:100});expect(saved).toHaveBeenCalledOnce();
});

it("版本面板折叠时不加载，偏好仅显式设置，关闭取消读取",async()=>{
  const api=new RMusicClient("http://versions"),request=vi.spyOn(api,"request").mockResolvedValue({identity_resolved:true,items:[{track,edition:{revision:0,kind:"unspecified",label:"",notes:""}}],total:1,offset:0,limit:20,other_recordings:[],preferred:{track_id:null,revision:0},edition:{revision:0,kind:"unspecified",label:"",notes:""}});
  await act(async()=>root.render(<MusicVersions api={api} track={track} play={async()=>{}} enqueue={async()=>{}}/>));expect(request).not.toHaveBeenCalled();
  await act(async()=>{host.querySelector("details")!.open=true;host.querySelector("details")!.dispatchEvent(new Event("toggle"));});expect(host.textContent).toContain("相同且唯一的录音身份");await click("设为推荐偏好");expect(request.mock.calls.find(([,o])=>o?.method==="PUT")?.[1]?.body).toBe(JSON.stringify({expected_revision:0,track_id:"t"}));
});

it("插件工具展示真实检查结果，合同失败不会显示通过",async()=>{
  const api=new RMusicClient("http://plugin-tools");let fail=true;
  vi.spyOn(api,"request").mockImplementation(async path=>{if(path==="/plugins/development")return {api_version:"1",sdk_version:"1",commands:["python3 scripts/plugin_dev.py check demo"],limits:{},trust:"可信代码"};if(fail)throw new Error("合同不兼容");return {passed:true,checks:["manifest"]};});
  await act(async()=>root.render(<PluginWorkbench api={api} plugins={[{id:"demo",name:"示例",version:"1.0.0"} as never]}/>));await input(host.querySelectorAll("select")[1],"demo");await submit(host.querySelector("form")!);expect(host.textContent).toContain("合同不兼容");expect(host.textContent).not.toContain("检查通过：");fail=false;await submit(host.querySelector("form")!);expect(host.textContent).toContain("检查通过：");
});

it("曲库发现按需加载，可保存同一筛选规则而不调用LLM",async()=>{
 const api=new RMusicClient("http://collections"),request=vi.spyOn(api,"request").mockResolvedValue({items:[track],total:1,offset:0,truncated:false});
 await act(async()=>root.render(<DiscoveryCollections api={api} plugins={[]} play={async()=>{}} showDetails={()=>{}}/>));expect(request).not.toHaveBeenCalled();
 await click("高解析曲库");expect(host.textContent).toContain("测试歌曲");expect(JSON.parse(request.mock.calls[0][1]!.body as string).rule).toMatchObject({min_sample_rate:88200,min_bit_depth:24});
 await click("保存为智能歌单");expect(document.querySelector<HTMLInputElement>('input[type="number"][max="768000"]')!.value).toBe("88200");expect(request.mock.calls.every(([path])=>!path.startsWith("/llm"))).toBe(true);
});

it("修改智能规则后重新读取歌单详情，不复用旧规则的缓存页",async()=>{
 const api=new RMusicClient("http://smart-detail"),request=vi.spyOn(api,"request").mockResolvedValue({items:[track],total:1});
 const view=(q:string)=><LocalPlaylistView onChanged={()=>{}} api={api} playlist={{id:"smart-cache",name:"动态",track_ids:[],smart_query:{q}}} back={()=>{}} play={async()=>{}} enqueue={async()=>{}} showError={()=>{}} playAll={()=>{}}/>;
 await act(async()=>root.render(view("旧规则")));expect(request).toHaveBeenCalledTimes(1);
 await act(async()=>root.render(view("新规则")));expect(request).toHaveBeenCalledTimes(2);expect(host.textContent).toContain("测试歌曲");
});
