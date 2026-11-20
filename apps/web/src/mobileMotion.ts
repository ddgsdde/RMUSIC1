import {flushSync} from "react-dom";
import {useCallback,useLayoutEffect,useRef,useState,type PointerEvent as ReactPointerEvent} from "react";

/** Gestures and transitions only affect presentation. */
export function transitionPlayer(change:()=>void,enabled:boolean) {
 const doc=document as Document&{startViewTransition?:(update:()=>void)=>{finished:Promise<void>}};
 if(!enabled||matchMedia("(prefers-reduced-motion: reduce)").matches||!doc.startViewTransition){change();return;}
 let applied=false;
 document.documentElement.dataset.playerTransition="true";
 try {doc.startViewTransition(()=>{applied=true;flushSync(change);document.querySelector<HTMLElement>(".scene-player")?.setAttribute("data-transitioned","true");}).finished.catch(()=>{}).finally(()=>{delete document.documentElement.dataset.playerTransition;});}
 catch {delete document.documentElement.dataset.playerTransition;if(!applied)change();}
}
type PlayerPhase="closed"|"preparing"|"opening"|"open"|"closing";
const sheetEnter="cubic-bezier(.22,.72,.22,1)";
const sheetExit="cubic-bezier(.38,0,.75,.68)";
const belowScreen="translate3d(0,100%,0)";
const atRest="translate3d(0,0,0)";

export function useAndroidPlayerSheet(quiet:boolean) {
 const [phase,setPhase]=useState<PlayerPhase>("closed");
 const layer=useRef<HTMLDivElement>(null),motion=useRef<Animation|null>(null),origin=useRef<HTMLElement|null>(null);
 const restoreFocus=useRef(false),exitTransform=useRef<string>(undefined);
 const open=useCallback(()=>{
  const active=document.activeElement;
  origin.current=active instanceof HTMLElement&&active.closest(".transport")?active:document.querySelector<HTMLElement>(".transport .now-playing-info");
  if(active instanceof HTMLElement&&active.matches("input,textarea,[contenteditable]"))active.blur();
  setPhase(current=>current==="closed"?"preparing":current);
 },[]);
 const ready=useCallback(()=>setPhase(current=>current==="preparing"?"opening":current),[]);
 const close=useCallback(()=>{
  const node=layer.current;
  if(node?.dataset.playerPhase==="opening")exitTransform.current=getComputedStyle(node).transform;
  else if(node?.dataset.playerPhase==="open")exitTransform.current=node.style.getPropertyValue("--player-drag-y")
    ? `translate3d(0,${node.style.getPropertyValue("--player-drag-y")},0)` : atRest;
  setPhase(current=>current==="preparing"?"closed":current==="opening"||current==="open"?"closing":current);
 },[]);
 const reset=useCallback(()=>{
  motion.current?.cancel();motion.current=null;restoreFocus.current=false;origin.current=null;
  setPhase("closed");
 },[]);
 useLayoutEffect(()=>()=>{motion.current?.cancel();motion.current=null;},[]);
 useLayoutEffect(()=>{
  const node=layer.current;
  if(phase==="closed"){
   motion.current?.cancel();motion.current=null;
   if(restoreFocus.current&&origin.current?.isConnected)origin.current.focus({preventScroll:true});
   restoreFocus.current=false;origin.current=null;exitTransform.current=undefined;return;
  }
  if(!node||phase!=="opening"&&phase!=="closing")return;
  const closing=phase==="closing";
  const start=closing?(exitTransform.current||atRest):belowScreen;
  motion.current?.cancel();motion.current=null;
  const finish=()=>{
   if(closing)restoreFocus.current=true;
   setPhase(current=>current===phase?(closing?"closed":"open"):current);
  };
  if(!closing)node.focus({preventScroll:true});
  if(quiet||document.documentElement.dataset.skinMotion==="quiet"||matchMedia("(prefers-reduced-motion: reduce)").matches||!node.animate){finish();return;}
  const animation=node.animate([{transform:start},{transform:closing?belowScreen:atRest}],
   {duration:closing?260:330,easing:closing?sheetExit:sheetEnter,fill:"both"});
  motion.current=animation;
  void animation.finished.then(()=>{
   if(motion.current!==animation)return;
   finish();
   if(!closing){animation.cancel();motion.current=null;}
  },()=>{});
 },[phase,quiet]);
 return {phase,layer,open,ready,close,reset,mounted:phase!=="closed",presented:phase!=="closed"&&phase!=="preparing"};
}
export function shouldDismissPlayer(distance:number,velocity:number,height:number) {
 return distance>Math.max(80,height*.22)||distance>24&&velocity>height*1.5;
}
export function dismissQueue(change:()=>void,enabled:boolean) {
 const panel=document.querySelector<HTMLElement>(".queue-panel");
 if(!enabled||!panel?.animate||matchMedia("(prefers-reduced-motion: reduce)").matches){change();return;}
 if(panel.dataset.closing)return;panel.dataset.closing="true";panel.style.pointerEvents="none";
 const android=!!panel.closest('[data-android-player="true"]');
 const start=android?getComputedStyle(panel).transform:"translateY(0)";
 if(android)panel.style.animation="none";
 panel.animate([{transform:start,opacity:1},{transform:belowScreen,opacity:android?1:.4}],
  {duration:android?260:250,easing:android?sheetExit:"cubic-bezier(.215,.61,.355,1)",fill:"forwards"}).finished.catch(()=>{}).finally(change);
}
export function usePlayerDismiss(enabled:boolean,dismiss:()=>void) {
 const drag=useRef<{y:number;x:number;t:number;node:HTMLElement}|null>(null);
 const finish=(event:ReactPointerEvent<HTMLElement>,cancelled=false)=>{
  const current=drag.current;if(!current)return;drag.current=null;
  const dy=Math.max(0,event.clientY-current.y),elapsed=Math.max(1,performance.now()-current.t);
  const commit=!cancelled&&shouldDismissPlayer(dy,dy/elapsed*1000,innerHeight);
  if(commit&&current.node.classList.contains("android-player-sheet")){
   current.node.style.setProperty("--player-drag-y",`${dy}px`);
   dismiss();
   return;
  }
  delete current.node.dataset.dragging;current.node.style.setProperty("--player-drag-y","0px");
  if(commit)dismiss();
 };
 return {onPointerDown:(event:ReactPointerEvent<HTMLElement>)=>{
  if(!enabled||!event.isPrimary||event.button!==0||(event.target as Element).closest("button,input,a"))return;
  const scene=event.currentTarget.closest<HTMLElement>(".scene-player");
  const node=scene?.closest<HTMLElement>(".android-player-sheet")??scene;
  if(!node||node.classList.contains("android-player-sheet")&&node.dataset.playerPhase!=="open")return;
  drag.current={x:event.clientX,y:event.clientY,t:performance.now(),node};event.currentTarget.setPointerCapture(event.pointerId);
 },onPointerMove:(event:ReactPointerEvent<HTMLElement>)=>{
  const current=drag.current;if(!current)return;
  if(Math.abs(event.clientX-current.x)>Math.abs(event.clientY-current.y)+16){finish(event,true);return;}
  current.node.dataset.dragging="true";current.node.style.setProperty("--player-drag-y",`${Math.max(0,event.clientY-current.y)}px`);
 },onPointerUp:(event:ReactPointerEvent<HTMLElement>)=>finish(event),onPointerCancel:(event:ReactPointerEvent<HTMLElement>)=>finish(event,true),onLostPointerCapture:(event:ReactPointerEvent<HTMLElement>)=>finish(event,true)};
}
