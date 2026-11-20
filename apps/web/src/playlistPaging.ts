import type { ProviderPlaylistTracks, Track } from "@r-music/sdk";

export const PLAYLIST_TRACK_LIMIT = 10000;

export async function collectPlaylistTracks(
  getPage: (
    page: number,
    signal?: AbortSignal,
  ) => Promise<ProviderPlaylistTracks>,
  signal: AbortSignal,
  onProgress: (count: number, total: number) => void,
  limit = PLAYLIST_TRACK_LIMIT,
) {
  const tracks: Track[] = [];
  const visited = new Set<number>();
  let page: number | null = 1;
  let total = 0;
  while (page !== null) {
    signal.throwIfAborted();
    if (visited.has(page))
      throw new Error("音源返回了重复分页，已停止读取歌单。");
    visited.add(page);
    const result = await getPage(page, signal);
    signal.throwIfAborted();
    total = result.total;
    const remaining = limit - tracks.length;
    tracks.push(...result.items.slice(0, remaining));
    onProgress(tracks.length, total);
    if (
      result.items.length > remaining ||
      (tracks.length === limit && result.next_page !== null)
    )
      return { tracks, total, truncated: true };
    page = result.next_page;
  }
  return { tracks, total, truncated: false };
}
