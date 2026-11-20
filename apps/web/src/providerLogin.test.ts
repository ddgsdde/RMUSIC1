import { afterEach, describe, expect, it, vi } from "vitest";
import { RMusicClient } from "@r-music/sdk";
import {
  mergeProviderLoginPoll,
  providerLoginCopy,
  providerLoginStatusLabel,
  requestProviderLogin,
  type ActiveProviderLogin,
} from "./providerLogin";

afterEach(() => vi.unstubAllGlobals());
const wechat: ActiveProviderLogin = {
  provider: "qqmusic",
  login_copy:{title:"QQ 音乐 · 微信扫码登录",scan:"使用微信扫一扫",scanned:"已扫码，请在微信中确认",qrLabel:"微信登录二维码"},
  session_id: "wechat-new",
  login_type: "wx",
  status: "waiting",
  qr_data_url: "data:image/png;base64,new",
};

describe("provider QR login", () => {
  it("sends the selected QQ or WeChat login type through the real SDK request", async () => {
    const fetch = vi.fn(
      async (_url: RequestInfo | URL, _options?: RequestInit) =>
        new Response(JSON.stringify({ session_id: "new", status: "waiting" })),
    );
    vi.stubGlobal("fetch", fetch);
    const api = new RMusicClient("https://core.test/prefix", "session");
    await requestProviderLogin(api, "qqmusic", "qq");
    await requestProviderLogin(api, "qqmusic", "wx");
    await requestProviderLogin(api, "qqmusic");
    expect(
      fetch.mock.calls.map(([, options]) =>
        JSON.parse(options!.body as string),
      ),
    ).toEqual([{ type: "qq" }, { type: "wx" }, {}]);
    expect(
      fetch.mock.calls.every(
        ([url]) =>
          url === "https://core.test/prefix/api/v1/providers/qqmusic/login",
      ),
    ).toBe(true);
  });
  it("keeps other plugins on their original empty-body login contract", async () => {
    const fetch = vi.fn(
      async (_url: RequestInfo | URL, _options?: RequestInit) =>
        new Response(
          JSON.stringify({ session_id: "other", status: "waiting" }),
        ),
    );
    vi.stubGlobal("fetch", fetch);
    await requestProviderLogin(
      new RMusicClient("https://core.test"),
      "other/source",
    );
    expect(fetch.mock.calls[0][0]).toBe(
      "https://core.test/api/v1/providers/other%2Fsource/login",
    );
    expect(JSON.parse(fetch.mock.calls[0][1]!.body as string)).toEqual({});
  });
  it("does not let an old QQ poll overwrite the new WeChat session", () => {
    const result = mergeProviderLoginPoll(wechat, "qqmusic", "qq-old", {
      status: "success",
      login_type: "qq",
      qr_data_url: null,
    });
    expect(result).toBe(wechat);
    expect(result?.qr_data_url).toContain("new");
  });
  it("rejects another provider's poll and cannot reopen a closed modal", () => {
    expect(
      mergeProviderLoginPoll(wechat, "other", "wechat-new", {
        status: "error",
      }),
    ).toBe(wechat);
    expect(
      mergeProviderLoginPoll(null, "qqmusic", "wechat-new", {
        status: "success",
      }),
    ).toBeNull();
  });
  it("preserves the selected login type when a confirming poll omits it", () => {
    const result = mergeProviderLoginPoll(wechat, "qqmusic", "wechat-new", {
      status: "confirming",
    })!;
    expect(result.login_type).toBe("wx");
    expect(providerLoginStatusLabel(result)).toBe("已扫码，请在微信中确认");
    expect(providerLoginStatusLabel({ ...result, status: "scanned" })).toBe(
      "已扫码，请在微信中确认",
    );
    expect(providerLoginCopy(result).qrLabel).toContain("微信");
  });
  it("uses phone QQ rather than QQ Music scanner instructions", () => {
    const qq = { ...wechat, login_type: "qq" as const,login_copy:{title:"QQ 音乐 · QQ 扫码登录",scan:"使用手机QQ扫一扫",scanned:"已扫码，请在手机QQ中确认",qrLabel:"QQ 登录二维码"} };
    expect(providerLoginCopy(qq).scan).toBe("使用手机QQ扫一扫");
    expect(providerLoginStatusLabel({ ...qq, status: "confirming" })).toBe(
      "已扫码，请在手机QQ中确认",
    );
    expect(providerLoginCopy({ ...qq, login_type: "mobile" }).scan).toBe(
      "使用手机QQ扫一扫",
    );
    expect(providerLoginCopy(wechat).scan).toBe("使用微信扫一扫");
  });
  it("accepts terminal QR clearing while preserving success and expiry messages", () => {
    const done = mergeProviderLoginPoll(wechat, "qqmusic", "wechat-new", {
      status: "success",
      qr_data_url: null,
      login_type: "wx",
    })!;
    expect(done.qr_data_url).toBeNull();
    expect(providerLoginStatusLabel(done)).toBe("登录成功");
    const omitted = mergeProviderLoginPoll(wechat, "qqmusic", "wechat-new", {
      status: "success",
    })!;
    expect(omitted.qr_data_url).toBeNull();
    const expired = mergeProviderLoginPoll(wechat, "qqmusic", "wechat-new", {
      status: "expired",
      qr_data_url: null,
      error: "登录二维码过期",
    })!;
    expect(providerLoginStatusLabel(expired)).toBe("二维码已失效，请重新扫码");
    expect(expired.error).toBe("登录二维码过期");
  });
});
