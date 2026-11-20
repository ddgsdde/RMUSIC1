import { describe, expect, it } from "vitest";
import { playbackEndBody } from "./playbackEnd";

describe("natural playback completion", () => {
  it("keeps the final browser position including adaptive offset", () => {
    expect(playbackEndBody(new Set(), "track-a", "track-a", "browser-1", 25 + 5))
      .toEqual({ track_id: "track-a", position: 30 });
  });
  it("rejects a stale native end after the zone changed tracks", () => {
    expect(playbackEndBody(new Set(), "track-b", "track-a", "native-1", 30))
      .toBeUndefined();
  });
  it("does not attribute an unidentifiable native end to the current track", () => {
    expect(playbackEndBody(new Set(), "track-b", undefined, "native-1", 30))
      .toBeUndefined();
  });
  it("ignores duplicate ends while allowing a new playback of the same track", () => {
    const handled = new Set<string>();
    expect(playbackEndBody(handled, "track-a", "track-a", "native-1", 30))
      .toEqual({ track_id: "track-a", position: 30 });
    expect(playbackEndBody(handled, "track-a", "track-a", "native-1", 30))
      .toBeUndefined();
    expect(playbackEndBody(handled, "track-a", "track-a", "native-2", 30))
      .toEqual({ track_id: "track-a", position: 30 });
  });
  it("omits unknown or invalid positions rather than inventing a duration", () => {
    for (const position of [undefined, NaN, Infinity, -1]) {
      expect(playbackEndBody(new Set(), "a", "a", "playback", position))
        .toEqual({ track_id: "a" });
    }
  });
});
