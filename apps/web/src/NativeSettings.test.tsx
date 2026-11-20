// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {it,expect,vi} from "vitest";
import {RMusicClient} from "@r-music/sdk";
import {NativeSettings} from "./NativeSettings";
vi.mock("./native",()=>({nativeOutput:()=>({device_id:"asio:QA",exclusive:true,buffer_ms:20,dsd_output:"native"}),invokeNative:async()=>[{id:"asio:QA",name:"ASIO QA"}]}));
it("saves explicit ASIO native DSD and rejects saving it with a non-ASIO device",async()=>{
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);localStorage.clear();
 const host=document.createElement("div");document.body.append(host);const root=createRoot(host),notify=vi.fn();
 try {
  await act(async()=>root.render(<NativeSettings api={new RMusicClient("http://test")} onError={error=>{throw error;}} notify={notify}/>));
  const [device,dsd]=host.querySelectorAll("select");
  expect(dsd.value).toBe("native");expect(dsd.querySelector<HTMLOptionElement>('[value="native"]')!.disabled).toBe(false);
  await act(async()=>host.querySelector<HTMLButtonElement>("button.primary")!.click());
  expect(JSON.parse(localStorage.getItem("rmusic.output")!)).toMatchObject({device_id:"asio:QA",dsd_output:"native",exclusive:true});
  await act(async()=>{device.value="";device.dispatchEvent(new Event("change",{bubbles:true}));});
  expect(dsd.querySelector<HTMLOptionElement>('[value="native"]')!.disabled).toBe(true);
  await act(async()=>host.querySelector<HTMLButtonElement>("button.primary")!.click());
  expect(notify).toHaveBeenLastCalledWith("原生 DSD 需要选择具体的 ASIO 驱动");
  expect(JSON.parse(localStorage.getItem("rmusic.output")!).device_id).toBe("asio:QA");
 } finally {await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();}
});
