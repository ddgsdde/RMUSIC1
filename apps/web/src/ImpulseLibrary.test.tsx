// @vitest-environment jsdom
import {act} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {ApiError,type ImpulseAssets,type RMusicClient,type Zone} from "@r-music/sdk";
import {ImpulseLibrary} from "./ImpulseLibrary";
import {DspEditor} from "./DspWorkbench";
let root:Root,host:HTMLDivElement;
const asset={id:"a".repeat(64),name:"房间 A&右.wav",bytes:1024,sample_rate:48000,channels:2,frames:65,channel_mask:3,created_at:1};
const library=(items:ImpulseAssets["items"]=[]):ImpulseAssets=>({items,limits:{max_files:128,max_storage_bytes:256*1024**2,max_upload_bytes:4*1024**2}});
function client(items:ImpulseAssets["items"]=[]) {return {listImpulses:vi.fn().mockResolvedValue(library(items)),importImpulse:vi.fn().mockResolvedValue(asset),deleteImpulse:vi.fn().mockResolvedValue(undefined),put:vi.fn()} as unknown as RMusicClient;}
function deferred<T>(){let resolve!:(value:T)=>void,reject!:(error:Error)=>void;const promise=new Promise<T>((yes,no)=>{resolve=yes;reject=no;});return{promise,resolve,reject};}
beforeEach(()=>{vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();});
async function render(api:RMusicClient,onError=vi.fn()){await act(async()=>root.render(<ImpulseLibrary api={api} onError={onError}/>));}
async function click(label:string){const button=[...host.querySelectorAll<HTMLButtonElement>("button")].find(value=>value.textContent===label||value.getAttribute("aria-label")===label);expect(button).toBeTruthy();await act(async()=>button!.click());}
async function select(file:File){const input=host.querySelector<HTMLInputElement>('input[aria-label="导入 WAV 响应"]')!;Object.defineProperty(input,"files",{value:[file],configurable:true});await act(async()=>input.dispatchEvent(new Event("change",{bubbles:true})));}
function file(bytes=new Uint8Array([0,255,128,10]).buffer){const file=new File([],asset.name,{type:"audio/wav"});Object.defineProperty(file,"size",{value:bytes.byteLength,configurable:true});Object.defineProperty(file,"arrayBuffer",{value:vi.fn().mockResolvedValue(bytes),configurable:true});return file;}
it("imports original bytes and deduplicates metadata without submitting the surrounding DSP form",async()=>{
 const api=client([asset]),saved=vi.fn(),bytes=new Uint8Array([0,255,128,10]).buffer;
 const zone:Zone={id:"zone",name:"QA",endpoint_id:"browser-qa",status:"stopped",volume:1,muted:false,repeat:"off",shuffle:false,position:0,track:null,queue:[],queue_index:0,updated_at:0,dsp:{enabled:false,preamp_db:0,balance:0,normalize:false,target_lufs:-14,bands:[]}};
 await act(async()=>root.render(<DspEditor api={api} zone={zone} onSaved={saved} onError={vi.fn()}/>));
 await act(async()=>{const list=host.querySelector<HTMLSelectElement>('select[aria-label="添加模块"]')!;list.value="convolution";list.dispatchEvent(new Event("change",{bubbles:true}));});
 await select(file(bytes));expect(api.importImpulse).toHaveBeenCalledWith(bytes,asset.name,expect.any(AbortSignal));expect(host.querySelectorAll(".impulse-list li")).toHaveLength(1);expect(host.textContent).toContain("48 kHz · 2 声道 · 65 采样点");expect(host.textContent).toContain("保留响应原始增益");expect(host.textContent).toContain("该输出尚不能应用响应");
 await click(`删除响应 ${asset.name}`);expect(api.deleteImpulse).toHaveBeenCalledWith(asset.id,expect.any(AbortSignal));expect(host.textContent).toContain("尚未导入响应文件");expect(api.put).not.toHaveBeenCalled();expect(saved).not.toHaveBeenCalled();
});
it("keeps a referenced resource after deletion rejection and allows a later retry",async()=>{
 const api=client([asset]);vi.mocked(api.deleteImpulse).mockRejectedValueOnce(new ApiError(409,"impulse_in_use","请先从音区移除此脉冲响应"));await render(api);await click(`删除响应 ${asset.name}`);
 expect(host.querySelector('[role="alert"]')?.textContent).toContain("请先从音区移除");expect(host.querySelectorAll(".impulse-list li")).toHaveLength(1);await click(`删除响应 ${asset.name}`);expect(host.querySelectorAll(".impulse-list li")).toHaveLength(0);
});
it("explains unsupported cores and refreshes after recovery",async()=>{
 const api=client();vi.mocked(api.listImpulses).mockRejectedValueOnce(new ApiError(404,"not_found","missing"));await render(api);expect(host.textContent).toContain("请升级核心后重试");expect([...host.querySelectorAll('button')].find(value=>value.textContent==="导入 WAV")?.disabled).toBe(true);
 await click("刷新响应文件");expect(host.textContent).toContain("尚未导入响应文件");expect(host.querySelector('[role="alert"]')).toBeNull();
});
it("rejects oversized files before reading their content",async()=>{
 const api=client(),selected=file();Object.defineProperty(selected,"size",{value:4*1024**2+1,configurable:true});await render(api);await select(selected);expect(host.textContent).toContain("响应文件不能超过 4.0 MiB");expect(selected.arrayBuffer).not.toHaveBeenCalled();expect(api.importImpulse).not.toHaveBeenCalled();
});
it("switching cores aborts the old list and ignores its late result",async()=>{
 const old=client(),current=client(),pending=deferred<ImpulseAssets>();vi.mocked(old.listImpulses).mockReturnValue(pending.promise);await render(old);const signal=vi.mocked(old.listImpulses).mock.calls[0][0]!;await render(current);expect(signal.aborted).toBe(true);
 await act(async()=>pending.resolve(library([asset])));expect(host.textContent).not.toContain(asset.name);expect(host.textContent).toContain("尚未导入响应文件");
});
it("switching cores while a file is read prevents its upload",async()=>{
 const old=client(),current=client(),pending=deferred<ArrayBuffer>(),selected=file();Object.defineProperty(selected,"arrayBuffer",{value:vi.fn().mockReturnValue(pending.promise)});await render(old);await select(selected);await render(current);
 await act(async()=>pending.resolve(new ArrayBuffer(4)));expect(old.importImpulse).not.toHaveBeenCalled();expect(current.importImpulse).not.toHaveBeenCalled();expect(host.textContent).not.toContain("已导入");
});
it("unmount aborts an active delete and ignores a late auth error",async()=>{
 const api=client([asset]),pending=deferred<void>(),onError=vi.fn();vi.mocked(api.deleteImpulse).mockReturnValue(pending.promise);await render(api,onError);await click(`删除响应 ${asset.name}`);const signal=vi.mocked(api.deleteImpulse).mock.calls[0][1]!;
 await act(async()=>root.render(<div/>));expect(signal.aborted).toBe(true);await act(async()=>pending.reject(new ApiError(401,"unauthorized","old session")));expect(onError).not.toHaveBeenCalled();
});

it("starts with one EQ, keeps singletons unique, and saves ordered EQ modules",async()=>{
  const api=client(),saved=vi.fn();
  vi.mocked(api.put).mockResolvedValue(undefined);
  const zone:Zone={id:"chain",name:"QA",endpoint_id:"bridge",status:"stopped",volume:0,muted:true,repeat:"off",shuffle:false,position:0,track:null,queue:[],queue_index:0,updated_at:0,
    dsp:{enabled:true,preamp_db:0,balance:0,normalize:false,target_lufs:-14,bands:[{enabled:true,frequency:400,gain_db:2,q:1}]}};
  await act(async()=>root.render(<DspEditor api={api} zone={zone} advanced modular capabilities={{core_pcm_dsp_v1:true}} onSaved={saved} onError={vi.fn()}/>));
  const dropdown=()=>host.querySelector<HTMLSelectElement>('select[aria-label="添加模块"]')!;
  const add=async(name:string)=>act(async()=>{const input=dropdown();input.value=name;input.dispatchEvent(new Event("change",{bubbles:true}));});
  expect(host.querySelectorAll(".dsp-module-list button")).toHaveLength(1);
  await add("gain");
  expect(dropdown().querySelector('option[value="gain"]')).toBeNull();
  await add("eq");
  await click("添加频段");
  await click("上移 EQ");
  await click("应用到音区");
  expect(api.put).toHaveBeenLastCalledWith("/zones/chain/dsp",expect.objectContaining({
    bands:[expect.objectContaining({frequency:1000})],extra_eq:[expect.objectContaining({bands:[expect.objectContaining({frequency:400})]})],
  }));
  await click("移除模块");
  await click("应用到音区");
  expect(api.put).toHaveBeenLastCalledWith("/zones/chain/dsp",expect.objectContaining({
    bands:[expect.objectContaining({frequency:400})],extra_eq:undefined,
  }));
});
