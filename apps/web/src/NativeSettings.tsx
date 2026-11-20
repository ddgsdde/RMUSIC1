import {AndroidQuality} from "./AndroidQuality";
import { useEffect, useState } from "react";
import type {RMusicClient} from "@r-music/sdk";
import {OutputLimitFields} from "./OutputLimits";
import { RefreshCw } from "lucide-react";
import {
  invokeNative,
  nativeOutput,
  type NativeDevice,
  type NativeOutput,
} from "./native";
export function NativeSettings({
  onError,
  notify,
  api,
}: {
  api:RMusicClient;
  onError: (error: unknown) => void;
  notify: (message: string) => void;
}) {
  const android = /Android/i.test(navigator.userAgent);
  const [devices, setDevices] = useState<NativeDevice[]>([]);
  const [output, setOutput] = useState<NativeOutput>(nativeOutput);
  const asio = output.device_id?.startsWith("asio:") === true;
  const dop = output.dsd_output === "dop";
  const nativeDsd = output.dsd_output === "native";
  const refresh = () =>
    invokeNative<NativeDevice[]>("native_devices")
      .then(setDevices)
      .catch(onError);
  useEffect(() => {
    refresh();
  }, []);
  if (android) return <><AndroidQuality onError={onError} notify={notify}/><section className="settings-section">
    <div className="section-heading"><h2>Android 本机输出</h2><button onClick={refresh}><RefreshCw size={15} />刷新设备</button></div>
    <label>音频设备<select value={output.device_id || ""} onChange={e => setOutput({...output,device_id:e.target.value || undefined})}>
      <option value="">系统默认输出</option>
      {devices.map(device => <option key={device.id} value={device.id}>{device.name}</option>)}
    </select></label>
    <label>输出方式<select value={output.android_mode || "system"} onChange={e => setOutput({...output,android_mode:e.target.value as NativeOutput["android_mode"]})}>
      <option value="system">系统输出</option><option value="usb_bitperfect">USB 原样请求 · Android 14+ 及兼容设备</option><option value="compatibility">PCM16 兼容输出</option>
    </select></label>
    <p>{output.android_mode === "usb_bitperfect" ? "需明确选择 USB 设备、音量 100% 且关闭处理。系统不接受原样格式时停止并提示；硬件逐位结果仍需实机验证。" : output.android_mode === "compatibility" ? "显式转换为 PCM16，并使用至少 250 ms 缓冲。系统仍可能混音或重采样。" : "应用默认不升频；系统可能混音或重采样。蓝牙编解码器由系统和耳机协商。"}</p>
    <label>缓冲时长 (ms)<input type="number" min={20} max={2000} step={10} value={output.buffer_ms} onChange={e => setOutput({...output,buffer_ms:Number(e.target.value)})} /></label>
    <OutputLimitFields value={output} onChange={setOutput}/>
    <p>遇到兼容性问题可手动切换输出方式；不会自动降级。支持连续队列、后台播放与系统媒体控制；进程终止后重新登录同一账号可恢复已保存的队列位置与控制状态；多房间分组尚未开放。</p>
    <button className="primary" onClick={() => {
      if (!Number.isFinite(output.buffer_ms) || output.buffer_ms < 20 || output.buffer_ms > 2000) {notify("缓冲时长应在 20–2000 ms 之间");return;}
      if (output.android_mode === "usb_bitperfect" && !output.device_id) {notify("USB 原样模式需要选择具体 USB 设备");return;}
      const config={...output,enabled:true,exclusive:output.android_mode === "usb_bitperfect",dsd_output:"disabled"};
      void api.put(`/outputs/${encodeURIComponent(localStorage.getItem("rmusic.endpoint-id")||"")}/config`,config).then(()=>{localStorage.setItem("rmusic.output",JSON.stringify(config));window.dispatchEvent(new Event("rmusic-output"));notify("输出设置已保存，下次起播生效");}).catch(onError);
    }}>保存本机输出</button>
  </section></>;
  return (
    <section className="settings-section">
      <div className="section-heading">
        <h2>Windows 本机输出</h2>
        <button onClick={refresh}>
          <RefreshCw size={15} />
          刷新设备
        </button>
      </div>
      <label>
        音频设备
        <select
          value={output.device_id || ""}
          onChange={(e) =>
            setOutput({ ...output, device_id: e.target.value || undefined,
              exclusive: e.target.value.startsWith("asio:") || output.exclusive })
          }
        >
          <option value="">系统默认输出</option>
          {devices.map((device) => (
            <option key={device.id} value={device.id}>
              {device.name}
              {device.default ? " · 默认" : ""}
            </option>
          ))}
        </select>
      </label>
      <label className="setting-row">
        <span>
          <strong>{asio ? "ASIO 驱动直连" : "独占模式"}</strong>
          <small>
            {asio ? "不经过 Windows 共享混音器；不支持的采样率或精度会报错" : "设备不支持歌曲格式时返回错误；共享模式使用 Windows 混音器"}
          </small>
        </span>
        <input
          className="toggle"
          type="checkbox"
          checked={asio || output.exclusive}
          disabled={asio || dop || nativeDsd}
          onChange={(e) =>
            setOutput({ ...output, exclusive: e.target.checked })
          }
        />
      </label>
      <label>
        缓冲时长 (ms)
        <input
          type="number"
          min={20}
          max={2000}
          step={10}
          value={output.buffer_ms}
          onChange={(e) =>
            setOutput({ ...output, buffer_ms: Number(e.target.value) })
          }
        />
      </label>
      <label>
        DSD 输出
        <select value={output.dsd_output || "disabled"} onChange={(e) =>
          setOutput({ ...output, dsd_output: e.target.value as NativeOutput["dsd_output"],
            exclusive: e.target.value !== "disabled" || asio || output.exclusive })}>
          <option value="disabled">关闭</option>
          <option value="dop">DoP · 设备需支持</option>
          <option value="native" disabled={!asio}>ASIO 原生 DSD · 驱动需支持</option>
        </select>
        {dop ? <small>选择支持 DoP 的具体设备；软件音量设为 100%，关闭 DSP 和淡入淡出，使用 DAC 调节音量。</small> : null}
        {nativeDsd ? <small>需要支持原生 DSD 的 ASIO 驱动；软件音量 100%，关闭 DSP 和淡入淡出，使用 DAC 调节音量。</small> : null}
      </label>
      <button
        className="primary"
        onClick={() => {
          if (dop && !output.device_id) {notify("DoP 需要选择具体的兼容设备");return;}
          if (nativeDsd && !asio) {notify("原生 DSD 需要选择具体的 ASIO 驱动");return;}
          if (output.buffer_ms < 20 || output.buffer_ms > 2000) {
            notify("缓冲时长应在 20–2000 ms 之间");
            return;
          }
          localStorage.setItem("rmusic.output", JSON.stringify(output));
          window.dispatchEvent(new Event("rmusic-output"));
          notify("输出设置已保存，下次起播生效");
        }}
      >
        保存本机输出
      </button>
    </section>
  );
}
