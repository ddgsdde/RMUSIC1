// @vitest-environment jsdom
import {act,useCallback} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,describe,expect,it,vi} from "vitest";
import type {Endpoint,RMusicClient,Track,Zone} from "@r-music/sdk";
import {ApiError} from "@r-music/sdk";
import type {NativePlayback} from "./native";
import {useAudio} from "./useAudio";
import {NativeOffline} from "./NativeCache";

const native=vi.hoisted(()=>({enabled:false,invoke:vi.fn(),profile:vi.fn(),listen:vi.fn()}));
vi.mock("./native",()=>({get isNative(){return native.enabled;},invokeNative:native.invoke,nativeProfile:native.profile,listenNative:native.listen,nativeOutput:()=>({exclusive:false,buffer_ms:100})}));
vi.mock("./browserCache",()=>({browserCache:{rememberTrack:vi.fn(async()=>{}),protect:vi.fn()},formatBytes:(n:number)=>`${n} B`}));
function deferred<T>() {let resolve!:(value:T)=>void,reject!:(error:Error)=>void;const promise=new Promise<T>((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};}
class TestAudio extends EventTarget {
 static instances:TestAudio[]=[];
 src="";volume=1;muted=false;currentTime=0;readyState=0;paused=true;ended=false;preload="";crossOrigin="";duration=60;seeking=false;error:{message:string;code?:number}|null={message:"测试网络中断"};
 play=vi.fn(()=>{this.paused=false;this.dispatchEvent(new Event("play"));return Promise.resolve();});
 pause=vi.fn(()=>{this.paused=true;this.dispatchEvent(new Event("pause"));});
 load=vi.fn();removeAttribute(name:string){if(name==="src")this.src="";}
 constructor(){super();TestAudio.instances.push(this);}
}
let root:Root,host:HTMLDivElement,controls:ReturnType<typeof useAudio>;
const endpoint={id:"browser-local",name:"测试客户端",kind:"browser",online:true,verified:true,capabilities:{}} as Endpoint;
const report=vi.fn();
const recovered=vi.fn();
const track=(id="song-a")=>({id,title:id,source:"local",format:"flac",duration:60,integrated_lufs:null}) as Track;
function zone(patch:Partial<Zone>={}):Zone{return {id:"zone-a",name:"测试音区",endpoint_id:endpoint.id,status:"playing",volume:.8,muted:false,repeat:"off",shuffle:false,position:0,track:track(),queue:[],queue_index:0,updated_at:0,dsp:{enabled:false,preamp_db:0,balance:0,normalize:false,target_lufs:-14,bands:[]},transitions:{fade_in_ms:0,fade_out_ms:0},...patch};}
function api(){return {baseUrl:"http://qa-core",mediaUrl:vi.fn(),request:vi.fn(),get:vi.fn(async()=>({})),post:vi.fn(async()=>({status:"stopped"}))} as unknown as RMusicClient;}
function Harness({client,current}:{client:RMusicClient;current:Zone}){const adopt=useCallback((value:Zone)=>{recovered(value);root.render(<Harness client={client} current={value}/>);},[client]);controls=useAudio(client,current,[endpoint],report,adopt);return <output>{controls.position}</output>;}
async function render(client:RMusicClient,current:Zone){await act(async()=>root.render(<Harness client={client} current={current}/>));}
beforeEach(()=>{
 vi.useFakeTimers();vi.clearAllMocks();native.enabled=false;TestAudio.instances=[];
 endpoint.capabilities={};endpoint.kind="browser";
 native.invoke.mockImplementation(async(command:string)=>command==="native_status"?{status:"stopped"}:undefined);
 native.profile.mockResolvedValue({id:"profile"});native.listen.mockResolvedValue(()=>{});
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);vi.stubGlobal("Audio",TestAudio);
 host=document.createElement("div");document.body.append(host);root=createRoot(host);
});
it("原生窗口尚未载入音区时保留正在播放的队列，明确停止状态仍关闭输出",async()=>{
 native.enabled=true;const client=api();
 native.invoke.mockImplementation(async(command:string)=>command==="native_status"?{core_managed:true,status:"playing",sequence_id:"running-sequence",track_id:"song-a",playback_id:"old-window",position:4}:undefined);
 function LoadingWindow(){controls=useAudio(client,undefined,[endpoint],report,recovered);return null;}
 await act(async()=>root.render(<LoadingWindow/>));
 expect(native.invoke.mock.calls.some(([command])=>command==="native_stop")).toBe(false);
 await render(client,zone({status:"stopped"}));
 expect(native.invoke.mock.calls.some(([command])=>command==="native_stop")).toBe(true);
});
describe("核心播放 ID 贯穿实际报告",()=>{
 it("同曲进入新核心实例时重新读取真实音频参数",async()=>{
  const client=api();vi.mocked(client.mediaUrl).mockResolvedValue("http://qa-core/media/song-a");vi.mocked(client.get).mockResolvedValueOnce({sample_rate:48000,bit_depth:16}).mockResolvedValueOnce({sample_rate:192000,bit_depth:24});
  await render(client,zone({playback_id:"core-first"}));expect(controls.sourceInfo?.sample_rate).toBe(48000);
  await render(client,zone({playback_id:"core-next"}));expect(client.get).toHaveBeenCalledTimes(2);expect(controls.sourceInfo?.sample_rate).toBe(192000);expect(controls.sourceInfo?.bit_depth).toBe(24);
 });
 it("浏览器同曲新实例重新取得地址，进度和结束携带捕获的核心 ID",async()=>{
  const client=api();vi.mocked(client.mediaUrl).mockResolvedValue("http://qa-core/media/song-a");await render(client,zone({playback_id:"core-first"}));
  await render(client,zone({playback_id:"core-next"}));expect(client.mediaUrl).toHaveBeenCalledTimes(2);const audio=TestAudio.instances[0];audio.currentTime=4;
  await act(async()=>audio.dispatchEvent(new Event("timeupdate")));expect(client.post).toHaveBeenCalledWith("/zones/zone-a/progress",{track_id:"song-a",playback_id:"core-next",position:4});
  audio.currentTime=60;audio.ended=true;await act(async()=>audio.dispatchEvent(new Event("ended")));expect(client.post).toHaveBeenCalledWith("/zones/zone-a/ended",{track_id:"song-a",playback_id:"core-next",position:60});
 });
 it("浏览器新解码器尚未结束时忽略旧解码器排队的 ended 事件",async()=>{
  const client=api();vi.mocked(client.mediaUrl).mockResolvedValue("http://qa-core/media/song-a");await render(client,zone({playback_id:"core-first"}));await render(client,zone({playback_id:"core-next"}));
  await act(async()=>TestAudio.instances[0].dispatchEvent(new Event("ended")));expect(client.post).not.toHaveBeenCalled();
 });
 it("旧 error 事件排队返回时，新音源没有解码错误便不切兼容流",async()=>{
  const client=api();vi.mocked(client.mediaUrl).mockResolvedValue("http://qa-core/media/song-a");await render(client,zone({playback_id:"core-first"}));await render(client,zone({playback_id:"core-next"}));
  TestAudio.instances[0].error=null;await act(async()=>TestAudio.instances[0].dispatchEvent(new Event("error")));expect(report).not.toHaveBeenCalled();expect(controls.adaptive).toBe(0);
 });
 it("短曲循环不把不同播放实例的初次缓冲累积为弱网降级",async()=>{
  const client=api();vi.mocked(client.mediaUrl).mockResolvedValue("http://qa-core/media/song-a");
  for(let i=0;i<3;i++){await render(client,zone({playback_id:`loop-${i}`}));await act(async()=>TestAudio.instances[0].dispatchEvent(new Event("waiting")));}
  expect(controls.adaptive).toBe(0);expect(report).not.toHaveBeenCalled();
 });
 it("同一播放实例连续三次真实缓冲保持原音质并提示恢复",async()=>{
  const client=api();vi.mocked(client.mediaUrl).mockResolvedValue("http://qa-core/media/song-a");await render(client,zone({playback_id:"core-first"}));
  for(let i=0;i<3;i++){TestAudio.instances[0].currentTime=i+1;await act(async()=>TestAudio.instances[0].dispatchEvent(new Event("waiting")));}
  expect(controls.adaptive).toBe(0);expect(report).toHaveBeenCalledWith(expect.stringContaining("仍保持原音质"));
 });
 it("原生新核心实例更换本机 ID，进度只使用对应核心 ID",async()=>{
  native.enabled=true;const client=api();await render(client,zone({playback_id:"core-first"}));const old=native.invoke.mock.calls.find(([command])=>command==="native_play")![1].playbackId;
  await render(client,zone({playback_id:"core-next"}));const current=native.invoke.mock.calls.filter(([command])=>command==="native_play").at(-1)![1].playbackId;expect(current).not.toBe(old);
  native.invoke.mockImplementation(async(command:string)=>command==="native_status"?{track_id:"song-a",playback_id:current,status:"playing",position:7,paused:false}:undefined);
  await act(async()=>vi.advanceTimersByTime(500));expect(client.post).toHaveBeenCalledWith("/zones/zone-a/progress",{track_id:"song-a",playback_id:"core-next",position:7});
  await act(async()=>native.listen.mock.calls.at(-1)![0]({track_id:"song-a",playback_id:current,status:"ended",position:60}));expect(client.post).toHaveBeenCalledWith("/zones/zone-a/ended",{track_id:"song-a",playback_id:"core-next",position:60});
 });
 it("新版核心的定位应答只重建一次原生输出，旧核心仍使用 native_seek",async()=>{
  native.enabled=true;const client=api();await render(client,zone({playback_id:"core-first"}));await act(async()=>controls.seek(25));expect(native.invoke.mock.calls.some(([command])=>command==="native_seek")).toBe(false);
  await render(client,zone({playback_id:"core-seek",position:25}));const plays=native.invoke.mock.calls.filter(([command])=>command==="native_play");expect(plays).toHaveLength(2);expect(plays[1][1].position).toBe(25);
 });
});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.useRealTimers();vi.unstubAllGlobals();});
describe("在线音源断流恢复",()=>{
 function recoveringClient(){const client=api();vi.mocked(client.mediaUrl).mockResolvedValue("http://qa-core/api/v1/media/song-a?ticket=t");vi.mocked(client.request).mockResolvedValue(zone({playback_id:"core-current",position:4}));return client;}
 async function networkError(at=17.25){const audio=TestAudio.instances[0];audio.currentTime=at;audio.error={code:2,message:"connection reset"};await act(async()=>audio.dispatchEvent(new Event("error")));}
 it("网络错误保留原格式，并从实际媒体位置恢复当前核心实例",async()=>{
  const client=recoveringClient();await render(client,zone({playback_id:"core-current"}));await networkError();
  expect(controls.adaptive).toBe(0);await act(async()=>vi.advanceTimersByTime(1000));
  expect(client.request).toHaveBeenCalledWith("/zones/zone-a",expect.anything());expect(client.mediaUrl).toHaveBeenCalledTimes(2);expect(TestAudio.instances[0].currentTime).toBe(17.25);expect(recovered).toHaveBeenCalledWith(expect.objectContaining({position:17.25,playback_id:"core-current"}));
 });
 it("恢复读取最新暂停、音量和静音，不重置实际播放位置",async()=>{
  const client=recoveringClient();await render(client,zone({playback_id:"core-current"}));await networkError(19);
  await render(client,zone({playback_id:"core-current",status:"paused",volume:.2,muted:true,position:4}));
  vi.mocked(client.request).mockResolvedValue(zone({playback_id:"core-current",status:"paused",volume:.2,muted:true,position:4}));
  await act(async()=>vi.advanceTimersByTime(1000));const audio=TestAudio.instances[0];expect(audio.currentTime).toBe(19);expect(audio.paused).toBe(true);expect(audio.volume).toBe(.2);expect(audio.muted).toBe(true);
 });
 it("核心已停止或重启后不自动重放旧曲",async()=>{
  const client=recoveringClient();await render(client,zone({playback_id:"core-current"}));await networkError();
  vi.mocked(client.request).mockResolvedValue(zone({playback_id:null,status:"stopped"}));await act(async()=>vi.advanceTimersByTime(1000));
  expect(client.mediaUrl).toHaveBeenCalledTimes(1);expect(TestAudio.instances[0].src).toBe("");expect(controls.playing).toBe(false);
 });
 it("换曲取消恢复读取，迟到结果不能恢复旧实例",async()=>{
  const client=recoveringClient(),response=deferred<Zone>();await render(client,zone({playback_id:"core-current"}));await networkError();
  vi.mocked(client.request).mockReturnValue(response.promise);await act(async()=>vi.advanceTimersByTime(1000));
  const request=vi.mocked(client.request).mock.calls.find(([path])=>path==="/zones/zone-a")!;
  await render(client,zone({track:track("song-b"),playback_id:"core-next"}));await act(async()=>response.resolve(zone({playback_id:"core-current"})));
  expect((request[1] as RequestInit).signal?.aborted).toBe(true);expect(recovered).not.toHaveBeenCalled();expect(client.mediaUrl).toHaveBeenCalledTimes(2);
 });
 it("登录失效停止重试，离线入口也会取消待恢复的播放",async()=>{
  const client=recoveringClient();await render(client,zone({playback_id:"core-current"}));await networkError();
  vi.mocked(client.request).mockRejectedValue(new ApiError(401,"expired","登录过期"));await act(async()=>vi.advanceTimersByTime(60000));
  expect(vi.mocked(client.request).mock.calls.filter(([path])=>path==="/zones/zone-a")).toHaveLength(1);expect(client.mediaUrl).toHaveBeenCalledTimes(1);
  await render(client,zone({playback_id:"core-next"}));await networkError();await act(async()=>window.dispatchEvent(new Event("rmusic-offline")));await act(async()=>vi.advanceTimersByTime(60000));expect(client.mediaUrl).toHaveBeenCalledTimes(2);
 });
 it("持续断网最多进行五次恢复探测，不产生请求堆积",async()=>{
  const client=recoveringClient();await render(client,zone({playback_id:"core-current"}));await networkError();vi.mocked(client.request).mockRejectedValue(new TypeError("Failed to fetch"));
  for(const delay of [1000,2000,4000,8000,16000,60000])await act(async()=>vi.advanceTimersByTime(delay));
  expect(vi.mocked(client.request).mock.calls.filter(([path])=>path==="/zones/zone-a")).toHaveLength(5);expect(client.mediaUrl).toHaveBeenCalledTimes(1);expect(controls.playing).toBe(false);
 });
 it("持续卡住八秒后恢复，进度继续前进时不重开音源",async()=>{
  const client=recoveringClient();await render(client,zone({playback_id:"core-current"}));const audio=TestAudio.instances[0];audio.currentTime=9;
  await act(async()=>audio.dispatchEvent(new Event("waiting")));await act(async()=>vi.advanceTimersByTime(7000));audio.currentTime=10;await act(async()=>audio.dispatchEvent(new Event("timeupdate")));await act(async()=>vi.advanceTimersByTime(2000));expect(client.mediaUrl).toHaveBeenCalledTimes(1);
  await act(async()=>audio.dispatchEvent(new Event("waiting")));await act(async()=>vi.advanceTimersByTime(9000));expect(client.mediaUrl).toHaveBeenCalledTimes(2);expect(audio.currentTime).toBe(10);
 });
 it("原生上游失败换新本机实例续播，设备错误不会自动重试",async()=>{
  native.enabled=true;const client=recoveringClient();await render(client,zone({playback_id:"core-current"}));const first=native.invoke.mock.calls.find(([command])=>command==="native_play")![1].playbackId;
  await act(async()=>native.listen.mock.calls.at(-1)![0]({track_id:"song-a",playback_id:first,status:"error",position:21,source_failed:true,error:"decoder interrupted"}));await act(async()=>vi.advanceTimersByTime(1000));
  const calls=native.invoke.mock.calls.filter(([command])=>command==="native_play");expect(calls).toHaveLength(2);expect(calls[1][1].position).toBe(21);expect(calls[1][1].playbackId).not.toBe(first);
  await act(async()=>native.listen.mock.calls.at(-1)![0]({track_id:"song-a",playback_id:calls[1][1].playbackId,status:"error",position:22,source_failed:false,error:"DAC disconnected"}));await act(async()=>vi.advanceTimersByTime(60000));expect(native.invoke.mock.calls.filter(([command])=>command==="native_play")).toHaveLength(2);
 });
 it("原生预加载连接失败也可恢复，不重放旧定位或调音量命令",async()=>{
  native.enabled=true;const client=recoveringClient();let starts=0;native.invoke.mockImplementation(async(command:string)=>{if(command==="native_play"&&starts++===0)throw new Error("无法连接核心");return {status:"stopped"};});
  await render(client,zone({playback_id:"core-current",position:13,volume:.25,muted:true,status:"paused"}));
  vi.mocked(client.request).mockResolvedValue(zone({playback_id:"core-current",position:13,volume:.25,muted:true,status:"paused"}));await act(async()=>vi.advanceTimersByTime(1000));
  const plays=native.invoke.mock.calls.filter(([command])=>command==="native_play");expect(plays).toHaveLength(2);expect(plays[1][1]).toMatchObject({position:13,volume:.25,muted:true,paused:true});expect(native.invoke.mock.calls.some(([command])=>command==="native_seek")).toBe(false);
 });
 it("明确设备失效保留消费位置并提示手动恢复，不进入音源自动重试",async()=>{
  native.enabled=true;const client=recoveringClient();await render(client,zone({playback_id:"core-current"}));
  const id=native.invoke.mock.calls.find(([command])=>command==="native_play")![1].playbackId;
  await act(async()=>native.listen.mock.calls.at(-1)![0]({track_id:"song-a",playback_id:id,status:"error",position:21.25,device_failed:true,source_failed:true,error:"output unavailable"}));
  expect(controls.position).toBe(21.25);expect(controls.playing).toBe(false);expect(report).toHaveBeenCalledWith(expect.stringContaining("设备恢复后点击播放"));
  await act(async()=>vi.advanceTimersByTime(60000));expect(native.invoke.mock.calls.filter(([command])=>command==="native_play")).toHaveLength(1);
 });
 it("无核心实例的原生设备等待在明确播放时重新请求持久恢复，暂停控制不打开输出",async()=>{
  native.enabled=true;endpoint.capabilities={continuous_pcm:true,continuous_recovery:true};const client=api();
  await render(client,zone({playback_id:null,status:"paused",volume:.17,muted:true}));
  const restores=()=>native.invoke.mock.calls.filter(([command])=>command==="native_sequence_restore");expect(restores()).toHaveLength(1);
  await render(client,zone({playback_id:null,status:"paused",volume:.09,muted:true}));expect(restores()).toHaveLength(1);
  await render(client,zone({playback_id:null,status:"playing",volume:.09,muted:true}));expect(restores()).toHaveLength(2);
  expect(native.invoke.mock.calls.filter(([command])=>command==="native_play")).toHaveLength(0);
  // A stopped snapshot also occurs after Core restart; Core decides whether the saved lease was revoked.
  await render(client,zone({playback_id:null,status:"stopped"}));expect(restores()).toHaveLength(3);
  expect(native.invoke.mock.calls.filter(([command])=>command==="native_play")).toHaveLength(0);
 });
 it("恢复响应迟到时保留已经确认的更新控制，忽略同实例旧进度",async()=>{
  const client=recoveringClient(),response=deferred<Zone>();await render(client,zone({playback_id:"core-current",updated_at:1}));await networkError(19);vi.mocked(client.request).mockReturnValue(response.promise);await act(async()=>vi.advanceTimersByTime(1000));
  await render(client,zone({playback_id:"core-current",updated_at:3,status:"paused",volume:.2,muted:true}));await act(async()=>response.resolve(zone({playback_id:"core-current",updated_at:2})));
  expect(TestAudio.instances[0].paused).toBe(true);expect(TestAudio.instances[0].volume).toBe(.2);expect(TestAudio.instances[0].muted).toBe(true);
  TestAudio.instances[0].readyState=4;await render(client,zone({playback_id:"core-current",updated_at:4,position:2,status:"paused",volume:.2,muted:true}));expect(TestAudio.instances[0].currentTime).toBe(19);
 });
 it("再次加载尚未得到媒体头就失败时不把恢复位置重置为零",async()=>{
  const client=recoveringClient();await render(client,zone({playback_id:"core-current"}));await networkError(19);await act(async()=>vi.advanceTimersByTime(1000));await networkError(0);await act(async()=>vi.advanceTimersByTime(2000));
  expect(client.mediaUrl).toHaveBeenCalledTimes(3);expect(TestAudio.instances[0].currentTime).toBe(19);
 });
 it("断流期间的伪 ended 不推进队列，恢复后的真实结束仍记一次",async()=>{
  const client=recoveringClient();await render(client,zone({playback_id:"core-current"}));await networkError(59);const audio=TestAudio.instances[0];audio.ended=true;await act(async()=>audio.dispatchEvent(new Event("ended")));expect(client.post).not.toHaveBeenCalled();
  await act(async()=>vi.advanceTimersByTime(1000));audio.error=null;audio.currentTime=60;audio.ended=true;await act(async()=>audio.dispatchEvent(new Event("ended")));expect(client.post).toHaveBeenCalledWith("/zones/zone-a/ended",{track_id:"song-a",playback_id:"core-current",position:60});
 });
 it("首次媒体请求被断开并返回不支持错误时先确认网络，不降级",async()=>{
  const client=recoveringClient();vi.stubGlobal("fetch",vi.fn().mockRejectedValue(new TypeError("Failed to fetch")));await render(client,zone({playback_id:"core-current"}));const audio=TestAudio.instances[0];audio.error={code:4,message:"source unavailable"};
  await act(async()=>audio.dispatchEvent(new Event("error")));expect(controls.adaptive).toBe(0);await act(async()=>vi.advanceTimersByTime(1000));expect(client.mediaUrl).toHaveBeenCalledTimes(2);expect(audio.src).not.toContain("transcode");
 });
 it("媒体可达但格式不支持时仍不静默降级为 AAC",async()=>{
  const client=recoveringClient();vi.stubGlobal("fetch",vi.fn().mockResolvedValue(new Response(null,{status:200})));await render(client,zone({playback_id:"core-current"}));const audio=TestAudio.instances[0];audio.error={code:4,message:"codec unavailable"};
  await act(async()=>audio.dispatchEvent(new Event("error")));await act(async()=>vi.advanceTimersByTime(1000));await act(async()=>audio.dispatchEvent(new Event("error")));expect(controls.adaptive).toBe(0);expect(audio.src).not.toContain("transcode");
 });
 it("同一位置连续 waiting 不误计为多次弱网缓冲",async()=>{
  const client=recoveringClient();await render(client,zone({playback_id:"core-current"}));for(let i=0;i<3;i++)await act(async()=>TestAudio.instances[0].dispatchEvent(new Event("waiting")));expect(controls.adaptive).toBe(0);await act(async()=>vi.advanceTimersByTime(9000));expect(client.mediaUrl).toHaveBeenCalledTimes(2);
 });
 it("初次票据请求断网也能在恢复后开始原格式播放",async()=>{
  const client=recoveringClient();vi.mocked(client.mediaUrl).mockRejectedValueOnce(new TypeError("Failed to fetch"));await render(client,zone({playback_id:"core-current"}));await act(async()=>vi.advanceTimersByTime(1000));expect(client.mediaUrl).toHaveBeenCalledTimes(2);expect(controls.adaptive).toBe(0);
 });
 it("原格式恢复使用实际已播放位置，不转码或丢失进度",async()=>{
  const client=recoveringClient();await render(client,zone({playback_id:"core-current",position:5}));const audio=TestAudio.instances[0];
  audio.currentTime=9;audio.error={code:2,message:"network interruption"};await act(async()=>audio.dispatchEvent(new Event("error")));await act(async()=>vi.advanceTimersByTime(1000));
  expect(audio.currentTime).toBe(9);expect(controls.position).toBe(9);expect(controls.adaptive).toBe(0);expect(audio.src).not.toContain("transcode");
 });
 it("更换核心连接后旧恢复的401不能影响新连接",async()=>{
  const old=recoveringClient(),response=deferred<Zone>();await render(old,zone({playback_id:"core-current"}));await networkError();vi.mocked(old.request).mockReturnValue(response.promise);await act(async()=>vi.advanceTimersByTime(1000));
  const next=recoveringClient();await render(next,zone({playback_id:"core-current"}));report.mockClear();await act(async()=>response.reject(new ApiError(401,"expired","旧会话已过期")));
  expect(report).not.toHaveBeenCalled();expect(recovered).not.toHaveBeenCalled();expect(next.mediaUrl).toHaveBeenCalledTimes(1);
 });
});
describe("浏览器播放地址准备期间的控制",()=>{
 it("采用最新暂停、音量、静音和定位，不恢复请求发出时的状态",async()=>{
  const client=api(),ticket=deferred<string>();vi.mocked(client.mediaUrl).mockReturnValue(ticket.promise);
  await render(client,zone());expect(controls.playing).toBe(true);await render(client,zone({status:"paused",volume:.2,muted:true,position:19}));expect(controls.playing).toBe(false);
  await act(async()=>ticket.resolve("http://qa-core/media/song-a"));const audio=TestAudio.instances[0];
  expect(audio.volume).toBe(.2);expect(audio.muted).toBe(true);expect(audio.currentTime).toBe(19);expect(audio.play).not.toHaveBeenCalled();expect(host.textContent).toBe("19");
 });
 it("加载期间从暂停改为播放会在地址就绪后开始",async()=>{
  const client=api(),ticket=deferred<string>();vi.mocked(client.mediaUrl).mockReturnValue(ticket.promise);
  await render(client,zone({status:"paused"}));await render(client,zone({volume:.3}));
  await act(async()=>ticket.resolve("http://qa-core/media/song-a"));expect(TestAudio.instances[0].play).toHaveBeenCalledTimes(1);expect(TestAudio.instances[0].volume).toBe(.3);
 });
 it("本机定位意图先于核心应答保留到地址就绪",async()=>{
  const client=api(),ticket=deferred<string>();vi.mocked(client.mediaUrl).mockReturnValue(ticket.promise);
  await render(client,zone());await act(async()=>controls.seek(23));
  await act(async()=>ticket.resolve("http://qa-core/media/song-a"));expect(TestAudio.instances[0].currentTime).toBe(23);expect(host.textContent).toBe("23");
 });
 it("停止和换曲后迟到的地址不能重新发声",async()=>{
  const client=api(),first=deferred<string>(),second=deferred<string>();vi.mocked(client.mediaUrl).mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
  await render(client,zone());await render(client,zone({track:track("song-b")}));
  expect(vi.mocked(client.mediaUrl).mock.calls[0][1]?.aborted).toBe(true);
  await act(async()=>second.resolve("http://qa-core/media/song-b"));await act(async()=>first.resolve("http://qa-core/media/song-a"));
  expect(TestAudio.instances[0].src).toContain("song-b");expect(TestAudio.instances[0].play).toHaveBeenCalledTimes(1);
  await render(client,zone({track:track("song-b"),status:"stopped"}));expect(TestAudio.instances[0].src).toBe("");expect(TestAudio.instances[0].paused).toBe(true);
 });
 it("切入离线播放后丢弃待返回的在线地址",async()=>{
  const client=api(),ticket=deferred<string>();vi.mocked(client.mediaUrl).mockReturnValue(ticket.promise);
  await render(client,zone());await act(async()=>window.dispatchEvent(new Event("rmusic-offline")));
  await act(async()=>ticket.resolve("http://qa-core/media/song-a"));expect(TestAudio.instances[0].src).toBe("");expect(TestAudio.instances[0].play).not.toHaveBeenCalled();
 });
 it("原格式报错后仍能定位暂停且不转码",async()=>{
  const client=api(),second=deferred<string>();vi.mocked(client.mediaUrl).mockResolvedValueOnce("http://qa-core/api/v1/media/song-a?ticket=fixture").mockReturnValueOnce(second.promise);
  await render(client,zone({position:5}));await act(async()=>TestAudio.instances[0].dispatchEvent(new Event("error")));
  TestAudio.instances[0].readyState=1;await render(client,zone({position:31,status:"paused",muted:true}));await act(async()=>second.resolve("http://qa-core/api/v1/media/song-a?ticket=fixture"));
  expect(TestAudio.instances[0].currentTime).toBe(31);expect(TestAudio.instances[0].src).not.toContain("transcode");expect(TestAudio.instances[0].paused).toBe(true);expect(host.textContent).toBe("31");
 });
 it("旧播放尝试迟到的权限错误不覆盖停止后的界面",async()=>{
  const client=api(),play=deferred<void>();vi.mocked(client.mediaUrl).mockResolvedValue("http://qa-core/media/song-a");
  await render(client,zone({status:"paused"}));TestAudio.instances[0].play.mockReturnValue(play.promise);
  await render(client,zone());await render(client,zone({status:"stopped"}));
  await act(async()=>play.reject(new DOMException("old permission failure","NotAllowedError")));expect(report).not.toHaveBeenCalled();
 });
});
describe("原生预加载的迟到任务",()=>{
 it("点击离线歌曲先取消在线准备，不等待离线解码就绪",async()=>{
  native.enabled=true;const client=api(),profile=deferred<{id:string}>(),offline=deferred<void>();native.profile.mockReturnValue(profile.promise);
  native.invoke.mockImplementation((command:string)=>Promise.resolve(command==="native_cache_status"?{entries:[{id:"cache-song",track_id:"offline-song",title:"离线歌曲",complete:true,format:"flac",sample_rate:96000,channels:2,total_bytes:100}]}:command==="native_offline_play"?offline.promise:{status:"stopped"}));
  await act(async()=>root.render(<><Harness client={client} current={zone()}/><NativeOffline api={{baseUrl:"http://core",token:"qa"}} onError={report}/></>));
  await act(async()=>host.querySelector<HTMLButtonElement>(".download-list button")!.click());
  await act(async()=>profile.resolve({id:"profile"}));expect(native.invoke.mock.calls.some(([command])=>command==="native_play")).toBe(false);
  await act(async()=>offline.resolve());expect(host.textContent).toContain("离线歌曲");
 });
 it("换曲后忽略旧 native_play 的失败",async()=>{
  native.enabled=true;const client=api(),old=deferred<void>();
  native.invoke.mockImplementation((command:string,args?:{track?:Track})=>command==="native_play"&&args?.track?.id==="song-a"?old.promise:Promise.resolve(command==="native_status"?{status:"stopped"}:undefined));
  await render(client,zone());await render(client,zone({track:track("song-b")}));
  const oldRequest=native.invoke.mock.calls.find(([command])=>command==="native_play")![1];
  expect(oldRequest.requestId).toBe(oldRequest.playbackId);expect(native.invoke).toHaveBeenCalledWith("native_request_cancel",{requestId:oldRequest.requestId});
  await act(async()=>old.reject(new Error("旧音源超时")));expect(report).not.toHaveBeenCalled();
 });
 it("立即中断后旧 profile 不能开始在线输出",async()=>{
  native.enabled=true;const client=api(),profile=deferred<{id:string}>();native.profile.mockReturnValue(profile.promise);
  await render(client,zone());await act(async()=>controls.interrupt());await act(async()=>profile.resolve({id:"profile"}));
  expect(native.invoke.mock.calls.some(([command])=>command==="native_play")).toBe(false);
 });
 it("离线切换取消旧 profile 且不会发起在线输出",async()=>{
  native.enabled=true;const client=api(),profile=deferred<{id:string}>();native.profile.mockReturnValue(profile.promise);
  await render(client,zone());await act(async()=>window.dispatchEvent(new Event("rmusic-offline")));await act(async()=>profile.resolve({id:"profile"}));
  expect(native.invoke.mock.calls.some(([command])=>command==="native_play")).toBe(false);
 });
});
describe("原生播放实例隔离",()=>{
 it("核心停止或重连后的新状态不能中断正在播放的离线实例",async()=>{
  native.enabled=true;const client=api();await render(client,zone());native.invoke.mockClear();
  await act(async()=>window.dispatchEvent(new CustomEvent("rmusic-offline",{detail:{playbackId:"offline-current",trackId:"offline-song"}})));
  vi.mocked(client.get).mockClear();
  await render(client,zone({status:"stopped",playback_id:null}));await render(client,zone({playback_id:"core-reconnected"}));
  expect(native.invoke.mock.calls.some(([command])=>command==="native_stop"||command==="native_play")).toBe(false);
  expect(client.get).not.toHaveBeenCalled();
  await act(async()=>native.listen.mock.calls.at(-1)![0]({track_id:"offline-song",playback_id:"offline-current",status:"playing",position:9}));expect(controls.nativeState.track_id).toBe("offline-song");
 });
 it("浏览器离线音乐不因核心新实例而自动切回在线播放",async()=>{
  const client=api();vi.mocked(client.mediaUrl).mockResolvedValue("http://qa-core/media/song-a");await render(client,zone());
  await act(async()=>window.dispatchEvent(new Event("rmusic-offline")));await render(client,zone({playback_id:"core-reconnected"}));expect(client.mediaUrl).toHaveBeenCalledTimes(1);
 });
 it("WebView 重新加载时保留 Rust 中仍在播放的完整离线文件",async()=>{
  native.enabled=true;const client=api();native.invoke.mockImplementation(async(command:string)=>command==="native_status"?{offline:true,cache_id:"cached",track_id:"offline-song",playback_id:"offline-current",status:"playing",position:9,paused:false}:undefined);
  await render(client,undefined as unknown as Zone);expect(native.invoke.mock.calls.some(([command])=>command==="native_stop")).toBe(false);expect(controls.nativeState.track_id).toBe("offline-song");
 });
 function playbackId(){return native.invoke.mock.calls.filter(([command])=>command==="native_play").at(-1)?.[1]?.playbackId as string;}
 async function emit(event:NativePlayback){await act(async()=>native.listen.mock.calls.at(-1)![0](event));}
 it("同曲重新加载后忽略旧实例的播放、错误和结束事件",async()=>{
  native.enabled=true;const client=api();await render(client,zone());const old=playbackId();
  await render(client,zone({playback_id:"core-reloaded"}));const current=playbackId();expect(old).toBeTruthy();expect(current).not.toBe(old);
  await emit({track_id:"song-a",playback_id:old,status:"playing",position:47});
  await emit({track_id:"song-a",playback_id:old,status:"error",error:"旧设备已关闭"});
  await emit({track_id:"song-a",playback_id:old,status:"ended",position:60});
  expect(controls.nativeState).toEqual({});expect(report).not.toHaveBeenCalled();expect(client.post).not.toHaveBeenCalled();
  await emit({track_id:"song-a",playback_id:current,status:"error",error:"当前设备错误"});expect(report).toHaveBeenCalledWith("当前设备错误");
 });
 it("切换核心后不接受另一核心的同曲实例",async()=>{
  native.enabled=true;const first=api();await render(first,zone());const old=playbackId();const second=api();second.baseUrl="http://other-core";
  await render(second,zone());await emit({track_id:"song-a",playback_id:old,status:"ended",position:60});
  expect(first.post).not.toHaveBeenCalled();expect(second.post).not.toHaveBeenCalled();
 });
 it("中断立即屏蔽旧事件，即使核心尚未应答停止",async()=>{
  native.enabled=true;const client=api();await render(client,zone());const old=playbackId();await act(async()=>controls.interrupt());
  await emit({track_id:"song-a",playback_id:old,status:"error",error:"中断后的旧错误"});await emit({track_id:"song-a",playback_id:old,status:"ended",position:60});
  expect(report).not.toHaveBeenCalled();expect(client.post).not.toHaveBeenCalled();expect(controls.playing).toBe(false);
 });
 it("同曲重建后旧轮询成功与失败不更新进度或报错",async()=>{
  native.enabled=true;const client=api(),old=deferred<NativePlayback>(),failed=deferred<NativePlayback>();let calls=0;
  native.invoke.mockImplementation((command:string)=>command==="native_status"?(++calls===1?old.promise:failed.promise):Promise.resolve());
  await render(client,zone());const id=playbackId();await act(async()=>vi.advanceTimersByTime(1500));
  expect(calls).toBe(1);
  await render(client,zone({playback_id:"core-reloaded"}));
  await act(async()=>old.resolve({track_id:"song-a",playback_id:id,position:48,paused:false,status:"playing"}));
  await act(async()=>vi.advanceTimersByTime(500));
  expect(calls).toBe(2);
  await render(client,zone({playback_id:"core-reloaded-again"}));
  await act(async()=>failed.reject(new Error("旧轮询失败")));
  expect(controls.position).toBe(0);expect(client.post).not.toHaveBeenCalled();expect(report).not.toHaveBeenCalled();
 });
 it("定位更换实例且拒绝旧进度，当前进度仍可上报",async()=>{
  native.enabled=true;const client=api();await render(client,zone());const old=playbackId();await act(async()=>controls.seek(25));
  const seek=native.invoke.mock.calls.find(([command])=>command==="native_seek")![1];expect(seek.playbackId).toBeTruthy();expect(seek.playbackId).not.toBe(old);
  await emit({track_id:"song-a",playback_id:old,status:"ended",position:60});expect(client.post).not.toHaveBeenCalled();
  native.invoke.mockImplementation(async(command:string)=>command==="native_status"?{track_id:"song-a",playback_id:seek.playbackId,status:"playing",position:26,paused:false}:undefined);
  await act(async()=>vi.advanceTimersByTime(500));expect(controls.position).toBe(26);expect(client.post).toHaveBeenCalledWith("/zones/zone-a/progress",{track_id:"song-a",position:26});
 });
 it("profile 准备期间的定位保留给首次原生播放",async()=>{
  native.enabled=true;const client=api(),profile=deferred<{id:string}>();native.profile.mockReturnValue(profile.promise);await render(client,zone());
  await act(async()=>controls.seek(23));expect(native.invoke.mock.calls.some(([command])=>command==="native_seek")).toBe(false);
  await act(async()=>profile.resolve({id:"profile"}));expect(native.invoke.mock.calls.find(([command])=>command==="native_play")![1].position).toBe(23);
 });
 it("离线入口只接受所选离线实例，不接受在线结束事件",async()=>{
  native.enabled=true;const client=api();native.invoke.mockImplementation(async(command:string)=>command==="native_cache_status"?{entries:[{id:"cached",track_id:"offline-song",title:"离线歌曲",complete:true,format:"flac",sample_rate:96000,channels:2,total_bytes:100}]}:command==="native_status"?{status:"stopped"}:undefined);
  await act(async()=>root.render(<><Harness client={client} current={zone()}/><NativeOffline api={{baseUrl:"http://core",token:"qa"}} onError={report}/></>));const old=playbackId();
  await act(async()=>host.querySelector<HTMLButtonElement>(".download-list button")!.click());const offline=native.invoke.mock.calls.find(([command])=>command==="native_offline_play")![1];expect(offline.playbackId).toBeTruthy();
  await emit({track_id:"song-a",playback_id:old,status:"error",error:"旧在线错误"});expect(report).not.toHaveBeenCalled();
  await emit({track_id:"offline-song",playback_id:offline.playbackId,status:"playing",position:5});expect(controls.nativeState.track_id).toBe("offline-song");
  native.invoke.mockImplementation(async(command:string)=>command==="native_status"?{track_id:"offline-song",playback_id:offline.playbackId,status:"playing",position:6,paused:false}:undefined);
  await act(async()=>vi.advanceTimersByTime(500));expect(controls.position).toBe(6);expect(client.post).not.toHaveBeenCalledWith("/zones/zone-a/progress",expect.anything());
 });
 it("当前结束事件只推进一次，结束后的状态轮询不再显示正在播放",async()=>{
  native.enabled=true;const client=api();await render(client,zone());const id=playbackId();
  native.invoke.mockImplementation(async(command:string)=>command==="native_status"?{track_id:"song-a",playback_id:id,status:"ended",position:60,paused:false}:undefined);
  const ended:NativePlayback={track_id:"song-a",playback_id:id,status:"ended",position:60};await emit(ended);await emit(ended);await act(async()=>vi.advanceTimersByTime(500));
  expect(client.post).toHaveBeenCalledTimes(1);expect(client.post).toHaveBeenCalledWith("/zones/zone-a/ended",{track_id:"song-a",position:60});expect(controls.playing).toBe(false);
 });
 it("播放命令尚未返回时仍接受当前输出，旧结束应答不能重启新实例",async()=>{
  native.enabled=true;const client=api(),starting=deferred<void>(),ended=deferred<Zone>();
  native.invoke.mockImplementation((command:string)=>command==="native_play"?starting.promise:Promise.resolve({status:"stopped"}));vi.mocked(client.post).mockReturnValue(ended.promise);
  await render(client,zone());const first=playbackId();await emit({track_id:"song-a",playback_id:first,status:"playing",position:2});expect(controls.position).toBe(2);
  await emit({track_id:"song-a",playback_id:first,status:"ended",position:60});await render(client,zone({playback_id:"core-reloaded"}));const next=playbackId();
  await act(async()=>ended.resolve(zone()));expect(playbackId()).toBe(next);expect(native.invoke.mock.calls.filter(([command])=>command==="native_play")).toHaveLength(2);
  await act(async()=>starting.resolve());
 });
 it("状态轮询补回丢失的结束事件并防止结束后迟到的播放状态",async()=>{
  native.enabled=true;const client=api();await render(client,zone());const id=playbackId();
  native.invoke.mockImplementation(async(command:string)=>command==="native_status"?{track_id:"song-a",playback_id:id,status:"ended",position:60,paused:false}:undefined);
  await act(async()=>vi.advanceTimersByTime(1000));expect(client.post).toHaveBeenCalledTimes(1);expect(controls.playing).toBe(false);
  await emit({track_id:"song-a",playback_id:id,status:"playing",position:59});expect(controls.position).toBe(60);expect(controls.playing).toBe(false);
  await emit({track_id:"song-a",playback_id:id,status:"preparing",position:0});expect(controls.position).toBe(60);expect(controls.playing).toBe(false);
 });
 it("当前错误在事件和轮询中只提示一次，切换后清除旧信号链",async()=>{
  native.enabled=true;const client=api();await render(client,zone());const id=playbackId();
  const error:NativePlayback={track_id:"song-a",playback_id:id,status:"error",error:"当前设备断开"};await emit(error);native.invoke.mockImplementation(async()=>error);
  await act(async()=>vi.advanceTimersByTime(1000));expect(report).toHaveBeenCalledTimes(1);
  await render(client,zone({playback_id:"core-reloaded"}));expect(controls.nativeState).toEqual({});
 });
 it("离线预加载被另一首取代后不显示旧失败",async()=>{
  native.enabled=true;const client=api(),old=deferred<void>();
  native.invoke.mockImplementation((command:string)=>command==="native_cache_status"?Promise.resolve({entries:[{id:"cached",track_id:"offline-song",title:"离线歌曲",complete:true,format:"flac",sample_rate:96000,channels:2,total_bytes:100}]}):command==="native_offline_play"?old.promise:Promise.resolve({status:"stopped"}));
  await act(async()=>root.render(<><Harness client={client} current={zone()}/><NativeOffline api={{baseUrl:"http://core",token:"qa"}} onError={report}/></>));await act(async()=>host.querySelector<HTMLButtonElement>(".download-list button")!.click());
  await act(async()=>window.dispatchEvent(new Event("rmusic-online")));await act(async()=>old.reject(new Error("旧离线文件失败")));expect(report).not.toHaveBeenCalled();expect(host.querySelector(".offline-player")).toBeNull();
 });
});

describe("原生连续播放的后台归属",()=>{
 it("连续播放自然结束撤销核心实例时，不把归零回报误当作旧播放器定位",async()=>{
  native.enabled=true;endpoint.capabilities={continuous_pcm:true,continuous_recovery:true};const client=api();await render(client,zone({playback_id:"core-a"}));
  const id=native.invoke.mock.calls.find(([command])=>command==="native_play")![1].playbackId;
  const state:NativePlayback={core_managed:true,zone_id:"zone-a",core_playback_id:"core-a",playback_id:id,track_id:"song-a",status:"ended",position:4,paused:false};
  await act(async()=>native.listen.mock.calls.at(-1)![0](state));native.invoke.mockImplementation(async(command:string)=>command==="native_status"?state:undefined);
  await render(client,zone({status:"stopped",playback_id:null,position:0,updated_at:100}));
  expect(native.invoke.mock.calls.some(([command])=>command==="native_seek")).toBe(false);expect(report).not.toHaveBeenCalled();
 });
 it("持久恢复正忙时稍后重新接管，停止后不继续重试打开音频",async()=>{
  native.enabled=true;endpoint.capabilities={continuous_pcm:true,continuous_recovery:true};const client=api();let calls=0;
  native.invoke.mockImplementation(async(command:string)=>command==="native_play"&&++calls===1?{recovering:true}:command==="native_status"?{status:"stopped"}:undefined);
  await render(client,zone({playback_id:"core-a"}));expect(native.invoke.mock.calls.filter(([command])=>command==="native_play")).toHaveLength(1);
  await act(async()=>vi.advanceTimersByTime(250));expect(native.invoke.mock.calls.filter(([command])=>command==="native_play")).toHaveLength(2);
  native.invoke.mockImplementation(async(command:string)=>command==="native_play"?{recovering:true}:command==="native_status"?{status:"stopped"}:undefined);
  await render(client,zone({playback_id:"core-b"}));const before=native.invoke.mock.calls.filter(([command])=>command==="native_play").length;
  await render(client,zone({status:"stopped",playback_id:null}));await act(async()=>vi.advanceTimersByTime(1000));
  expect(native.invoke.mock.calls.filter(([command])=>command==="native_play")).toHaveLength(before);
 });
 it("核心重启的空实例先保留游标并请求恢复，不打开旧曲的单源播放器",async()=>{
  native.enabled=true;endpoint.capabilities={continuous_pcm:true,continuous_recovery:true};const client=api();const pending=deferred<{zone?:Zone}>();
  native.invoke.mockImplementation((command:string)=>command==="native_sequence_restore"?pending.promise:Promise.resolve(command==="native_status"?{status:"stopped"}:undefined));
  await render(client,zone({status:"stopped",playback_id:null}));expect(native.invoke).toHaveBeenCalledWith("native_stop",{preserveRecovery:true});
  expect(native.invoke.mock.calls.some(([command])=>command==="native_play")).toBe(false);
  const request=native.invoke.mock.calls.find(([command])=>command==="native_sequence_restore")![1];
  await render(client,zone({track:track("song-b"),playback_id:"new-user-play"}));
  expect(native.invoke).toHaveBeenCalledWith("native_request_cancel",{requestId:request.requestId});
  await act(async()=>pending.resolve({zone:zone({playback_id:"old-recovery"})}));expect(recovered).not.toHaveBeenCalled();
 });
 it("轮询仅为匹配音区和核心实例的已恢复工作线程重新绑定界面别名",async()=>{
  native.enabled=true;endpoint.capabilities={continuous_pcm:true,continuous_recovery:true};const client=api();let alias="native-recovered",plays=0;
  native.invoke.mockImplementation(async(command:string,args?:{playbackId?:string})=>{
    if(command==="native_play"){if(++plays>1)alias=args!.playbackId!;return;}
    if(command==="native_status")return {core_managed:true,zone_id:"zone-a",core_playback_id:"core-restored",track_id:"song-a",playback_id:alias,status:"playing",position:7,paused:false};
  });
  await render(client,zone({playback_id:"core-restored"}));await act(async()=>vi.advanceTimersByTime(500));
  expect(plays).toBe(2);expect(controls.position).toBe(7);expect(client.post).not.toHaveBeenCalled();
 });
 it("跨曲错误可用已听检查点恢复核心旧曲，轮询也可补回错误事件",async()=>{
  native.enabled=true;endpoint.capabilities={continuous_pcm:true};const client=api();vi.mocked(client.request).mockResolvedValue(zone({playback_id:"core-a",position:1}));
  await render(client,zone({playback_id:"core-a"}));const id=native.invoke.mock.calls.find(([command])=>command==="native_play")![1].playbackId;
  const failure:NativePlayback={track_id:"song-b",playback_id:id,core_playback_id:"core-b",core_managed:true,status:"error",connection_failed:true,position:.55,error:"handoff unavailable",recovery:[{playback_id:"core-a",track_id:"song-a",position:2},{playback_id:"core-b",track_id:"song-b",position:.5}]};
  native.invoke.mockImplementation(async(command:string)=>command==="native_status"?failure:undefined);
  await act(async()=>vi.advanceTimersByTime(500));expect(controls.position).toBe(2);expect(report).toHaveBeenCalledWith(expect.stringContaining("正在尝试续播"));
  await act(async()=>vi.advanceTimersByTime(1000));const plays=native.invoke.mock.calls.filter(([command])=>command==="native_play");expect(plays).toHaveLength(2);expect(plays[1][1]).toMatchObject({position:2,sequence:{playback_id:"core-a"}});expect(client.post).not.toHaveBeenCalled();
 });
 it("跨曲恢复检查点也必须匹配当前核心实例，旧定位和旧别名不能触发恢复",async()=>{
  native.enabled=true;endpoint.capabilities={continuous_pcm:true};const client=api();await render(client,zone({playback_id:"core-new"}));const id=native.invoke.mock.calls.find(([command])=>command==="native_play")![1].playbackId;
  const failure:NativePlayback={track_id:"song-b",playback_id:id,core_playback_id:"core-b",core_managed:true,status:"error",connection_failed:true,recovery:[{playback_id:"core-old",track_id:"song-a",position:2}]};
  await act(async()=>native.listen.mock.calls.at(-1)![0](failure));await act(async()=>native.listen.mock.calls.at(-1)![0]({...failure,playback_id:"old-local",recovery:[{playback_id:"core-new",track_id:"song-a",position:2}]}));await act(async()=>vi.advanceTimersByTime(10000));
  expect(report).not.toHaveBeenCalled();expect(controls.nativeState).toEqual({});expect(native.invoke.mock.calls.filter(([command])=>command==="native_play")).toHaveLength(1);
 });
 it("后台管理的进度和结束不由 WebView 再次上报",async()=>{
  native.enabled=true;endpoint.capabilities={continuous_pcm:true};const client=api();const current=zone({playback_id:"core-a"});await render(client,current);
  const request=native.invoke.mock.calls.find(([command])=>command==="native_play")![1];expect(request.sequence).toEqual({zone_id:"zone-a",endpoint_id:endpoint.id,playback_id:"core-a"});
  const event:NativePlayback={track_id:"song-a",playback_id:request.playbackId,core_playback_id:"core-a",core_managed:true,sequence_id:"device-run",status:"playing",position:5,paused:false};
  native.invoke.mockImplementation(async(command:string)=>command==="native_status"?event:undefined);
  await act(async()=>native.listen.mock.calls.at(-1)![0](event));await act(async()=>vi.advanceTimersByTime(6000));expect(controls.position).toBe(5);
  await act(async()=>native.listen.mock.calls.at(-1)![0]({...event,status:"ended",position:60}));expect(client.post).not.toHaveBeenCalled();expect(controls.playing).toBe(false);
 });
 it("本地别名相同也不接受同曲旧核心实例",async()=>{
  native.enabled=true;endpoint.capabilities={continuous_pcm:true};const client=api();await render(client,zone({playback_id:"core-new"}));
  const id=native.invoke.mock.calls.find(([command])=>command==="native_play")![1].playbackId;
  await act(async()=>native.listen.mock.calls.at(-1)![0]({track_id:"song-a",playback_id:id,core_playback_id:"core-old",core_managed:true,status:"error",error:"旧源错误"}));
  expect(controls.nativeState).toEqual({});expect(report).not.toHaveBeenCalled();expect(client.post).not.toHaveBeenCalled();
 });
 it("过期原生请求带回核心新曲时采用新音区，并只撤销当前音区计划",async()=>{
  native.enabled=true;endpoint.capabilities={continuous_pcm:true};const client=api();const next=zone({track:track("song-b"),playback_id:"core-b"});
  native.invoke.mockImplementation(async(command:string,args?:{track?:Track})=>command==="native_play"&&args?.track?.id==="song-a"?{zone:next}:undefined);
  await render(client,zone({playback_id:"core-a"}));expect(recovered).toHaveBeenCalledWith(next);
  const plays=native.invoke.mock.calls.filter(([command])=>command==="native_play");expect(plays).toHaveLength(2);expect(plays[1][1].sequence.playback_id).toBe("core-b");
  await act(async()=>window.dispatchEvent(new CustomEvent("rmusic-sequence-invalidated",{detail:{zone_id:"other-zone",plan_id:"other-plan"}})));
  expect(native.invoke.mock.calls.some(([command])=>command==="native_sequence_cancel")).toBe(false);
  await act(async()=>window.dispatchEvent(new CustomEvent("rmusic-sequence-invalidated",{detail:{zone_id:"zone-a",plan_id:"current-plan"}})));
  expect(native.invoke).toHaveBeenCalledWith("native_sequence_cancel",{zoneId:"zone-a",planId:"current-plan"});
 });
});

describe("桌面分组输出所有权",()=>{
 it("分组帧直接展示，WebView 不重复起播或计时，退出后恢复普通播放器",async()=>{
  native.enabled=true;endpoint.kind="wasapi";endpoint.capabilities={continuous_pcm:true,native_group_v1:true};
  const client=api();let state:NativePlayback={group_managed:true,core_managed:true,profile_id:"profile",zone_id:"zone-a",track_id:"song-a",status:"playing",position:3.25};
  native.invoke.mockImplementation(async(command:string)=>command==="native_status"?state:undefined);
  await render(client,zone({playback_id:"group-a"}));await act(async()=>vi.advanceTimersByTimeAsync(1000));
  expect(controls.local).toBe(true);expect(controls.position).toBe(3.25);expect(controls.playing).toBe(true);
  expect(native.invoke.mock.calls.some(([command])=>command==="native_play")).toBe(false);expect(client.post).not.toHaveBeenCalled();
  state={status:"stopped",group_managed:false};await act(async()=>vi.advanceTimersByTimeAsync(500));
  expect(native.invoke.mock.calls.filter(([command])=>command==="native_play")).toHaveLength(1);
 });
 it("切换核心后忽略旧分组状态，不覆盖新核心播放器",async()=>{
  native.enabled=true;endpoint.kind="wasapi";endpoint.capabilities={continuous_pcm:true,native_group_v1:true};
  const first=api(),second=api();second.baseUrl="http://second-core";const old=deferred<NativePlayback>();
  native.profile.mockImplementation(async(address:string)=>({id:address==="http://second-core"?"second":"profile"}));
  native.invoke.mockImplementation(async(command:string)=>command==="native_status"?old.promise:undefined);
  await render(first,zone({playback_id:"old"}));
  native.invoke.mockImplementation(async(command:string)=>command==="native_status"?{status:"stopped"}:undefined);
  await render(second,zone({playback_id:"new",track:track("song-b")}));
  await act(async()=>old.resolve({group_managed:true,profile_id:"profile",zone_id:"zone-a",track_id:"song-a",position:44,status:"playing"}));
  await act(async()=>vi.advanceTimersByTimeAsync(500));
  const plays=native.invoke.mock.calls.filter(([command])=>command==="native_play");expect(plays).toHaveLength(1);expect(plays[0][1].profileId).toBe("second");expect(controls.position).not.toBe(44);
 });
});
