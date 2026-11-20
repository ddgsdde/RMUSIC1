// @vitest-environment jsdom
import {afterEach,beforeEach,describe,expect,it,vi} from "vitest";
import {clearMocks,mockIPC} from "@tauri-apps/api/mocks";
import {NativeClient} from "./native";
const invoke=vi.fn<(command:string,args?:Record<string,unknown>)=>Promise<unknown>>();
function deferred<T>() {let resolve!:(value:T)=>void,reject!:(error:string)=>void;const promise=new Promise<T>((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};}
let sequence=0;
beforeEach(()=>{vi.clearAllMocks();mockIPC((command,args)=>invoke(command,args as Record<string,unknown>));});
afterEach(clearMocks);
function client(){return new NativeClient(`http://core-${++sequence}`);}
describe("原生读取的取消合同",()=>{
 it("响应上传使用专用原生命令，完整保留跨块的二进制字节",async()=>{
  const api=client(),bytes=Uint8Array.from({length:16387},(_,i)=>i%256),controller=new AbortController();
  invoke.mockImplementation(command=>Promise.resolve(command==="profiles_list"?[{id:"profile",address:api.baseUrl}]:{id:"impulse"}));
  await expect(api.importImpulse(bytes.buffer,"房间 A&右.wav",controller.signal)).resolves.toEqual({id:"impulse"});
  const upload=invoke.mock.calls.find(([command])=>command==="native_impulse_import")![1]!;expect(upload).toMatchObject({profileId:"profile",name:"房间 A&右.wav",requestId:expect.any(String)});
  expect(Uint8Array.from(atob(upload.data as string),c=>c.charCodeAt(0))).toEqual(bytes);expect(invoke.mock.calls.some(([command])=>command==="native_request")).toBe(false);
 });
 it("响应上传取消立即结束等待并发送取消标识，忽略迟到的401",async()=>{
  const api=client(),controller=new AbortController(),response=deferred<unknown>(),sent=deferred<void>();
  invoke.mockImplementation(command=>{if(command==="profiles_list")return Promise.resolve([{id:"profile",address:api.baseUrl}]);if(command==="native_impulse_import"){sent.resolve();return response.promise;}return Promise.resolve();});
  const pending=api.importImpulse(new ArrayBuffer(4),"test.wav",controller.signal),rejected=expect(pending).rejects.toMatchObject({name:"AbortError"});await sent.promise;controller.abort();await rejected;
  const upload=invoke.mock.calls.find(([command])=>command==="native_impulse_import")![1]!;await vi.waitFor(()=>expect(invoke).toHaveBeenCalledWith("native_request_cancel",{requestId:upload.requestId}));response.reject("HTTP 401: old session");
 });
 it("预先取消或超限的响应在连接前拒绝",async()=>{
  const api=client(),controller=new AbortController();controller.abort();await expect(api.importImpulse(new ArrayBuffer(4),undefined,controller.signal)).rejects.toMatchObject({name:"AbortError"});
  await expect(api.importImpulse(new ArrayBuffer(4*1024**2+1))).rejects.toThrow("4 MiB");expect(invoke).not.toHaveBeenCalled();
 });
 it("已取消的读取不进入原生IPC",async()=>{
  const api=client(),controller=new AbortController();controller.abort();
  vi.mocked(invoke).mockImplementation(command=>Promise.resolve(command==="profiles_list"?[{id:"profile",address:api.baseUrl}]:{}));
  await expect(api.request("/library/tracks",{signal:controller.signal})).rejects.toMatchObject({name:"AbortError"});expect(invoke).not.toHaveBeenCalled();
 });
 it("准备连接期间取消，不再发送旧查询",async()=>{
  const api=client(),controller=new AbortController(),profiles=deferred<unknown[]>();vi.mocked(invoke).mockReturnValue(profiles.promise);
  const request=api.request("/library/tracks",{signal:controller.signal});const rejected=expect(request).rejects.toMatchObject({name:"AbortError"});
  await Promise.resolve();controller.abort();profiles.resolve([{id:"profile",address:api.baseUrl}]);await rejected;
  expect(vi.mocked(invoke).mock.calls.map(([method])=>method)).toEqual(["profiles_list"]);
 });
 it("取消后迟到的原生401转为取消，不误判为当前登录过期",async()=>{
  const api=client(),controller=new AbortController(),response=deferred<unknown>(),sent=deferred<void>();
  vi.mocked(invoke).mockImplementation((command)=>{if(command==="profiles_list")return Promise.resolve([{id:"profile",address:api.baseUrl}]);sent.resolve();return response.promise;});
  const request=api.request("/library/tracks",{signal:controller.signal});const rejected=expect(request).rejects.toMatchObject({name:"AbortError"});
  await sent.promise;
  controller.abort();response.reject("HTTP 401: old core session expired");await rejected;
 });
 it("IPC 已发出后立即取消等待，并向 Rust 发送该请求的取消标识",async()=>{
  const api=client(),controller=new AbortController(),response=deferred<unknown>(),sent=deferred<void>();let settled=false;
  vi.mocked(invoke).mockImplementation(command=>{
   if(command==="profiles_list")return Promise.resolve([{id:"profile",address:api.baseUrl}]);
   if(command==="native_request"){sent.resolve();return response.promise;}
   return Promise.resolve();
  });
  const request=api.request("/library/tracks",{signal:controller.signal});request.catch(()=>{settled=true;});
  await sent.promise;controller.abort();for(let i=0;i<12;i++)await Promise.resolve();
  try{
   expect(settled).toBe(true);
   const args=vi.mocked(invoke).mock.calls.find(([command])=>command==="native_request")![1] as {requestId:string};
   expect(args.requestId).toEqual(expect.any(String));expect(args.requestId.length).toBeGreaterThan(0);
   await vi.waitFor(()=>expect(invoke).toHaveBeenCalledWith("native_request_cancel",{requestId:args.requestId}));
  }finally{response.resolve({});await request.catch(()=>{});}
 });
 it("并发读取只取消指定请求，成功完成后不再发取消命令",async()=>{
  const api=client(),first=new AbortController(),second=new AbortController(),a=deferred<unknown>(),b=deferred<unknown>(),sent=deferred<void>();let count=0;
  vi.mocked(invoke).mockImplementation(command=>{
   if(command==="profiles_list")return Promise.resolve([{id:"profile",address:api.baseUrl}]);
   if(command==="native_request"){if(++count===2)sent.resolve();return count===1?a.promise:b.promise;}
   return Promise.resolve();
  });
  const old=api.request("/library/tracks?search=a",{signal:first.signal});const cancelled=expect(old).rejects.toMatchObject({name:"AbortError"});
  const current=api.request("/library/tracks?search=b",{signal:second.signal});await sent.promise;first.abort();a.resolve({old:true});b.resolve({items:["b"]});
  await cancelled;await expect(current).resolves.toEqual({items:["b"]});second.abort();for(let i=0;i<5;i++)await Promise.resolve();
  const reads=vi.mocked(invoke).mock.calls.filter(([command])=>command==="native_request").map(([,args])=>args as {requestId:string});
  expect(reads[0].requestId).not.toBe(reads[1].requestId);await vi.waitFor(()=>expect(vi.mocked(invoke).mock.calls.filter(([command])=>command==="native_request_cancel")).toEqual([["native_request_cancel",{requestId:reads[0].requestId}]]));
 });
 it("未取消的当前错误与成功结果保持不变",async()=>{
  const api=client();vi.mocked(invoke).mockImplementation(command=>command==="profiles_list"?Promise.resolve([{id:"profile",address:api.baseUrl}]):Promise.reject("HTTP 401: current session expired"));
  await expect(api.request("/library/tracks")).rejects.toMatchObject({status:401,code:"native_http"});
  vi.mocked(invoke).mockResolvedValue({items:[],total:0});await expect(api.request("/library/tracks")).resolves.toEqual({items:[],total:0});
 });
});
