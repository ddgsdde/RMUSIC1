// @vitest-environment jsdom
import {act} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {ApiError,RMusicClient,type Zone,type OutputGroup} from "@r-music/sdk";
import {SavedOutputGroups} from "./SavedOutputGroups";
let host:HTMLDivElement,root:Root;const zones=["a","b"].map(id=>({id,name:"房间 "+id,endpoint_id:id})) as Zone[];
async function click(text:string){const button=[...host.querySelectorAll("button")].find(b=>b.textContent===text||b.getAttribute("aria-label")===text)!;expect(button).toBeDefined();await act(async()=>button.click());}
async function input(value:string){await act(async()=>{const field=host.querySelector<HTMLInputElement>('input[aria-label="组合名称"]')!;Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(field,value);field.dispatchEvent(new Event("input",{bubbles:true}));});}
async function submit(){await act(async()=>host.querySelector("form")!.dispatchEvent(new Event("submit",{bubbles:true,cancelable:true})));}
beforeEach(()=>{vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();});
it("保存响应丢失后使用相同标识和版本重试，选择组合只传递音区",async()=>{
 const api=new RMusicClient("http://qa","synthetic"),choose=vi.fn(),puts:{path:string;body:any}[]=[];let saved:OutputGroup|undefined,first=true;
 vi.spyOn(api,"request").mockImplementation(async(path,options)=>{if(path==="/output-groups")return {items:saved?[saved]:[]};const body=JSON.parse(options!.body as string);puts.push({path,body});saved={id:path.split("/").at(-1)!,name:body.name,zone_ids:body.zone_ids,revision:1,updated_at:1};if(first){first=false;throw new TypeError("lost response");}return saved;});
 await act(async()=>root.render(<SavedOutputGroups api={api} zones={zones} canChoose onChoose={choose}/>));await click("常用音区组合");await click("新建组合");await input("客厅组合");await act(async()=>{host.querySelectorAll<HTMLInputElement>('.group-room input').forEach(input=>input.click());});await submit();expect(host.textContent).toContain("尚未收到操作结果");await submit();expect(puts).toHaveLength(2);expect(puts[1]).toEqual(puts[0]);expect(puts[0].body.expected_revision).toBe(0);await click("选择此组合");expect(choose).toHaveBeenCalledWith(["a","b"]);expect(puts.every(call=>call.path.startsWith("/output-groups/"))).toBe(true);
});
it("冲突刷新保留编辑输入，缺失音区不能选择，删除组合不启动播放",async()=>{
 const api=new RMusicClient("http://qa","synthetic"),choose=vi.fn();let group={id:"g",name:"原组合",zone_ids:["a","missing"],revision:1,updated_at:1};
 const request=vi.spyOn(api,"request").mockImplementation(async(path,options)=>{if(path==="/output-groups")return {items:[group]};if(options?.method==="PUT")throw new ApiError(409,"group_conflict","conflict");if(options?.method==="DELETE")return;throw new Error(path);});
 await act(async()=>root.render(<SavedOutputGroups api={api} zones={zones} canChoose onChoose={choose}/>));await click("常用音区组合");expect([...host.querySelectorAll("button")].find(b=>b.textContent==="选择此组合")!.disabled).toBe(true);await click("编辑组合 原组合");await input("本机草稿");await act(async()=>host.querySelectorAll<HTMLInputElement>('.group-room input').forEach(input=>{if(input.parentElement?.textContent?.includes("已移除")||input.parentElement?.textContent?.includes("房间 b"))input.click();}));await submit();expect(host.textContent).toContain("当前输入已保留");group={...group,name:"其他控制端",revision:2};await click("刷新列表");expect(host.querySelector<HTMLInputElement>('input[aria-label="组合名称"]')!.value).toBe("本机草稿");expect(host.textContent).toContain("其他控制端");await click("删除组合 其他控制端");expect(host.textContent).not.toContain("其他控制端");expect(choose).not.toHaveBeenCalled();expect(request.mock.calls.every(([path])=>path.startsWith("/output-groups"))).toBe(true);
});
it("旧 Core 列表迟到不会覆盖当前 Core 的组合",async()=>{
 const old=new RMusicClient("http://old-core","synthetic"),api=new RMusicClient("http://new-core","synthetic"),choose=vi.fn();let release:(value:any)=>void=()=>{};
 vi.spyOn(old,"request").mockImplementation(()=>new Promise(resolve=>{release=resolve;}));vi.spyOn(api,"request").mockResolvedValue({items:[{id:"new",name:"当前 Core 组合",zone_ids:["a","b"],revision:1,updated_at:1}]});
 await act(async()=>root.render(<SavedOutputGroups api={old} zones={zones} canChoose onChoose={choose}/>));await click("常用音区组合");await act(async()=>root.render(<SavedOutputGroups api={api} zones={zones} canChoose onChoose={choose}/>));await act(async()=>release({items:[{id:"old",name:"旧 Core 组合",zone_ids:["a","b"],revision:1,updated_at:1}]}));expect(host.textContent).toContain("当前 Core 组合");expect(host.textContent).not.toContain("旧 Core 组合");
});
