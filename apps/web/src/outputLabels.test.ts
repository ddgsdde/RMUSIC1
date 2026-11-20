import {expect,it} from "vitest";
import type {Zone} from "@r-music/sdk";
import type {AudioDevice} from "./native";
import {deviceOutputLabels,outputName,zoneDisplayName} from "./outputLabels";
it("设备名称与输出名称独立显示，浏览器旧默认名称兼容且保留别名",()=>{
 const devices=[{name:"书房 Mac",outputs:[{id:"dac",name:"USB DAC",kind:"coreaudio"}]},{name:"Mac · Safari",outputs:[{id:"browser-one",name:"系统音频输出",kind:"browser"}]}] as AudioDevice[];
 expect(deviceOutputLabels(devices)).toEqual({dac:"书房 Mac · USB DAC","browser-one":"Mac · Safari · 浏览器输出"});
 expect(outputName({name:"客厅音箱",kind:"browser"})).toBe("客厅音箱");
 expect(outputName({name:"系统音频输出",kind:"android"})).toBe("系统音频输出");
 const zone={id:"zone-local",name:"本机",endpoint_id:"browser-one"} as Zone;
 expect(zoneDisplayName(zone)).toBe("浏览器输出");
 expect(zoneDisplayName({...zone,name:"我的书房"})).toBe("我的书房");
 expect(zoneDisplayName({...zone,id:"custom-zone"})).toBe("本机");
});
