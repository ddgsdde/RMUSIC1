import { afterEach, describe, expect, it, vi } from "vitest";
import { RMusicClient } from "@r-music/sdk";

afterEach(() => vi.unstubAllGlobals());
describe("provider and discovery SDK transport", () => {
  it("encodes provider and playlist IDs and preserves browser cancellation", async () => {
    const fetch = vi.fn(
      async () =>
        new Response(
          JSON.stringify({
            items: [],
            total: 0,
            next_page: null,
            playlist_id: "liked",
          }),
        ),
    );
    vi.stubGlobal("fetch", fetch);
    const client = new RMusicClient("https://core.test/prefix", "session");
    const abort = new AbortController();
    await client.providerPlaylistTracks(
      "provider/a",
      "list/b",
      4,
      abort.signal,
    );
    const [url, options] = fetch.mock.calls[0] as unknown as [
      string,
      RequestInit,
    ];
    expect(url).toBe(
      "https://core.test/prefix/api/v1/providers/provider%2Fa/playlists/list%2Fb/tracks?page=4",
    );
    expect(options.signal).toBe(abort.signal);
    expect(new Headers(options.headers).get("Authorization")).toBe(
      "Bearer session",
    );
  });
  it("sends explicit no-fallback policy and seed without changing the requested source", async () => {
    const fetch = vi.fn(
      async () =>
        new Response(
          JSON.stringify({ items: [], source: "qqmusic", fallback: false }),
        ),
    );
    vi.stubGlobal("fetch", fetch);
    await new RMusicClient("https://core.test").recommendations({
      source: "qqmusic",
      fallback_local: false,
      seed_track_id: "中文 & seed",
      limit: 20,
    });
    const url = new URL((fetch.mock.calls[0] as unknown as [string])[0]);
    expect(url.searchParams.get("source")).toBe("qqmusic");
    expect(url.searchParams.get("fallback_local")).toBe("false");
    expect(url.searchParams.get("seed_track_id")).toBe("中文 & seed");
    expect(url.searchParams.get("limit")).toBe("20");
  });
  it("routes shared SDK helpers through the overridden native transport", async () => {
    class NativeTransport extends RMusicClient {
      paths: string[] = [];
      override async request<T>(path: string): Promise<T> {
        this.paths.push(path);
        return { items: [], next_page: null } as T;
      }
    }
    const native = new NativeTransport("https://core.test");
    await native.providerPlaylists("qqmusic", "collected", 3);
    expect(native.paths).toEqual([
      "/providers/qqmusic/playlists?collection=collected&page=3",
    ]);
  });
});
