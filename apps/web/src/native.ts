import { clientId } from "./clientId";
import {
  ApiError,
  RMusicClient,
  type AuthResult,
  type AccountImportJob,
  type Track,
  type Zone,
  type CoreAddress,
  type ConnectionMetrics,
  type ImpulseAsset,
  type OutputLimits,
} from "@r-music/sdk";
export const isNative =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
export const isAndroidClient=isNative&&/Android/i.test(navigator.userAgent);
export interface NativeProfile {
  id: string;
  name: string;
  address: string;
  certificate_sha256?: string;
  alternatives?: CoreAddress[];
  core_id?:string;
  core_public_key?:string;
}
export interface NativeDevice {
  id: string;
  name: string;
  default?: boolean;
  mix_rate?: number;
  channels?: number;
}
export interface OutputCapabilities {sample_rates?:number[];bit_depths?:number[];exclusive?:boolean;dsd?:string[];conversion_locations?:string[]}
export interface DeviceOutput {
 capabilities?:OutputCapabilities;device_volume?:{supported:boolean;value?:number};
 id:string;name:string;kind:string;online:boolean;configurable?:boolean;default?:boolean;in_use?:boolean;native_device_id?:string|null;mix_rate?:number;channels?:number;
 config?:NativeOutput&{enabled:boolean};endpoint?:import("@r-music/sdk").Endpoint;
}
export interface AudioDevice {id:string;name:string;platform:string;online:boolean;renameable?:boolean;outputs:DeviceOutput[]}
export interface NativeCacheEntry {
  document_uri?:string|null;
  document_tree?:string|null;
  profile_id?: string;
  title?: string | null;
  artist?: string | null;
  id: string;
  track_id: string;
  recording_id: string;
  format: string;
  total_bytes: number;
  complete: boolean;
  manual_download: boolean;
  ranges: [number, number][];
  sample_rate: number;
  channels: number;
  verified?: boolean;
  bit_depth?: number;
  bitrate?: number;
}
export interface NativePlayback {
  output_config?:NativeOutput;
  zone_id?:string;
  core_managed?:boolean;
  group_managed?:boolean;
  profile_id?:string;
  group_id?:string;
  core_playback_id?:string;
  sequence_id?:string;
  measured_version?:1|2;
  connection_failed?:boolean;
  device_failed?:boolean;
  http_status?:number|null;
  recovery?:{playback_id:string;track_id:string;position:number}[];
  offline?:boolean;
  cache_id?:string|null;
  duration?:number|null;
  volume?:number;
  muted?:boolean;
  status?: string;
  track_id?: string;
  playback_id?: string;
  position?: number;
  buffered_fraction?: number | null;
  buffered_until?: number | null;
  paused?: boolean;
  signal_path?: {
    output: string;
    sample_rate: number;
    channels: number;
    exclusive: boolean;
    bit_depth?: number;
    sample_type?: string;
    mixer_sample_rate?: number;
    bit_perfect_eligible: boolean;
    processing: string[];
  };
  meter?: { peak: number; clipped_samples: number };
  error?: string;
  source_failed?:boolean;
}

export function nativeBufferedEnd(state: NativePlayback, zone?: Zone): number | undefined {
  const track = zone?.track;
  if (!track?.duration || !zone?.playback_id || zone.buffering || zone.status === "stopped" ||
      state.playback_id !== zone.playback_id || state.track_id !== track.id || state.status !== "playing") return;
  const fraction = state.buffered_fraction ?? 0;
  const until = state.buffered_until ?? 0;
  const end = Math.max(fraction * track.duration, until);
  if (Number.isFinite(end) && end > 0) return Math.min(track.duration, end);
}

export interface NativeOutput extends OutputLimits {
  device_id?: string;
  exclusive: boolean;
  buffer_ms: number;
  dsd_output?: "disabled" | "dop" | "native";
  android_mode?: "system" | "usb_bitperfect" | "compatibility";
}
export async function invokeNative<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    const message =
      typeof error === "string"
        ? error
        : error instanceof Error
          ? error.message
          : String(error);
    const status = message.match(/HTTP\s+(\d+)/)?.[1];
    if (status)
      throw new ApiError(
        Number(status),
        "native_http",
        message.replace(/^HTTP\s+\d+:?\s*/, ""),
      );
    throw new Error(message);
  }
}
export async function nativeProfiles() {
  return invokeNative<NativeProfile[]>("profiles_list");
}
export async function saveNativeProfile(address: string, fingerprint?: string, alternatives?:CoreAddress[]):Promise<NativeProfile> {
  const profiles = await nativeProfiles();
  const existing = profiles.find(
    (p) => p.address.replace(/\/+$/, "") === address.replace(/\/+$/, ""),
  );
  const profile:NativeProfile = {
    id: existing?.id || clientId(),
    name: new URL(address).hostname,
    address: address.replace(/\/+$/, ""),
    certificate_sha256:
      fingerprint===undefined ? existing?.certificate_sha256 : fingerprint.replace(/[:\s]/g, "") || undefined,
    alternatives:alternatives ?? existing?.alternatives ?? [],
    core_id:existing?.core_id,
    core_public_key:existing?.core_public_key,
  };
  if (
    profile.certificate_sha256 &&
    !/^[a-fA-F0-9]{64}$/.test(profile.certificate_sha256)
  )
    throw new Error("证书 SHA-256 指纹必须为 64 位十六进制字符");
  await invokeNative("profiles_save", { profile });
  profilePromises.delete(address);
  return profile;
}
const profilePromises = new Map<string, Promise<NativeProfile>>();
export function nativeProfile(address: string) {
  let profile = profilePromises.get(address);
  if (!profile) {
    profile = nativeProfiles().then(
      (profiles) =>
        profiles.find(
          (p) => p.address.replace(/\/+$/, "") === address.replace(/\/+$/, ""),
        ) || saveNativeProfile(address),
    ).catch(error=>{profilePromises.delete(address);throw error;});
    profilePromises.set(address, profile);
  }
  return profile;
}
async function cancellable<T>(command:string,args:Record<string,unknown>,signal?:AbortSignal):Promise<T> {
  const requestId=signal?clientId():undefined;
  const pending=invokeNative<T>(command,{...(requestId?{requestId}:{}),...args});
  if(!signal)return await pending;
  return await new Promise<T>((resolve,reject)=>{
    const abort=()=>{invokeNative("native_request_cancel",{requestId}).catch(()=>{});reject(signal.reason??new DOMException("请求已取消","AbortError"));};
    signal.addEventListener("abort",abort,{once:true});
    pending.then(resolve,reject).finally(()=>signal.removeEventListener("abort",abort));
    if(signal.aborted)abort();
  });
}
export class NativeClient extends RMusicClient {
  override async connectionMetrics(speed:boolean,signal?:AbortSignal):Promise<ConnectionMetrics>{
    signal?.throwIfAborted();const profile=await nativeProfile(this.baseUrl);
    return await cancellable("native_connection_metrics",{profileId:profile.id,speed},signal);
  }
  override async uploadAccountArchive(job:string,file:Blob,signal?:AbortSignal):Promise<AccountImportJob> {
    signal?.throwIfAborted();const profile=await nativeProfile(this.baseUrl);
    const id=await invokeNative<string>("native_account_upload_begin",{profileId:profile.id,jobId:job,total:file.size});
    const abort=()=>{void invokeNative("native_account_upload_abort",{id}).catch(()=>{});};
    signal?.addEventListener("abort",abort,{once:true});
    try {
      for(let offset=0;offset<file.size;offset+=256*1024){
        signal?.throwIfAborted();const bytes=new Uint8Array(await file.slice(offset,offset+256*1024).arrayBuffer());
        const parts:string[]=[];for(let at=0;at<bytes.length;at+=8192)parts.push(String.fromCharCode(...bytes.subarray(at,at+8192)));
        await invokeNative("native_account_upload_chunk",{id,data:btoa(parts.join(""))});
      }
      signal?.throwIfAborted();return await invokeNative<AccountImportJob>("native_account_upload_finish",{id});
    }catch(error){abort();throw error;}finally{signal?.removeEventListener("abort",abort);}
  }

  override async importImpulse(bytes:Blob|ArrayBuffer,name="impulse.wav",signal?:AbortSignal):Promise<ImpulseAsset> {
    signal?.throwIfAborted();
    try {
      if((bytes instanceof Blob?bytes.size:bytes.byteLength)>4*1024**2)throw new Error("响应文件不能超过 4 MiB");
      const data=new Uint8Array(bytes instanceof Blob?await bytes.arrayBuffer():bytes);signal?.throwIfAborted();
      const parts:string[]=[];for(let i=0;i<data.length;i+=8192)parts.push(String.fromCharCode(...data.subarray(i,i+8192)));
      const encoded=btoa(parts.join(""));const profile=await nativeProfile(this.baseUrl);signal?.throwIfAborted();
      return await cancellable<ImpulseAsset>("native_impulse_import",{profileId:profile.id,name,data:encoded},signal);
    }finally{signal?.throwIfAborted();}
  }
  override async request<T>(
    path: string,
    options: RequestInit = {},
  ): Promise<T> {
    options.signal?.throwIfAborted();
    try {
      const profile = await nativeProfile(this.baseUrl);
      options.signal?.throwIfAborted();
      const body =
        typeof options.body === "string" ? JSON.parse(options.body) : undefined;
      if (path === "/auth/login")
        return await invokeNative<AuthResult>("native_login", {
          profileId: profile.id,
          username: body.username,
          password: body.password,
          client: body.client,
          totp: body.totp,
        }) as T;
      if (path === "/downloads" && options.method === "POST") {
        const track = await this.get<Track>(
          `/library/tracks/${encodeURIComponent(body.track_id)}`,
        );
        options.signal?.throwIfAborted();
        return await nativeDownload(this, track) as T;
      }
      return await cancellable<T>("native_request", {
        profileId: profile.id,
        method: options.method || "GET",
        path: `/api/v1${path}`,
        body,
      },options.signal??undefined);
    } finally {
      // Cancellation can race with completion; no stale result/error may escape.
      options.signal?.throwIfAborted();
    }
  }
}
export async function nativeCover(api: RMusicClient, path: string, signal?: AbortSignal) {
  const profile = await nativeProfile(api.baseUrl);
  const url = new URL(path, api.baseUrl);
  signal?.throwIfAborted();
  return cancellable<string>("native_cover", { profileId: profile.id, path: url.pathname }, signal);
}
export async function nativeDownload(api: RMusicClient, track: Track) {
  const profile = await nativeProfile(api.baseUrl);
  return invokeNative<NativeCacheEntry>("native_download", {
    profileId: profile.id,
    track,
  });
}
export async function listenNative(
  callback: (value: NativePlayback) => void,
  onCacheError: (error: string) => void,
) {
  const { listen } = await import("@tauri-apps/api/event");
  const [playback, cache] = await Promise.all([
    listen<NativePlayback>("native-playback", (event) =>
      callback(event.payload),
    ),
    listen<string>("native-cache-error", (event) =>
      onCacheError(event.payload),
    ),
  ]);
  return () => {
    playback();
    cache();
  };
}
export function nativeOutput(): NativeOutput {
  try {
    return (
      JSON.parse(localStorage.getItem("rmusic.output") || "null") || {
        exclusive: false,
        buffer_ms: 100,
      }
    );
  } catch {
    return { exclusive: false, buffer_ms: 100 };
  }
}

/** Android system bars follow the actual client scene; never alter audio routing. */
export function setWindowAppearance(color:string,light:boolean,navigationColor=color){
 const host=window as Window&{RMusicAppearance?:{set:(color:string,light:boolean)=>void;setBars?:(color:string,navigation:string,light:boolean)=>void}};if(!host.RMusicAppearance)return;
 const toHex=(value:string)=>{if(/^#[0-9a-f]{6}$/i.test(value))return value;const values=value.match(/^rgba?\(\s*(\d+)[, ]+\s*(\d+)[, ]+\s*(\d+)/);return values?"#"+values.slice(1,4).map(v=>Math.max(0,Math.min(255,Number(v))).toString(16).padStart(2,"0")).join(""):undefined;};
 const hex=toHex(color),navigation=toHex(navigationColor);if(!hex||!navigation)return;
 try{if(host.RMusicAppearance.setBars)host.RMusicAppearance.setBars(hex,navigation,light);else host.RMusicAppearance.set(hex,light);}catch{}
}
let sceneAppearance:{color:string;light:boolean;navigation?:string}|undefined;
export function setSceneWindowAppearance(color?:string,light=true,navigation?:string){sceneAppearance=color?{color,light,navigation}:undefined;restoreWindowAppearance();}
export function restoreWindowAppearance(){if(sceneAppearance)setWindowAppearance(sceneAppearance.color,sceneAppearance.light,sceneAppearance.navigation);else {const style=getComputedStyle(document.documentElement);setWindowAppearance(style.getPropertyValue("--bg").trim()||style.backgroundColor,document.documentElement.dataset.mode==="light");}}
