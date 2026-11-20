import { describe, expect, it } from "vitest";
import type { ProviderPlaylistTracks, Track } from "@r-music/sdk";
import { collectPlaylistTracks } from "./playlistPaging";

const track = (id: string) => ({ id }) as Track;
const page = (
  ids: string[],
  next_page: number | null,
  total = 7,
): ProviderPlaylistTracks => ({
  items: ids.map(track),
  next_page,
  total,
  playlist_id: "liked",
});

describe("provider playlist pagination", () => {
  it("follows actual next_page values and preserves provider totals", async () => {
    const seen: number[] = [];
    const result = await collectPlaylistTracks(
      async (number) => {
        seen.push(number);
        return number === 1 ? page(["a"], 4, 900) : page(["b"], null, 900);
      },
      new AbortController().signal,
      () => {},
    );
    expect(seen).toEqual([1, 4]);
    expect(result).toEqual({
      tracks: [track("a"), track("b")],
      total: 900,
      truncated: false,
    });
  });
  it("stops at the explicit cap without claiming the remaining songs were loaded", async () => {
    const result = await collectPlaylistTracks(
      async () => page(["a", "b", "c"], 2, 20),
      new AbortController().signal,
      () => {},
      2,
    );
    expect(result).toEqual({
      tracks: [track("a"), track("b")],
      total: 20,
      truncated: true,
    });
  });
  it("reports a full final page as complete when no following page exists", async () => {
    const result = await collectPlaylistTracks(
      async () => page(["a", "b"], null, 2),
      new AbortController().signal,
      () => {},
      2,
    );
    expect(result.truncated).toBe(false);
  });
  it("rejects cancellation after an in-flight native request finishes", async () => {
    const controller = new AbortController();
    let progress = false;
    await expect(
      collectPlaylistTracks(
        async () => {
          controller.abort();
          return page(["a"], 2);
        },
        controller.signal,
        () => {
          progress = true;
        },
      ),
    ).rejects.toMatchObject({ name: "AbortError" });
    expect(progress).toBe(false);
  });
  it("rejects cyclic pages instead of repeatedly fetching a partial playlist", async () => {
    await expect(
      collectPlaylistTracks(
        async () => page(["a"], 1),
        new AbortController().signal,
        () => {},
      ),
    ).rejects.toThrow("重复分页");
  });
  it("preserves intentional repeated tracks in playlist order", async () => {
    const result = await collectPlaylistTracks(
      async (number) =>
        number === 1 ? page(["a"], 2) : page(["a", "b"], null),
      new AbortController().signal,
      () => {},
    );
    expect(result.tracks.map((item) => item.id)).toEqual(["a", "a", "b"]);
  });
});
