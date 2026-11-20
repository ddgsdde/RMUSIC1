// @vitest-environment jsdom
import {act} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {NativeStorageSettings} from "./NativeStorageSettings";
const mocks=vi.hoisted(()=>({invoke:vi.fn(),listen:vi.fn(),event:undefined as undefined|((event:{payload:Record<string,unknown>})=>void)}));
vi.mock("./native",()=>({invokeNative:mocks.invoke}));
vi.mock("@tauri-apps/api/event",()=>({listen:mocks.listen}));
let host:HTMLDivElement,root:Root;
beforeEach(()=>{vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);vi.clearAllMocks();host=document.createElement("div");document.body.append(host);root=createRoot(host);mocks.listen.mockImplementation(async(_:string,event:any)=>{mocks.event=event;return()=>{};});});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();});
const button=(text:string)=>Array.from(host.querySelectorAll('button')).find(b=>b.textContent===text)!;
it("取消目录选择保留原位置且不触发迁移",async()=>{
 const notify=vi.fn(),changed=vi.fn();mocks.invoke.mockImplementation(async(command:string)=>command==='native_storage_choose'?{cancelled:true}:{name:'原目录',tree:'content://old',available:true,pending:0});
 await act(async()=>root.render(<NativeStorageSettings notify={notify} onError={vi.fn()} changed={changed}/>));
 await act(async()=>button('选择目录').click());expect(host.textContent).toContain('原目录');expect(notify).not.toHaveBeenCalled();expect(mocks.invoke.mock.calls.some(([command])=>command==='native_storage_action')).toBe(false);
});
it("目录失权保留位置与恢复入口，无需连接Core",async()=>{
 let state={name:'SD卡音乐',tree:'content://sd',available:false,pending:2,error:'离线目录授权已失效'};
 mocks.invoke.mockImplementation(async(command:string)=>{if(command==='native_storage_choose'){state={...state,available:true,error:''};return{cancelled:false};}return state;});
 await act(async()=>root.render(<NativeStorageSettings notify={vi.fn()} onError={vi.fn()} changed={vi.fn()}/>));
 expect(host.querySelector('[role=alert]')?.textContent).toContain('原文件与下载记录保留');await act(async()=>button('重新授权目录').click());expect(host.textContent).toContain('SD卡音乐');expect(host.querySelector('[role=alert]')).toBeNull();expect(mocks.invoke.mock.calls.every(([command])=>String(command).startsWith('native_storage_'))).toBe(true);
});
it("迁移进度不重复读取文件清单，完成后更新缓存页",async()=>{
 mocks.invoke.mockResolvedValue({name:'音乐',tree:'content://music',available:true,pending:1});const changed=vi.fn();
 await act(async()=>root.render(<NativeStorageSettings notify={vi.fn()} onError={vi.fn()} changed={changed}/>));
 const reads=mocks.invoke.mock.calls.length;
 await act(async()=>mocks.event?.({payload:{busy:true,file:'六声道测试',received:128,total:256}}));expect(host.textContent).toContain('六声道测试');expect(mocks.invoke.mock.calls.length).toBe(reads);expect(button('选择目录').disabled).toBe(true);
 await act(async()=>mocks.event?.({payload:{busy:false,moved:1}}));expect(changed).toHaveBeenCalledTimes(1);expect(button('选择目录').disabled).toBe(false);
});
