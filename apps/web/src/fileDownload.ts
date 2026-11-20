import type { RMusicClient } from "@r-music/sdk";
import {isNative,invokeNative,nativeProfile} from "./native";

export function downloadFilename(disposition: string | null, fallback: string) {
  const extended = disposition?.match(
    /filename\*\s*=\s*(?:"([^"]+)"|([^;]+))/i,
  );
  const encoded = (extended?.[1] || extended?.[2])
    ?.trim()
    .match(/^utf-8'[^']*'(.*)$/i);
  let filename =
    disposition
      ?.match(/filename\s*=\s*(?:"([^"]+)"|([^;]+))/i)
      ?.slice(1)
      .find(Boolean)
      ?.trim() || fallback;
  if (encoded) {
    try {
      filename = decodeURIComponent(encoded[1]);
    } catch {
      /* Keep the ordinary filename when malformed. */
    }
  }
  return filename.replace(/[\\/\0]/g, "_");
}

export async function saveApiFile(
  api: RMusicClient,
  path: string,
  fallback: string,
  options: {preferRequestedFilename?:boolean} = {},
) {
  if(isNative){const profile=await nativeProfile(api.baseUrl);await invokeNative("native_save_file",{profileId:profile.id,path:`/api/v1${path}`,filename:fallback});return;}
  const response = await api.fetchResource(api.url(`/api/v1${path}`), {
    headers: { Authorization: `Bearer ${api.token}` },
  });
  if (!response.ok)
    throw new Error(
      response.status === 409
        ? "文件仍在准备中，请稍后重试"
        : "文件下载失败，请检查连接与登录状态",
    );
  const blob = await response.blob();
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = downloadFilename(
    options.preferRequestedFilename ? null : response.headers.get("Content-Disposition"),
    fallback,
  );
  anchor.click();
  setTimeout(() => URL.revokeObjectURL(url), 60000);
}
