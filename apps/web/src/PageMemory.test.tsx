// @vitest-environment jsdom
import {act,useEffect,useState} from "react";
import {createRoot,type Root} from "react-dom/client";
import {beforeEach,afterEach,it,expect,vi} from "vitest";
import {PageMemory} from "./PageMemory";
let host:HTMLDivElement,root:Root;const active=new Set<string>();
function Page({id}:{id:string}){const[value,setValue]=useState(0);useEffect(()=>{active.add(id);return()=>{active.delete(id);};},[id]);return <button onClick={()=>setValue(v=>v+1)}>{id}:{value}</button>;}
beforeEach(()=>{active.clear();vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);vi.stubGlobal("matchMedia",()=>({matches:true}));host=document.createElement("div");host.className="main-content";document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();});
async function render(id:string,scope="account-a"){await act(async()=>root.render(<PageMemory page={id} scope={scope} active cacheable><Page id={id}/></PageMemory>));}
it("返回页面恢复状态与滚动位置，隐藏页面停止 Effects",async()=>{
 await render("library");await act(async()=>host.querySelector("button")!.click());host.scrollTop=250;await render("stats");expect(active).toEqual(new Set(["stats"]));host.scrollTop=30;await render("library");expect(active).toEqual(new Set(["library"]));expect([...host.querySelectorAll("button")].find(b=>b.textContent==="library:1")).toBeTruthy();expect(host.scrollTop).toBe(250);
});
it("账号改变清空页面，最近页面最多保留三个",async()=>{
 for(const id of ["a","b","c","d"])await render(id);expect(host.querySelectorAll("button")).toHaveLength(3);expect(host.textContent).not.toContain("a:0");await render("d","account-b");expect(host.querySelectorAll("button")).toHaveLength(1);
});
