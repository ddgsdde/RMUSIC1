// @vitest-environment jsdom
import {act,useState} from "react";
import {createRoot} from "react-dom/client";
import {expect,it,vi} from "vitest";
import type {Route} from "../App";
import {ProfilePage,StudioShell} from "./StudioShell";

it("手机的我的音乐独立打开，旧皮肤的底栏设置替换为音乐入口",async()=>{
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
 const offline=vi.fn(),logout=vi.fn();
 function Mobile(){
  const [route,setRoute]=useState<Route>("discover");
  const [menu,setMenu]=useState(false);
  return <><StudioShell mobile layout="studio" route={route} user="listener" device="Android" online
    tabs={["discover","search","library-home","settings"]} query="" search={()=>{}} navigate={setRoute}
    menu={menu} setMenu={setMenu} offline={offline} logout={logout}/>
   {route==="profile"&&<main><ProfilePage user="listener" device="Android" online navigate={setRoute} offline={offline} logout={logout}/></main>}
   {route==="core-connection"&&<main>连接详情</main>}
  </>;
 }
 const host=document.createElement("div"),root=createRoot(host);
 try {
  await act(async()=>root.render(<Mobile/>));
  expect(host.querySelector('nav button[aria-label="设置"]')).toBeNull();
  expect(host.querySelectorAll("nav button")).toHaveLength(5);
  await act(async()=>host.querySelector<HTMLButtonElement>('button[aria-label="我的"]')!.click());
  expect(host.querySelector("main h1")?.textContent).toBe("我的音乐");
  expect(host.querySelector('[role="dialog"]')).toBeNull();
  const core=[...host.querySelectorAll<HTMLButtonElement>("main button")].find(button=>button.textContent?.startsWith("Core 连接"));
  await act(async()=>core!.click());
  expect(host.querySelector("main")?.textContent).toBe("连接详情");
 } finally {
  await act(async()=>root.unmount());vi.unstubAllGlobals();
 }
});
