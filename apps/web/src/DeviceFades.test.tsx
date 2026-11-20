// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect,it,vi} from "vitest";
import type {RMusicClient,Zone} from "@r-music/sdk";
import {PlaybackSettings} from "./PlaybackSettings";

it("改耳机的淡入只写入耳机，保留其淡出和音箱的设置",async()=>{
  const patch=vi.fn().mockResolvedValue({}),api={patch} as unknown as RMusicClient;
  const renderDevice=(id:string,head:number,tail:number)=><section data-device={id}><PlaybackSettings api={api} zone={{id,transitions:{fade_in_ms:head,fade_out_ms:tail}} as Zone} refresh={async()=>{}} onError={e=>{throw e}}/></section>;
  const host=document.createElement("div"),root=createRoot(host);
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
  try{
    await act(async()=>root.render(<>{renderDevice("speaker",250,500)}{renderDevice("headphones",0,1000)}</>));
    const fade=host.querySelector<HTMLSelectElement>('[data-device="headphones"] select')!;
    await act(async()=>{fade.value="2000";fade.dispatchEvent(new Event("change",{bubbles:true}));});
    expect(patch.mock.calls).toEqual([["/zones/headphones",{transitions:{fade_in_ms:2000,fade_out_ms:1000}}]]);
    expect(host.querySelector<HTMLSelectElement>('[data-device="speaker"] select')!.value).toBe("250");
  }finally{await act(async()=>root.unmount());vi.unstubAllGlobals();}
});
