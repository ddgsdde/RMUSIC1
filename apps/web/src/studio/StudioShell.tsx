import type {MobileUI} from "../mobileAppearance";
import {androidDefaultTabs,defaultTabs,tabNames,type Tab} from "./skins";
import {useEffect,useRef} from "react";
import {ArrowDownToLine,BookOpen,ChevronDown,ChevronRight,ChartNoAxesColumn,CircleUserRound,Compass,Disc3,Heart,LibraryBig,ListMusic,LogOut,PanelLeftClose,PanelLeftOpen,Radio,Search,Settings,SlidersHorizontal,Speaker} from "lucide-react";
import type {Route} from "../App";
import {Modal} from "../components";

const libraryRoutes:Route[]=["library-home","library","albums","artists","library-manage"];
const primary=[
 {id:"discover",name:"发现",icon:Compass},{id:"library-home",name:"音乐库",icon:LibraryBig},
 {id:"favorites",name:"我喜欢",icon:Heart},{id:"playlists",name:"播放列表",icon:ListMusic},
 {id:"zones",name:"播放设备",icon:Speaker},{id:"dsp",name:"DSP",icon:SlidersHorizontal},{id:"stats",name:"聆听统计",icon:ChartNoAxesColumn},
 {id:"settings",name:"设置",icon:Settings},
] as const;
export function routeTitle(route:Route){
  if(libraryRoutes.includes(route))return "音乐库";
  return primary.find(r=>r.id===route)?.name||({settings:"设置","daily-playlist":"每日歌单",profile:"我的音乐","core-connection":"Core 连接",search:"搜索",zones:"播放设备",quality:"音质与缓存",player:"正在播放"} as Record<string,string>)[route]||"音乐";
}
export function Mark(){return <Disc3 className="record-mark" size={24} strokeWidth={1.55} aria-hidden="true"/>}
export function ProfilePage({user, online, device, navigate, offline, logout}:{
 user:string;online:boolean;device:string;navigate:(route:Route)=>void;offline:()=>void;logout:()=>void;
}){
 const groups=[
  {title:"音乐",items:[
   {label:"我喜欢",icon:Heart,action:()=>navigate("favorites")},
   {label:"本机离线音乐",icon:ArrowDownToLine,action:offline},
   {label:"播放列表",icon:ListMusic,action:()=>navigate("playlists")},
   {label:"聆听统计",icon:ChartNoAxesColumn,action:()=>navigate("stats")},
  ]},
  {title:"设备与应用",items:[
   {label:"播放设备",icon:Speaker,action:()=>navigate("zones")},
   {label:"设置",icon:Settings,action:()=>navigate("settings")},
  ]},
 ];
 return <div className="mobile-profile-page">
  <header className="mobile-profile-account">
   <div><h1>我的音乐</h1><p><strong>{user}</strong><span aria-hidden="true">·</span>{device}</p></div>
   <span aria-hidden="true">{user.slice(0,1).toUpperCase()}</span>
  </header>
  <button type="button" className="mobile-profile-core" onClick={()=>navigate("core-connection")}>
   <span className="mobile-profile-core-icon"><Radio size={19} aria-hidden="true"/></span>
   <span><strong>Core 连接</strong><small>{online?"已连接":"正在重连"}<span aria-hidden="true"> · </span>链路与备用地址</small></span>
   <ChevronRight size={17} aria-hidden="true"/>
  </button>
  {groups.map(group=><section className="mobile-profile-group" key={group.title}>
   <h2>{group.title}</h2>
   <div className="mobile-profile-links">
    {group.items.map(({label,icon:Icon,action})=><button type="button" key={label} onClick={action}>
     <Icon size={18} aria-hidden="true"/><span>{label}</span><ChevronRight size={16} aria-hidden="true"/>
    </button>)}
   </div>
  </section>)}
  <button type="button" className="mobile-profile-signout" onClick={logout}><LogOut size={16}/>退出登录</button>
 </div>;
}
const tabIcons={discover:Compass,search:Search,"library-home":BookOpen,playlists:ListMusic,favorites:Heart,stats:ChartNoAxesColumn,zones:Speaker,settings:Settings};
export function StudioShell({mobile,railHidden=false,toggleRail,inert=false,layout="studio",legacyAndroid=false,route,user,tabs=legacyAndroid?androidDefaultTabs:defaultTabs,online,device,output,showZones,query,search,navigate,menu,setMenu,offline,logout}:{
 mobile:boolean;railHidden?:boolean;toggleRail?:()=>void;inert?:boolean;layout?:MobileUI;legacyAndroid?:boolean;route:Route;user:string;tabs?:Tab[];online:boolean;device:string;output?:string;
 showZones?:()=>void;query:string;search:(v:string)=>void;navigate:(r:Route)=>void;menu:boolean;setMenu:(v:boolean)=>void;offline:()=>void;logout:()=>void;
}){
 const searchInput=useRef<HTMLInputElement>(null);
 useEffect(()=>{
  if(mobile)return;
  const shortcut=(event:KeyboardEvent)=>{
   if((event.metaKey||event.ctrlKey)&&event.key.toLowerCase()==="k"){event.preventDefault();searchInput.current?.focus();}
  };
  window.addEventListener("keydown",shortcut);
  return()=>window.removeEventListener("keydown",shortcut);
 },[mobile]);
 const mobileTabs=tabs.map(id=>id==="settings"?([...defaultTabs,"playlists","zones"] as Tab[]).find(tab=>!tabs.includes(tab))??"zones":id);
 const profileActive=route==="profile"||route==="core-connection"||route==="settings";
 const active=(id:Route)=>id===route||id==="library-home"&&libraryRoutes.includes(route);
 const go=(r:Route)=>{setMenu(false);navigate(r);};
 const utils=[{label:"播放设备",icon:Speaker,action:()=>go("zones")},{label:"设置",icon:Settings,action:()=>go("settings")},{label:"本机离线音乐",icon:ArrowDownToLine,action:()=>{setMenu(false);offline();}},{label:"聆听统计",icon:ChartNoAxesColumn,action:()=>go("stats")},{label:"播放列表",icon:ListMusic,action:()=>go("playlists")}];
 if(mobile&&route==="player")return null;
 const searchBox=<label className="studio-search"><Search size={18}/><input ref={searchInput} type="search" autoFocus={mobile&&!legacyAndroid} value={query} placeholder="搜索歌曲、专辑、艺术家" aria-label="搜索歌曲、专辑、艺术家" onChange={e=>search(e.target.value)} onKeyDown={e=>{if(e.key==="Escape"||e.key==="Enter"&&!e.nativeEvent.isComposing)e.currentTarget.blur();}}/>{!mobile&&<kbd>⌘ K</kbd>}</label>;
 return <>
  {!mobile&&<>
   <aside id="music-navigation" hidden={railHidden} className="record-rail" aria-label="音乐导航">
    <button className="studio-brand" aria-label="R-Music 发现" onClick={()=>go("discover")}><Mark/><b>R·MUSIC</b></button>
    <nav aria-label="主导航">{primary.map(({id,name,icon:Icon})=><button key={id} aria-current={active(id)?"page":undefined} onClick={()=>go(id)}><Icon size={19}/><span>{name}</span></button>)}</nav>
    <button className="record-core" onClick={()=>go("core-connection")}><span><Radio size={18}/></span><div><strong>音乐核心</strong><small><i data-online={online}/>{online?"已连接":"正在重连"}</small></div></button>
   </aside>
   <header className="studio-mast">
    <div className="mast-search-group">
      <button className="studio-tool" aria-label={railHidden?"展开左侧导航":"收起左侧导航"} title={railHidden?"展开左侧导航":"收起左侧导航"} aria-expanded={!railHidden} aria-controls="music-navigation" onClick={toggleRail}>
        {railHidden?<PanelLeftOpen/>:<PanelLeftClose/>}
      </button>
      {searchBox}
    </div>
    <div className="studio-mast-end"><button className="record-output" onClick={showZones??(()=>go("zones"))} aria-label="选择播放设备" title={output}><Speaker size={18}/><span>{output||"选择播放设备"}</span><ChevronDown size={14}/></button>
    <button className="studio-profile" aria-label="打开账号与导航" aria-expanded={menu} onClick={()=>setMenu(true)}>{user.slice(0,1).toUpperCase()}</button></div>
   </header>
  </>}
  {mobile&&route==="search"&&<div inert={inert||undefined} className="studio-phone-search">{searchBox}</div>}
  {mobile&&<nav inert={inert||undefined} className={legacyAndroid&&(layout==="classic"||layout==="liser")?"music-mobile-dock":"studio-dock"} style={{gridTemplateColumns:`repeat(${mobileTabs.length+1}, minmax(0,1fr))`}} aria-label="主导航">{mobileTabs.map(id=>{const Icon=legacyAndroid&&id==="discover"?Disc3:tabIcons[id],name=id==="library-home"&&!legacyAndroid?"曲库":tabNames[id];return <button key={id} aria-label={name} aria-current={active(id)?"page":undefined} onClick={()=>go(id)}><Icon size={21}/><span>{name}</span></button>})}<button aria-label="我的" aria-current={profileActive?"page":undefined} onClick={()=>go("profile")}><CircleUserRound size={21}/><span>我的</span></button></nav>}
  {!mobile&&menu&&<Modal title="我的音乐" onClose={()=>setMenu(false)}><div className="studio-account"><CircleUserRound/><strong>{user}</strong><small><i data-online={online}/>{online?"核心已连接":"正在重新连接"} · {device}</small></div><div className="studio-menu-links">{utils.map(({label,icon:Icon,action})=><button key={label} onClick={action}><Icon/>{label}</button>)}</div><div className="studio-signout"><button onClick={()=>{setMenu(false);logout();}}><LogOut/>退出登录</button></div></Modal>}
 </>;
}
