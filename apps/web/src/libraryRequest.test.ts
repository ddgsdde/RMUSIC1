import {describe,it,expect} from "vitest";
import {isCloudSearch,requestRevision} from "./libraryRequest";
describe("聚合搜索刷新",()=>{
 it("曲库导入事件不能重新触发同一搜索",()=>{expect(requestRevision("夜航",1)).toBe(requestRevision("夜航",999));expect(requestRevision("",1)).not.toBe(requestRevision("",2));});
 it("默认搜索聚合云端，本地筛选与专辑内搜索不请求云端",()=>{expect(isCloudSearch("夜航","","","")).toBe(true);expect(isCloudSearch("夜航","local","","")).toBe(false);expect(isCloudSearch("夜航","","album","")).toBe(false);expect(isCloudSearch(" ","","","")).toBe(false);});
});
