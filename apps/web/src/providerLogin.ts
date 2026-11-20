import type {
  ProviderLoginPoll,
  ProviderLoginSession,
  ProviderLoginType,
  RMusicClient,
} from "@r-music/sdk";

export interface ActiveProviderLogin extends ProviderLoginSession {
  provider: string;
}

export function providerLoginFinished(status: string) {
  return [
    "done",
    "success",
    "authenticated",
    "logged_in",
    "expired",
    "error",
    "timeout",
    "refused",
    "cancelled",
  ].includes(status);
}

export function requestProviderLogin(
  api: RMusicClient,
  provider: string,
  type?: ProviderLoginType,
) {
  return api.post<ProviderLoginSession>(
    `/providers/${encodeURIComponent(provider)}/login`,
    type ? { type } : {},
  );
}

export function mergeProviderLoginPoll(
  current: ActiveProviderLogin | null,
  provider: string,
  sessionId: string,
  result: ProviderLoginPoll,
): ActiveProviderLogin | null {
  if (current?.provider !== provider || current.session_id !== sessionId)
    return current;
  return {
    ...current,
    ...result,
    ...(providerLoginFinished(result.status) ? { qr_data_url: null } : {}),
  };
}

export function providerLoginCopy(login: ActiveProviderLogin) {
  return login.login_copy || {
    title: `${login.provider_name||"音源"} · 连接账号`,
    scan: "使用对应音源客户端扫一扫",
    scanned: "已扫码，请在对应音源客户端确认",
    qrLabel: "音源登录二维码",
  };
}

export function providerLoginStatusLabel(login: ActiveProviderLogin) {
  if (["done", "success", "authenticated", "logged_in"].includes(login.status))
    return "登录成功";
  if (["scanned", "confirming"].includes(login.status))
    return providerLoginCopy(login).scanned;
  if (["expired", "timeout"].includes(login.status))
    return "二维码已失效，请重新扫码";
  if (["error", "refused", "cancelled"].includes(login.status))
    return "登录未完成，请重新扫码";
  return providerLoginCopy(login).scan;
}
