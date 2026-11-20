import {useEffect,useRef,useState,type PointerEvent} from "react";
import {GripVertical} from "lucide-react";

export function moveOrder(length:number,from:number,to:number):number[]{
 const order=Array.from({length},(_,i)=>i);
 if(from===to||from<0||to<0||from>=length||to>=length)return order;
 order.splice(to,0,order.splice(from,1)[0]);return order;
}
/** Handles own the gesture; swiping anywhere else continues to scroll normally. */
export function useTrackReorder({version,count,disabled,onMove}:{version:string;count:number;disabled?:boolean;onMove:(from:number,to:number)=>void}){
 const container=useRef<HTMLDivElement>(null),gesture=useRef<{from:number;to:number;x:number;y:number;startY:number;active:boolean;frame:number;preview?:HTMLElement;origin?:DOMRect;move:typeof onMove}|null>(null);
 const [target,setTarget]=useState<number|null>(null),[dragging,setDragging]=useState<number|null>(null);
 const cancel=()=>{if(gesture.current){cancelAnimationFrame(gesture.current.frame);gesture.current.preview?.style.removeProperty("transform");}gesture.current=null;setTarget(null);setDragging(null);};
 useEffect(()=>cancel,[version,disabled]);
 const hit=()=>{const g=gesture.current,root=container.current;if(!g||!root)return;const row=document.elementFromPoint(g.x,g.y)?.closest<HTMLElement>("[data-reorder-index]");if(row&&root.contains(row)){g.to=Number(row.dataset.reorderIndex);setTarget(g.to);}};
 const tick=()=>{const g=gesture.current,root=container.current;if(!g||!root)return;if(g.active){if(g.preview&&g.origin)g.preview.style.transform=`translateY(${g.y-g.startY}px)`;let scroll:HTMLElement|null=root;while(scroll&&!(scroll.scrollHeight>scroll.clientHeight+1&&/(auto|scroll)/.test(getComputedStyle(scroll).overflowY)))scroll=scroll.parentElement;scroll??=document.scrollingElement as HTMLElement;const rect=scroll===document.scrollingElement?{top:0,bottom:innerHeight}:scroll.getBoundingClientRect();const speed=g.y<rect.top+64?-Math.min(14,(rect.top+64-g.y)/4):g.y>rect.bottom-64?Math.min(14,(g.y-rect.bottom+64)/4):0;if(speed){scroll.scrollTop+=speed;hit();}}g.frame=requestAnimationFrame(tick);};
 const start=(event:PointerEvent<HTMLButtonElement>,index:number)=>{if(disabled||event.button!==0)return;cancel();event.currentTarget.setPointerCapture(event.pointerId);gesture.current={from:index,to:index,x:event.clientX,y:event.clientY,startY:event.clientY,active:false,frame:requestAnimationFrame(tick),move:onMove};};
 const handle=(index:number,title:string)=><button type="button" className="track-drag-handle" style={{touchAction:"none"}} aria-label={`拖动排序 ${title}`} title="拖动排序，也可聚焦后按方向键上下移动" disabled={disabled} onPointerDown={event=>start(event,index)} onPointerMove={event=>{const g=gesture.current;if(!g)return;g.x=event.clientX;g.y=event.clientY;if(Math.abs(g.y-g.startY)>5){g.active=true;setDragging(g.from);}if(g.active){if(!g.preview){g.preview=event.currentTarget.closest<HTMLElement>("[data-reorder-index]")||undefined;g.origin=g.preview?.getBoundingClientRect();}hit();}}} onPointerUp={()=>{const g=gesture.current;cancel();if(g?.active&&g.from!==g.to)g.move(g.from,g.to);}} onPointerCancel={cancel} onLostPointerCapture={cancel} onKeyDown={event=>{if(event.key!=="ArrowUp"&&event.key!=="ArrowDown")return;event.preventDefault();const next=index+(event.key==="ArrowUp"?-1:1);if(!disabled&&next>=0&&next<count)onMove(index,next);}}><GripVertical size={18}/></button>;
 return {container,handle,rowProps:(index:number)=>({"data-reorder-index":index,"data-reorder-target":target===index&&dragging!==index?"true":undefined,"data-reorder-dragging":dragging===index?"true":undefined})};
}
