// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect,it,vi} from "vitest";
import {RMusicClient,type Track} from "@r-music/sdk";
import {RelatedCatalogPanel} from "./RelatedCatalog";
import {CatalogNavigation} from "./CatalogLinks";
it("歌曲资料直接链接到单一实体详情，不再嵌套专辑歌手混合目录",async()=>{
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
 const api=new RMusicClient("http://fixture","synthetic-session"),navigate=vi.fn();
 const track={id:"qqmusic:seed",title:"夜航",artist:"Crush / IU",album:"海岸线",provider_data:{artist_mids:["crush","iu"]},duration:4,source:"qqmusic",format:"wav",sample_rate:44100,bit_depth:16,channels:2,bitrate:0,recording_id:"seed",available:true} satisfies Track;
 vi.spyOn(api,'request').mockResolvedValue({resolved:true,artists:[{id:'crush',name:'Crush'},{id:'iu',name:'IU'}],album:{id:'album',title:'海岸线'}});
 const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
 try{
  await act(async()=>root.render(<CatalogNavigation api={api} location="details" navigate={navigate} onError={vi.fn()}><RelatedCatalogPanel api={api} track={track} play={vi.fn()} enqueue={vi.fn()}/></CatalogNavigation>));
  expect(host.textContent).not.toContain('完整专辑');expect(host.querySelector('select')).toBeNull();
  await act(async()=>host.querySelectorAll<HTMLElement>('[role=link]')[2].click());expect(navigate).toHaveBeenCalledWith('artist','iu','IU');
 }finally{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();}
});
