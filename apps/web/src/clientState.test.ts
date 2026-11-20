// @vitest-environment jsdom
import {beforeEach,expect,it} from "vitest";
import type {Zone} from "@r-music/sdk";
import {preferredZone} from "./deviceIdentity";
import {migrateClientPreferences,saveClientPreferences,readClientPreferences} from "./clientPreferences";
import {translatedLyrics} from "./lyrics";
const defaults={theme:"graphite",mode:"dark" as const,low_performance:false,custom_themes:[]};
beforeEach(()=>localStorage.clear());
it("注册本机之后不继续默认控制另一台离线电脑，明确选择的远程音区仍保留",()=>{
 const zones=[{id:"old",endpoint_id:"windows"},{id:"phone",endpoint_id:"android"}] as Zone[];
 expect(preferredZone(zones,"old","android","")).toBe("phone");
 expect(preferredZone(zones,"phone","android","old")).toBe("old");
});
it("客户端偏好只迁移一次，之后其他客户端的账号偏好不能覆盖本机",()=>{
 expect(migrateClientPreferences({theme:"ocean"},defaults).theme).toBe("ocean");
 saveClientPreferences({...defaults,mode:"light"});
 expect(migrateClientPreferences({theme:"sand",mode:"dark"},defaults)).toMatchObject({theme:"graphite",mode:"light"});
});
it("将独立译文按实际时间戳对齐，不把未对齐的一行绑定到错误原文",()=>{
 const lines=translatedLyrics("[00:01.00]Hello\n[00:03.00]World","[00:01.02]你好\n[00:05.00]其他句");
 expect(lines[0].translation).toBe("你好");expect(lines[1].translation).toBeUndefined();
});

it("移除网页 Mineradio 后回退原主题和皮肤，保留其他偏好且不再保存特效设置",()=>{
 localStorage.setItem("rmusic.client-preferences.v1",JSON.stringify({...defaults,theme:"mineradio",player_skin:"mineradio",mineradio:{preset:8},lyric_font_size:31}));
 const restored=readClientPreferences(defaults);
 expect(restored).toMatchObject({theme:"graphite",player_skin:"silk",lyric_font_size:31});
 expect(restored).not.toHaveProperty("mineradio");
 saveClientPreferences(restored);
 expect(JSON.parse(localStorage.getItem("rmusic.client-preferences.v1")!)).not.toHaveProperty("mineradio");
 expect(migrateClientPreferences({},defaults)).toMatchObject({theme:"graphite",player_skin:"silk"});
});
