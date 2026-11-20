import { describe, expect, it } from "vitest";
import { downloadFilename } from "./fileDownload";
describe("portable audio download filenames", () => {
  it("prefers RFC5987 UTF-8 names for Chinese portable files", () => {
    expect(
      downloadFilename(
        "attachment; filename=audio.flac; filename*=UTF-8''%E5%A4%9C%E8%88%AA.flac",
        "fallback",
      ),
    ).toBe("夜航.flac");
  });
  it("keeps a quoted ordinary name when the extended filename is malformed", () => {
    expect(
      downloadFilename(
        "attachment; filename=\"Night Sail.flac\"; filename*=UTF-8''%broken",
        "fallback",
      ),
    ).toBe("Night Sail.flac");
  });
  it("uses a safe fallback without a disposition header", () => {
    expect(downloadFilename(null, "folder/night.flac")).toBe(
      "folder_night.flac",
    );
  });
});
