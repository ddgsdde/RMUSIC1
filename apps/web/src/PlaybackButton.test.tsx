// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect,it,vi} from "vitest";
import {PlaybackButton} from "./PlaybackButton";

it("keeps the same icon during a short transport request and only spins for sustained buffering",async()=>{
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
 vi.useFakeTimers();
 const host=document.createElement("div"),root=createRoot(host);
 const command=vi.fn(async()=>{});
 const render=(playing:boolean,busy:boolean,waiting:boolean)=>act(async()=>{
  root.render(<PlaybackButton playing={playing} busy={busy} waiting={waiting} disabled={false} command={command}/>);
 });
 const tick=(ms:number)=>act(async()=>vi.advanceTimersByTime(ms));
 try {
  await render(false,false,false);
  const button=host.querySelector("button")!;
  const initialIcon=button.querySelector("svg");
  await render(false,true,false);
  expect(button.disabled).toBe(true);
  expect(button.querySelector("svg")).toBe(initialIcon);
  await render(false,true,true);
  await tick(100);
  expect(button.querySelector("svg")).toBe(initialIcon);
  await render(true,false,false);
  expect(button.getAttribute("aria-label")).toBe("暂停");
  expect(button.querySelector(".spin")).toBeNull();
  await render(false,false,true);
  await tick(200);
  expect(button.querySelector(".spin")).not.toBeNull();
  button.click();
  expect(command).toHaveBeenLastCalledWith("pause");
 } finally {
  await act(async()=>root.unmount());
  vi.useRealTimers();
  vi.unstubAllGlobals();
 }
});
