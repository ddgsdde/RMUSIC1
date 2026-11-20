import {afterEach,expect,it,vi} from "vitest";
import {RMusicClient} from "@r-music/sdk";

afterEach(()=>vi.unstubAllGlobals());
it("曲库、歌单和队列请求保留显式视图，每个请求只发送一个 view 参数",async()=>{
 const requests:URL[]=[];
 vi.stubGlobal("fetch",vi.fn(async(input:string)=>{
  const url=new URL(input);requests.push(url);
  return url.searchParams.getAll("view").length>1
   ? new Response(JSON.stringify({error:{message:"duplicate field view"}}),{status:400})
   : new Response(JSON.stringify({items:[],id:"room",queue:[]}));
 }));
 const api=new RMusicClient("https://core.test","qa");
 for(const path of ["/library/tracks?limit=50","/playlists/likes/tracks?sort=default&view=compact","/library/tracks?view=full"]){await api.get(path);}
 await api.post("/zones/room/queue?view=state",{track_ids:[]});
 expect(requests.map(url=>url.searchParams.getAll("view"))).toEqual([["compact"],["compact"],["full"],["state"]]);
});
