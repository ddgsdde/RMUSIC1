// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach,expect,it,vi} from "vitest";
import {ApiError,RMusicClient} from "@r-music/sdk";
import {FavoritesBinding} from "./FavoritesBinding";
afterEach(()=>vi.unstubAllGlobals());
it("选择单张可写歌单并确认转移，不将只读收藏作为双向目标",async()=>{
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);const api=new RMusicClient("https://core","session");vi.spyOn(api,"get").mockResolvedValue({id:"favorites-user",track_ids:[]});
 const request=vi.spyOn(api,"request").mockRejectedValueOnce(new ApiError(409,"binding_exists","existing")).mockResolvedValue({});
 const close=vi.fn(),changed=vi.fn().mockResolvedValue(undefined),error=vi.fn(),host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 try{await act(async()=>root.render(<FavoritesBinding api={api} cloud={[{id:"target",source:"example",name:"指定歌单",account_context:"one",writable:true,write_capabilities:["add","remove"]},{id:"readonly",source:"example",name:"只读收藏",writable:false}] as any} plugins={[{id:"example",name:"音源",enabled:true,capabilities:["playlist_write_v1","account_context_v1"]}] as any} onClose={close} onChanged={changed} onError={error} notify={vi.fn()}/>));
 expect(host.textContent).not.toContain("只读收藏不能");expect(host.querySelectorAll("select option")).toHaveLength(2);
 await act(async()=>{const select=host.querySelector("select")!;select.value=select.options[1].value;select.dispatchEvent(new Event("change",{bubbles:true}));});
 await act(async()=>[...host.querySelectorAll("button")].find(b=>b.textContent==="绑定并双向同步")!.click());expect(host.querySelector('[role="alert"]')?.textContent).toContain("原副本停止同步");expect(close).not.toHaveBeenCalled();
 await act(async()=>[...host.querySelectorAll("button")].find(b=>b.textContent==="确认转移并绑定")!.click());expect(request).toHaveBeenLastCalledWith("/favorites/binding",{method:"PUT",body:JSON.stringify({provider:"example",playlist_id:"target",account_context:"one",take_over:true})});expect(changed).toHaveBeenCalledOnce();expect(close).toHaveBeenCalledOnce();expect(error).not.toHaveBeenCalled();
 }finally{await act(async()=>root.unmount());host.remove();}
});

it("冲突变化后显示最新两边版本，保留对话框并要求再次选择",async()=>{
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
 const api=new RMusicClient("https://core","session");
 const favorite=(id:string,name:string)=>({id:"favorites-user",name:"我喜欢",track_ids:[],sync:{provider:"example",playlist_id:"target",status:"conflict",capabilities:["add","remove"],conflict:{id,reason:"顺序冲突",local:{name:"我喜欢",track_ids:["one"]},cloud:{name,track_ids:["two"]}}}});
 vi.spyOn(api,"get").mockResolvedValueOnce(favorite("first","旧云端版本")).mockResolvedValueOnce(favorite("second","最新云端版本")).mockResolvedValue(favorite("second","最新云端版本"));
 const post=vi.spyOn(api,"post").mockRejectedValueOnce(new ApiError(409,"playlist_conflict_changed","已变化")).mockResolvedValue({});
 const onClose=vi.fn(),host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 try{
  await act(async()=>root.render(<FavoritesBinding api={api} cloud={[]} plugins={[]} onClose={onClose} onChanged={vi.fn().mockResolvedValue(undefined)} onError={vi.fn()} notify={vi.fn()}/>));
  const useCloud=()=>[...host.querySelectorAll("button")].find(button=>button.textContent==="采用云端版本")!.click();
  await act(async()=>useCloud());
  expect(post).toHaveBeenLastCalledWith("/playlists/favorites-user/sync",{resolve:"cloud",conflict_id:"first"});
  expect(host.textContent).toContain("最新云端版本");
  expect(onClose).not.toHaveBeenCalled();
  await act(async()=>useCloud());
  expect(post).toHaveBeenLastCalledWith("/playlists/favorites-user/sync",{resolve:"cloud",conflict_id:"second"});
 }finally{await act(async()=>root.unmount());host.remove();}
});
