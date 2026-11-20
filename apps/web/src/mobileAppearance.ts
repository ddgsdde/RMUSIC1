import type {Preferences} from "./App";
export type MobileUI = "studio" | "classic" | "compact" | "liser";
export const mobileInterfaces = [
 {id:"studio" as const,name:"唱片志",description:"大幅封面、紧凑资料和左右滑动的唱片架"},
 {id:"classic" as const,name:"经典列表",description:"横排精选，更早看到专辑和曲目"},
];
export const androidInterfaces = [
 {id:"compact" as const,name:"简洁",description:"内容优先、五栏导航，专辑与歌曲直接进入首屏"},
 {id:"classic" as const,name:"经典",description:"紧凑曲目列表、清晰分区与封面氛围"},
 {id:"liser" as const,name:"Liser",description:"柔和渐变、圆形唱片与悬浮导航"},
 {id:"studio" as const,name:"唱片工作室",description:"唱片封套首页与精选音乐"},
];
export function mobileUI(value:unknown,android=false):MobileUI {
 if(android)return androidInterfaces.find(item=>item.id===value)?.id||"compact";
 return value==="classic"?"classic":"studio";
}
export function switchMobileUI(preferences:Preferences,next:MobileUI,android=false):Partial<Preferences> {
 const current=mobileUI(preferences.mobile_ui,android);
 if(current===next)return {};
 if(!android)return {mobile_ui:next};
 const mobile_themes={...preferences.mobile_themes,[current]:preferences.theme};
 return {mobile_ui:next,mobile_themes,studio_skin:undefined,theme:mobile_themes[next]||(next==="liser"?"liser-violet":"graphite")};
}

export function sceneColor(cover:readonly number[],base:string,light:boolean,atmosphere:boolean):string {
 if(!atmosphere)return base;
 const channels=base.match(/^#([\da-f]{2})([\da-f]{2})([\da-f]{2})$/i)?.slice(1).map(v=>parseInt(v,16))||[18,18,18];
 const blend=light ? .15 : .35;
 return "#"+channels.map((value,i)=>Math.round(value*(1-blend)+(cover[i]||0)*255*blend).toString(16).padStart(2,"0")).join("");
}
