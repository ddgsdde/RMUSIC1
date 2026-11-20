// @vitest-environment jsdom
import {act,createElement,Fragment} from "react";
import {createRoot} from "react-dom/client";
import {afterEach,expect,it,vi} from "vitest";
import {activeSkin,applySkin,builtins} from "./studio/skins";
import {mobileUI,sceneColor,switchMobileUI} from "./mobileAppearance";
import {readClientPreferences,saveClientPreferences} from "./clientPreferences";
import {shouldDismissPlayer,transitionPlayer,useAndroidPlayerSheet} from "./mobileMotion";
import type {Preferences} from "./App";
const defaults:Preferences={theme:"graphite",mode:"system",low_performance:false,custom_themes:[]};
afterEach(()=>{localStorage.clear();document.documentElement.removeAttribute("style");delete document.documentElement.dataset.mobileStyle;document.documentElement.removeAttribute("data-native-android");document.body.replaceChildren();Reflect.deleteProperty(document,"startViewTransition");vi.restoreAllMocks();vi.unstubAllGlobals();});
it("merges overlapping phone layouts without discarding the selected palette",()=>{
 expect(mobileUI(undefined)).toBe("studio");
 expect(mobileUI("compact")).toBe("studio");
 expect(mobileUI("liser")).toBe("studio");
 const previous={...defaults,theme:"liser-blue",mobile_ui:"classic" as const};
 const newLayout={...previous,...switchMobileUI(previous,"studio")};
 saveClientPreferences(newLayout);
 const restored=readClientPreferences(defaults);
 expect(restored.mobile_ui).toBe("studio");
 expect(activeSkin(restored).id).toBe("signal");
});
it("Android keeps the rounded Liser layout and the previous built-in palettes",()=>{
 const previous={...defaults,mobile_ui:"liser" as const,theme:"liser-violet"};
 saveClientPreferences(previous);
 const restored=readClientPreferences(defaults);
 expect(mobileUI(restored.mobile_ui,true)).toBe("liser");
 expect(mobileUI(restored.mobile_ui)).toBe("studio");
 expect(activeSkin(restored,true).dark.bg).toBe("#171321");
 expect(activeSkin(restored).id).toBe("signal");
 applySkin(restored,"dark",true,true);
 expect(document.documentElement.hasAttribute("data-native-android")).toBe(true);
 expect(document.documentElement.dataset.mobileStyle).toBe("liser");
 applySkin(restored,"light",false);
 expect(document.documentElement.hasAttribute("data-native-android")).toBe(false);
});
it("applies the record palette to browser chrome and preserves reduced atmosphere",()=>{
 expect(builtins.map(skin=>skin.id)).toEqual(["press","grove","signal"]);
 applySkin(defaults,"light",true);
 expect(document.documentElement.style.getPropertyValue("--bg")).toBe("#f8f6f0");
 expect(document.documentElement.style.getPropertyValue("--mobile-accent")).toBe("#b8412c");
 expect(document.documentElement.dataset.mobileStyle).toBe("studio");
 applySkin({...defaults,theme:"liser-emerald"},"dark");
 expect(document.documentElement.style.getPropertyValue("--bg")).toBe(builtins[1].dark.bg);
 expect(document.documentElement.dataset.mobileStyle).toBeUndefined();
 expect(sceneColor([1,.2,.1],"#f8f4fc",true,true)).toMatch(/^#[0-9a-f]{6}$/);
 expect(sceneColor([1,0,0],"#f8f4fc",true,false)).toBe("#f8f4fc");
});
it("reduced motion and unsupported view transitions navigate exactly once",()=>{
 vi.stubGlobal("matchMedia",()=>({matches:true}));const change=vi.fn();transitionPlayer(change,true);expect(change).toHaveBeenCalledTimes(1);
 expect(shouldDismissPlayer(20,2000,800)).toBe(false);expect(shouldDismissPlayer(190,200,800)).toBe(true);expect(shouldDismissPlayer(60,1600,800)).toBe(true);
});
it("snapshot transitions do not replay a second entrance animation after finishing",async()=>{
 vi.stubGlobal("matchMedia",()=>({matches:false}));
 const scene=document.createElement("div");scene.className="scene-player";document.body.append(scene);
 Object.defineProperty(document,"startViewTransition",{configurable:true,value:(update:()=>void)=>{update();return {finished:Promise.resolve()};}});
 const change=vi.fn();transitionPlayer(change,true);await new Promise(resolve=>setTimeout(resolve,0));
 expect(change).toHaveBeenCalledOnce();expect(scene.dataset.transitioned).toBe("true");expect(document.documentElement.dataset.playerTransition).toBeUndefined();
});

it("keeps the browsing surface mounted and reverses an in-flight Android player transition from its current position",async()=>{
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
 vi.stubGlobal("matchMedia",()=>({matches:false}));
 const host=document.createElement("div");
 document.body.append(host);
 const root=createRoot(host);
 let sheet!:ReturnType<typeof useAndroidPlayerSheet>;
 function Harness(){
  sheet=useAndroidPlayerSheet(false);
  return createElement(Fragment,null,
   createElement("main",{inert:sheet.presented},"library"),
   sheet.mounted&&createElement("div",{ref:sheet.layer,tabIndex:-1,"data-player-phase":sheet.phase},"player"));
 }
 try{
  await act(async()=>root.render(createElement(Harness)));
  const library=host.querySelector("main")!;
  await act(async()=>sheet.open());
  expect(sheet.phase).toBe("preparing");
  expect(library.hasAttribute("inert")).toBe(false);
  await act(async()=>sheet.close());
  expect(sheet.mounted).toBe(false);
  await act(async()=>sheet.ready());
  expect(sheet.mounted).toBe(false);

  await act(async()=>sheet.open());
  const layer=sheet.layer.current!;
  const animations:{finish:()=>void;cancel:ReturnType<typeof vi.fn>}[]=[];
  const animate=vi.fn((_frames:Keyframe[],_options:KeyframeAnimationOptions)=>{
   let finish!:()=>void;
   const finished=new Promise<void>(resolve=>{finish=resolve;});
   const cancel=vi.fn();animations.push({finish,cancel});
   return {finished,cancel} as unknown as Animation;
  });
  layer.animate=animate as typeof layer.animate;
  await act(async()=>sheet.ready());
  expect(sheet.phase).toBe("opening");
  expect(library.hasAttribute("inert")).toBe(true);
  expect(animate.mock.calls[0][0][0].transform).toBe("translate3d(0,100%,0)");
  layer.style.transform="matrix(1, 0, 0, 1, 0, 132)";
  await act(async()=>sheet.close());
  expect(animations[0].cancel).toHaveBeenCalledOnce();
  expect(animate.mock.calls[1][0][0].transform).toBe("matrix(1, 0, 0, 1, 0, 132)");
  await act(async()=>animations[0].finish());
  expect(sheet.phase).toBe("closing");
  await act(async()=>animations[1].finish());
  expect(sheet.mounted).toBe(false);
  expect(host.querySelector("main")).toBe(library);
  expect(library.hasAttribute("inert")).toBe(false);

  await act(async()=>sheet.open());
  const reopened=sheet.layer.current!;
  reopened.animate=animate as typeof reopened.animate;
  await act(async()=>sheet.ready());
  await act(async()=>animations[2].finish());
  expect(sheet.phase).toBe("open");
  reopened.style.setProperty("--player-drag-y","176px");
  await act(async()=>sheet.close());
  expect(animate.mock.calls[3][0][0].transform).toBe("translate3d(0,176px,0)");
  await act(async()=>animations[3].finish());
  expect(sheet.phase).toBe("closed");
 }finally{await act(async()=>root.unmount());host.remove();}
});
