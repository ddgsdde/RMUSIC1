import {mobileUI,type MobileUI} from "../mobileAppearance";
import type {ThemeManifest} from "@r-music/sdk";
import type {Preferences} from "../App";
import {androidThemes,validateTheme} from "../themes";

export const tabNames={discover:"发现",search:"搜索","library-home":"音乐库",playlists:"歌单",favorites:"我喜欢",stats:"聆听统计",zones:"播放设备",settings:"设置"} as const;
export type Tab=keyof typeof tabNames;
export const defaultTabs:Tab[]=["discover","library-home","search"];
export type Palette=ThemeManifest["dark"];
export type Skin={mobile_ui?:MobileUI;kind:"rmusic-skin";version:1;id:string;name:string;layout:"sleeve"|"gallery"|"type";font:"serif"|"sans"|"mono";spacing:"open"|"dense";radius:number;scale:number;nav:"labels"|"icons";motion:"system"|"quiet";tabs?:Tab[];cover?:"square"|"soft"|"disc";dark:Palette;light:Palette};
export const colors=["bg","surface","raised","text","muted","border","accent","accentText"] as const;
const palette=(...values:string[]):Palette=>Object.fromEntries(colors.map((k,i)=>[k,values[i]])) as Palette;
const press:Skin={kind:"rmusic-skin",version:1,id:"press",name:"唱片志",layout:"sleeve",font:"serif",spacing:"open",radius:10,scale:1,nav:"labels",motion:"system",
 dark:palette("#1d1b19","#24211e","#312b25","#f4eae0","#c3b5a9","#443c34","#ffac92","#331914"),
 light:palette("#f8f6f0","#f4f0e7","#eeece4","#25231f","#69655e","#e1dbce","#b8412c","#ffffff")};
export const builtins:Skin[]=[press,
 {...press,id:"grove",name:"林间电台",cover:"soft",layout:"gallery",font:"sans",radius:14,dark:palette("#111e1b","#1b2b25","#293a32","#edf3de","#a9bbac","#384e44","#c3dd96","#1e2f1b"),light:palette("#eff3e9","#fafcf5","#e0e8d5","#22362b","#556759","#c6d2c2","#41643d","#ffffff")},
 {...press,id:"signal",name:"午夜电波",cover:"disc",layout:"type",font:"mono",spacing:"dense",radius:6,dark:palette("#191725","#242235","#302b42","#f5ecff","#bcb2ce","#48415b","#c9b3ff","#231737"),light:palette("#f2eef8","#fcf9ff","#e8e0f2","#29233a","#6c607b","#d1c6df","#61468f","#ffffff")}];
export const androidDefaultTabs:Tab[]=["discover","search","library-home","playlists"];
const androidPress:Skin={kind:"rmusic-skin",version:1,id:"press",name:"黑胶印室",layout:"sleeve",font:"serif",spacing:"open",radius:10,scale:1,nav:"labels",motion:"system",
 dark:palette("#151817","#202422","#2a302d","#f5f0e5","#b3b7ad","#414741","#f9aa79","#24160d"),
 light:palette("#f6f3eb","#fdfbf6","#eee8db","#252822","#606457","#cfcbbd","#99471e","#ffffff")};
export const androidBuiltins:Skin[]=[androidPress,
 {...androidPress,id:"grove",name:"林间电台",cover:"soft",tabs:["discover","library-home","playlists","search"],layout:"gallery",font:"sans",radius:22,dark:palette("#111e1b","#1b2b25","#293a32","#edf3de","#a9bbac","#384e44","#c3dd96","#1e2f1b"),light:palette("#eff3e9","#fafcf5","#e0e8d5","#22362b","#556759","#c6d2c2","#41643d","#ffffff")},
 {...androidPress,id:"signal",name:"午夜电波",cover:"disc",tabs:["discover","search","library-home","settings"],layout:"type",font:"mono",spacing:"dense",radius:3,nav:"icons",dark:palette("#191725","#242235","#302b42","#f5ecff","#bcb2ce","#48415b","#c9b3ff","#231737"),light:palette("#f2eef8","#fcf9ff","#e8e0f2","#29233a","#6c607b","#d1c6df","#61468f","#ffffff")}];
const clone=(s:Skin):Skin=>({...s,dark:{...s.dark},light:{...s.light}});
export function validateSkin(raw:unknown):Skin{
 if(!raw||typeof raw!=="object")throw Error("皮肤文件格式无效");
 const v=raw as Skin;
 if(v.kind!=="rmusic-skin"||v.version!==1)throw Error("不支持的皮肤版本");
 if(typeof v.name!=="string"||!v.name.trim()||v.name.length>40||typeof v.id!=="string"||!/^[-a-z0-9]{1,48}$/.test(v.id))throw Error("皮肤名称或标识无效");
 for(const [value,allowed] of [[v.layout,["sleeve","gallery","type"]],[v.font,["sans","serif","mono"]],[v.spacing,["open","dense"]],[v.nav,["labels","icons"]],[v.motion,["system","quiet"]]] as const)
  if(!(allowed as readonly string[]).includes(value))throw Error("皮肤的布局选项无效");
 if(!Number.isInteger(v.radius)||v.radius<0||v.radius>32||!Number.isFinite(v.scale)||v.scale<0.85||v.scale>1.25)throw Error("皮肤尺寸超出范围");
 const tabs=v.tabs??defaultTabs,cover=v.cover??"square";
 if(!Array.isArray(tabs)||(tabs.length<3||tabs.length>4)||new Set(tabs).size!==tabs.length||!tabs.every(t=>Object.hasOwn(tabNames,t)))throw Error("导航需要三或四个不同的有效入口");
 if(!["square","soft","disc"].includes(cover))throw Error("封面样式无效");
 if(v.mobile_ui!==undefined&&!["compact","classic","liser","studio"].includes(v.mobile_ui))throw Error("手机界面选项无效");
 const theme=validateTheme({schema_version:1,id:v.id,name:v.name,light:v.light,dark:v.dark});
 return {mobile_ui:v.mobile_ui,kind:"rmusic-skin",version:1,tabs:[...tabs],cover,name:v.name.trim(),id:v.id,layout:v.layout,font:v.font,spacing:v.spacing,radius:v.radius,scale:v.scale,nav:v.nav,motion:v.motion,light:theme.light,dark:theme.dark};
}
export function storedSkins(p:Preferences):Skin[]{return (Array.isArray(p.studio_skins)?p.studio_skins:[]).slice(0,20).flatMap(s=>{try{return [validateSkin(s)]}catch{return []}});}
export function importedColorSkins(p:Preferences):Skin[]{
 return (p.custom_themes||[]).flatMap(raw=>{try{
   const t=validateTheme(raw);return [{...clone(press),id:"theme-"+t.id.slice(0,40),name:t.name,dark:t.dark,light:t.light}];
 }catch{return []}});
}
const retiredPalettes:Record<string,string>={
 graphite:"press",mineradio:"press",paper:"press",sand:"press",
 ocean:"signal","liser-blue":"signal","liser-violet":"signal","liser-emerald":"grove",
};
export function activeSkin(p:Preferences,android=false):Skin{
 const custom=storedSkins(p), imported=importedColorSkins(p);
 const explicit=[...(android?androidBuiltins:builtins),...custom,...imported].find(s=>s.id===p.studio_skin);
 if(explicit)return clone(explicit);
 const legacy=imported.find(s=>s.id==="theme-"+p.theme?.slice(0,40));
 if(legacy)return clone(legacy);
 if(android){
  const layout=mobileUI(p.mobile_ui,true),base=clone(layout==="liser"?androidBuiltins[2]:androidPress);
  if(layout!=="studio"){base.font="sans";base.spacing="dense";base.radius=14;}
  const id=p.studio_skin?.startsWith("theme-")?p.studio_skin.slice(6):p.theme;
  const theme=androidThemes.find(item=>item.id===id);
  if(theme&&(theme.id!=="graphite"||layout==="classic"||layout==="liser")){
   const dark=theme.id==="graphite"&&layout==="classic"?{...theme.dark,bg:"#121212",surface:"#121212",raised:"#242424",accent:"#1ed760",accentText:"#07120a"}:theme.dark;
   return {...base,id:"theme-"+theme.id,name:theme.name,light:theme.light,dark};
  }
  return base;
 }
 const id=(p.studio_skin?.startsWith("theme-")?retiredPalettes[p.studio_skin.slice(6)]:undefined)
  ||retiredPalettes[p.theme];
 return clone(builtins.find(s=>s.id===id)||press);
}
export function importSkin(text:string,base:Skin):Skin{
 if(text.length>32768)throw Error("皮肤文件最多 32 KiB");
 const v=JSON.parse(text);
 if(v?.kind==="rmusic-skin")return validateSkin(v);
 if(v.mobile_ui!==undefined&&!["compact","classic","liser","studio"].includes(v.mobile_ui))throw Error("手机界面选项无效");
 const theme=validateTheme(v);return {...clone(base),id:theme.id,name:theme.name,light:theme.light,dark:theme.dark};
}
export function applySkin(p:Preferences,mode:"dark"|"light",mobile=false,android=false){
 const skin=activeSkin(p,android),root=document.documentElement;
 root.dataset.theme=skin.id;root.dataset.mode=mode;root.style.colorScheme=mode;
 if(mobile)root.dataset.mobileStyle=mobileUI(p.mobile_ui,android);else delete root.dataset.mobileStyle;
 root.toggleAttribute("data-native-android",android);
 for(const k of colors){root.style.setProperty("--"+k,skin[mode][k]);root.style.setProperty("--mobile-"+k,skin[mode][k]);}
 root.dataset.artShape=skin.cover??"square";root.dataset.deck=skin.layout;root.dataset.studioFont=skin.font;root.dataset.navLabels=skin.nav;root.dataset.spacing=skin.spacing;root.dataset.skinMotion=skin.motion;
 root.style.setProperty("--mobile-page-padding",skin.spacing==="dense"?"14px":"20px");
 root.style.setProperty("--skin-radius",skin.radius+"px");root.style.setProperty("--type-scale",String(skin.scale));
}

