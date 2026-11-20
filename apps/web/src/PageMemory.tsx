import {Activity,Component,useLayoutEffect,useRef,type ReactNode} from "react";
interface Props {scope:string;page:string;active:boolean;cacheable:boolean;children:ReactNode}
interface Saved {page:string;node:ReactNode}
interface State {scope:string;pages:Saved[]}
/** Hidden Activities stop requests/timers and preserve a bounded recent UI window.
 * Audio and playback actors remain outside this boundary. */
export class PageMemory extends Component<Props,State> {
 state:State={scope:"",pages:[]};
 static getDerivedStateFromProps(props:Props,state:State):State {
  const pages=state.scope===props.scope?state.pages:[];
  if(!props.active||!props.cacheable)return pages===state.pages?state:{scope:props.scope,pages};
  const current=pages.find(p=>p.page===props.page);
  if(current?.node===props.children&&pages.at(-1)===current)return state;
  return {scope:props.scope,pages:[...pages.filter(p=>p.page!==props.page),{page:props.page,node:props.children}].slice(-3)};
 }
 render(){const p=this.props;return <>{this.state.pages.map(saved=><Activity key={`${this.state.scope}:${saved.page}`} mode={p.active&&p.cacheable&&p.page===saved.page?"visible":"hidden"}><RouteFrame>{saved.node}</RouteFrame></Activity>)}{p.active&&!p.cacheable&&<RouteFrame key={`${p.scope}:${p.page}`}>{p.children}</RouteFrame>}</>;}
}
function RouteFrame({children}:{children:ReactNode}) {
 const element=useRef<HTMLDivElement>(null),position=useRef(0);
 useLayoutEffect(()=>{
  const node=element.current,scroll=node?.closest<HTMLElement>(".main-content");
  if(scroll)scroll.scrollTop=position.current;
  const animation=document.documentElement.dataset.performance!=="low"&&node?.animate&&!matchMedia("(prefers-reduced-motion: reduce)").matches?node.animate([{opacity:.55,transform:"translateY(7px)"},{opacity:1,transform:"translateY(0)"}],{duration:210,easing:"cubic-bezier(.2,.8,.2,1)"}):undefined;
  return()=>{if(scroll)position.current=scroll.scrollTop;animation?.cancel();};
 },[]);
 return <div ref={element} className="route-page">{children}</div>;
}
