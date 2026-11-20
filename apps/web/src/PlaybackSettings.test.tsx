import {it,expect} from "vitest";
import {renderToStaticMarkup} from "react-dom/server";
import type {Endpoint,RMusicClient,Zone} from "@r-music/sdk";
import {PlaybackSettings} from "./PlaybackSettings";
it("Playback settings keep provider qualities in the provider settings and hide unsupported fades",()=>{
 const endpoint:Endpoint={id:"hq",name:"HQPlayer",online:true,verified:false,kind:"hqplayer",capabilities:{transitions:false}};
 const html=renderToStaticMarkup(<PlaybackSettings api={{} as RMusicClient} admin zone={{id:"zone",name:"HQPlayer"} as Zone} endpoint={endpoint} refresh={async()=>{}} notify={()=>{}} onError={()=>{}}/>);
 expect(html).not.toContain("在线播放音质");expect(html).not.toContain("曲首淡入");expect(html).not.toContain("曲尾淡出");
});
