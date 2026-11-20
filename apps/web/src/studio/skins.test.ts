import {describe,it,expect} from "vitest";
import type {Preferences} from "../App";
import {activeSkin,builtins,defaultTabs,importSkin,validateSkin} from "./skins";

describe("portable skins",()=>{
 it("round-trips a customized layout, navigation and both palettes",()=>{
   const skin=validateSkin({...builtins[1],id:"custom-1",mobile_ui:"liser",radius:24,tabs:["settings","search","discover","zones"]});
   expect(importSkin(JSON.stringify(skin),builtins[0])).toEqual(skin);
   expect(activeSkin({studio_skin:skin.id,studio_skins:[skin]} as Preferences)).toEqual(skin);
 });
 it("rejects invalid navigation, unchecked CSS values and unsupported versions",()=>{
   for(const value of [{...builtins[0],tabs:defaultTabs.map(()=>"settings")},{...builtins[0],version:42},{...builtins[0],light:{...builtins[0].light,accent:"url(https://bad)"}},{...builtins[0],radius:100}]){
     expect(()=>importSkin(JSON.stringify(value),builtins[0])).toThrow();
   }
 });
 it("preserves a previously imported colour theme",()=>{
   const t={schema_version:1,id:"existing",name:"旧配色",light:builtins[2].light,dark:builtins[2].dark};
   const p={theme:t.id,custom_themes:[t]} as Preferences;
   expect(activeSkin(p).dark).toEqual(t.dark);
   expect(activeSkin({...p,studio_skin:"theme-existing"}).light).toEqual(t.light);
 });
});

