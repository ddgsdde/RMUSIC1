import runtimeURL from "./engine/wasm_exec.js?url";
import engineURL from "./engine/core-tls.wasm.gz?url";

interface Engine {
  begin(id:number, socket:string, host:string, key:string, method:string, path:string, headers:string[][], reader:ReadableStreamDefaultReader<Uint8Array>|null):Promise<{status:number; headers:[string,string][]}>;
  read(id:number):Promise<Uint8Array<ArrayBuffer>|null>;
  close(id:number):void;
}
declare global {
  interface Window {
    Go: new()=>{ importObject:WebAssembly.Imports; run(instance:WebAssembly.Instance):Promise<void> };
    rmusicTLS?:Engine;
  }
}
let loading:Promise<Engine>|undefined;
function engine():Promise<Engine> {
  if(loading)return loading;
  loading=(async()=>{
    if(!window.Go)await new Promise<void>((resolve,reject)=>{
      const s=document.createElement("script");s.src=runtimeURL;s.onload=()=>resolve();s.onerror=()=>reject(new Error("加密组件加载失败，请刷新"));document.head.append(s);
    });
    const response=await fetch(engineURL,{credentials:"omit",redirect:"error",cache:"force-cache"});
    if(!response.ok||!response.body)throw new Error("加密组件下载失败，请重试");
    const bytes=await new Response(response.body.pipeThrough(new DecompressionStream("gzip"))).arrayBuffer();
    const go=new window.Go();
    const result=await WebAssembly.instantiate(bytes,go.importObject);
    void go.run(result.instance).catch(()=>{loading=undefined;});
    if(!window.rmusicTLS)throw new Error("浏览器未能启动加密连接");
    return window.rmusicTLS;
  })().catch(e=>{loading=undefined;throw e;});
  return loading;
}

type Identity={core_id:string; core_public_key:string};
const identities=new Map<string,Promise<Identity>>();
function identity(root:string):Promise<Identity>{
  const saved=identities.get(root);if(saved)return saved;
  const job=(async()=>{
    const r=await fetch(root+"/api/v1/status",{credentials:"omit",redirect:"error",cache:"no-store",signal:AbortSignal.timeout(15000)});
    if(!r.ok)throw new Error("无法读取 Core 身份");
    const v=await r.json();
    if(!v.capabilities?.native_e2e_v1||!/^[a-f0-9-]{36}$/i.test(v.core_id)||! /^[A-Za-z0-9_-]{43}$/.test(v.core_public_key))throw new Error("此 Core 未提供加密登录，请更新 Core 或使用 HTTPS");
    const current={core_id:v.core_id as string,core_public_key:v.core_public_key as string};
    const pin=localStorage.getItem("rmusic.tls-pin:"+root);
    if(pin&&pin!==JSON.stringify(current))throw new Error("此地址的 Core 身份已变化，请通过已验证的 HTTPS 地址重新连接");
    return current;
  })().catch(e=>{identities.delete(root);throw e;});
  identities.set(root,job);return job;
}

let next=0;
export async function coreFetch(base:string, target:URL, options:RequestInit):Promise<Response>{
  const root=new URL(base||".",document.baseURI);
  const prefix=root.pathname.replace(/\/$/,"");
  if(target.origin!==root.origin||!target.pathname.startsWith(prefix+"/api/v1/"))throw new Error("无效的 Core 请求地址");
  if(root.protocol==="https:")return fetch(target,{...options,redirect:"error"});
  if(root.protocol!=="http:")throw new Error("无效的 Core 连接协议");
  const headers=new Headers(options.headers);
  const path=target.pathname.slice(prefix.length)+target.search;
  if(path==="/api/v1/status"&&!headers.has("authorization")&&(!options.method||options.method==="GET")){
    void engine().catch(()=>{}); // Start the one-time download while the user enters their credentials.
    return fetch(target,{...options,credentials:"omit",redirect:"error"});
  }
  const rootURL=root.origin+prefix;
  const [tls,peer]=await Promise.all([engine(),identity(rootURL)]);
  options.signal?.throwIfAborted();
  const key="rmusic.tls-pin:"+rootURL, serialized=JSON.stringify(peer), pinned=localStorage.getItem(key);
  if(pinned&&pinned!==serialized)throw new Error("Core 身份不匹配，已停止连接");
  const id=++next,request=new Request(target,options),signal=request.signal;
  const abort=()=>tls.close(id);
  const release=()=>{signal.removeEventListener("abort",abort);tls.close(id);};
  signal.addEventListener("abort",abort,{once:true});
  try{
    const fields:string[][]=[];request.headers.forEach((v,k)=>fields.push([k,v]));
    const ws=new URL(rootURL+"/api/v1/native/connect");ws.protocol="ws:";
    const meta=await tls.begin(id,ws.href,"core-"+peer.core_id+".rmusic.invalid",peer.core_public_key,request.method,path,fields,request.body?.getReader()??null);
    signal.throwIfAborted();
    if(!pinned)localStorage.setItem(key,serialized);
    let body:ReadableStream<Uint8Array<ArrayBuffer>>|null=null;
    if(request.method==="HEAD"||[204,205,304].includes(meta.status))release();
    else body=new ReadableStream({
      async pull(controller){
        try{const bytes=await tls.read(id);if(bytes===null){signal.removeEventListener("abort",abort);controller.close();}else controller.enqueue(bytes);}
        catch(e){release();controller.error(e);}
      },
      cancel:release,
    });
    return new Response(body,{status:meta.status,headers:meta.headers});
  }catch(e){release();throw e;}
}

