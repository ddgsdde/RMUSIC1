// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { RMusicClient, type Track } from "@r-music/sdk";
import { DailyPlaylist, PersonalRecommendations } from "./PersonalRecommendations";
import { SongButtons, SongCollectionProvider, SongMenuButton } from "./SongCollection";
import { TrackContextMenuProvider } from "./TrackContextMenu";

const seed = {
  id: "local:a",
  artist: "艺人",
  title: "当前歌曲",
  album: "甲",
  source: "local",
  available: true,
  format: "flac",
  duration: 180,
  sample_rate: 44100,
  bit_depth: 16,
  channels: 2,
  bitrate: 0,
  recording_id: "a",
} satisfies Track;
const match = { ...seed, id: "other:b", recording_id: "b", title: "新发现" };
const daily = { date: "2026-11-19", items: [match], reasons: {} };
let root: Root, host: HTMLDivElement;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
async function render(api: RMusicClient, play: () => Promise<void>) {
  await act(async () =>
    root.render(
      <PersonalRecommendations api={api} play={play} canPlay onError={vi.fn()}>
        <SongCollectionProvider api={api} plugins={[]} notify={vi.fn()} onError={vi.fn()}>
          <TrackContextMenuProvider
            api={api}
            currentTrackId={seed.id}
            location="discover"
            play={play}
            enqueue={vi.fn()}
            details={vi.fn()}
            onError={vi.fn()}
          >
            <DailyPlaylist api={api} play={play} canPlay onError={vi.fn()} onOpen={vi.fn()} />
            <SongButtons track={seed} current />
            <SongMenuButton track={match} />
            <input aria-label="搜索" />
          </TrackContextMenuProvider>
        </SongCollectionProvider>
      </PersonalRecommendations>,
    ),
  );
}
function button(text: string) {
  return [...host.querySelectorAll("button")].find(
    (b) => b.textContent === text || b.getAttribute("aria-label") === text,
  )!;
}
function fixture(fetchSimilar: () => Promise<typeof daily>) {
  const api = new RMusicClient("http://core", "test");
  let favorites: string[] = [];
  vi.spyOn(api, "get").mockImplementation(async (path) =>
    path === "/favorites" ? { track_ids: favorites } : { items: [] },
  );
  const request = vi.spyOn(api, "request").mockImplementation(async (path, opts) => {
    if (path === "/recommendations/daily") return daily;
    if (path === "/recommendations/similar") return fetchSimilar();
    if (path === "/favorites/local%3Aa") {
      favorites = JSON.parse(String(opts?.body)).liked ? [seed.id] : [];
      return { track_ids: favorites };
    }
    throw new Error(path);
  });
  return { api, request };
}
it("首页只读取每日歌单，当前曲目菜单按需找相似，点歌才播放", async () => {
  const { api, request } = fixture(async () => daily),
    play = vi.fn(async () => {});
  await render(api, play);
  expect(host.querySelector(".personal-daily")?.textContent).toContain("每日歌单");
  expect(request.mock.calls.map(([path]) => path)).toEqual(["/recommendations/daily"]);
  await act(async () => button("更多歌曲操作 新发现").click());
  expect(button("找相似")).toBeUndefined();
  await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })));
  await act(async () => button("更多歌曲操作 当前歌曲").click());
  await act(async () => button("找相似").click());
  expect(request.mock.calls.at(-1)).toEqual([
    "/recommendations/similar",
    expect.objectContaining({ method: "POST", body: JSON.stringify({ track_id: seed.id }) }),
  ]);
  expect(play).not.toHaveBeenCalled();
  expect(host.querySelector('[role="dialog"]')).toBeNull();
  await act(async () => (host.querySelector("aside .personal-song") as HTMLButtonElement).click());
  expect(play).toHaveBeenCalledWith(match, [match]);
  expect(host.querySelector("aside")).toBeNull();
});
it("收藏后静默等候，结果不抢焦点，取消收藏丢弃迟到的结果", async () => {
  let finish!: (v: typeof daily) => void;
  const { api } = fixture(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    ),
    play = vi.fn(async () => {});
  await render(api, play);
  await act(async () => button("喜欢 当前歌曲").click());
  expect(host.querySelector("aside")).toBeNull();
  const search = host.querySelector("input")!;
  search.focus();
  await act(async () => finish(daily));
  expect(host.querySelector("aside")?.textContent).toContain("新发现");
  expect(document.activeElement).toBe(search);
  expect(play).not.toHaveBeenCalled();
  await act(async () => button("取消喜欢 当前歌曲").click());
  expect(host.querySelector("aside")).toBeNull();
  await act(async () => button("喜欢 当前歌曲").click());
  await act(async () => button("取消喜欢 当前歌曲").click());
  await act(async () => finish(daily));
  expect(host.querySelector("aside")).toBeNull();
  expect(play).not.toHaveBeenCalled();
});
