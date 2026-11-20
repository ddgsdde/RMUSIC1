import {useEffect, useRef, useState, type FormEvent} from "react";
import type {AccountImportJob, ExternalAccount, ExternalSourcePolicy, ProviderManifest, RMusicClient} from "@r-music/sdk";
import {Modal} from "./components";
import {createPortal} from "react-dom";
import {isNative} from "./native";
import {saveApiFile} from "./fileDownload";
const deny: ExternalSourcePolicy = {providers: [], local_cache: false, local_library: false, bandwidth_bytes_per_second: 0};
type Mode = "create" | "edit" | "delete" | "export" | "import";
export function ExternalAccounts({api, plugins, notify, onError}: {api: RMusicClient; plugins: ProviderManifest[]; notify: (message: string) => void; onError: (error: unknown) => void}) {
  const [accounts, setAccounts] = useState<ExternalAccount[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [mode, setMode] = useState<Mode | null>(null);
  const [selected, setSelected] = useState<ExternalAccount>();
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const [current, setCurrent] = useState("");
  const [totp, setTotp] = useState("");
  const [confirmation, setConfirmation] = useState("");
  const [policy, setPolicy] = useState<ExternalSourcePolicy>(deny);
  const [enabled, setEnabled] = useState(true);
  const [reset, setReset] = useState(false);
  const [file, setFile] = useState<File>();
  const [job, setJob] = useState<AccountImportJob>();
  const [jobs, setJobs] = useState<AccountImportJob[]>([]);
  const [merge, setMerge] = useState("");
  const [busy, setBusy] = useState(false);
  const working = useRef(false);
  const lifetime = useRef(new AbortController());
  const reload = async (after = "") => {
    const result = await api.request<{items: ExternalAccount[]; next_cursor: string | null}>(`/auth/external-accounts${after ? `?cursor=${encodeURIComponent(after)}` : ""}`, {signal: lifetime.current.signal});
    setAccounts(old => after ? [...old, ...result.items] : result.items); setCursor(result.next_cursor);
  };
  const reloadJobs = async () => {
    const result = await api.request<{items: AccountImportJob[]}>("/auth/account-imports", {signal: lifetime.current.signal}); setJobs(result.items);
  };
  useEffect(() => {
    const controller = new AbortController(); lifetime.current = controller;
    setMode(null); setCurrent(""); setTotp(""); setPassword(""); setJob(undefined);
    void Promise.all([reload(), reloadJobs()]).catch(error => {if (!controller.signal.aborted) onError(error);});
    return () => controller.abort();
  }, [api]);
  useEffect(() => {
    if (!job || !["applying", "uploading"].includes(job.state)) return;
    const controller = new AbortController(); let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const value = await api.request<AccountImportJob>(`/auth/account-imports/${job.id}`, {signal: controller.signal});
        if (controller.signal.aborted) return;
        setJob(value);
        if (value.state === "completed") {notify("账号数据导入完成"); void reload(); void reloadJobs();}
        else if (["applying", "uploading"].includes(value.state)) timer = setTimeout(poll, 750);
      } catch (error) {if (!controller.signal.aborted) onError(error);}
    };
    timer = setTimeout(poll, 750); return () => {controller.abort(); clearTimeout(timer);};
  }, [api, job?.id, job?.state]);
  const open = (next: Mode, account?: ExternalAccount) => {
    setSelected(account); setMode(next); setName(account?.username || ""); setPassword(""); setCurrent(""); setTotp(""); setConfirmation("");
    setPolicy(account ? {...account.policy, providers: [...account.policy.providers]} : {...deny, providers: []}); setEnabled(account?.enabled ?? true); setReset(false); setFile(undefined); setJob(undefined); setMerge("");
  };
  const close = () => {if (working.current) return; setMode(null); setPassword(""); setCurrent(""); setTotp(""); setFile(undefined); void reloadJobs().catch(onError);};
  const submit = async (event: FormEvent) => {
    event.preventDefault(); if (working.current || !mode) return; working.current = true; setBusy(true);
    const signal = lifetime.current.signal; const credentials = {current_password: current, ...(totp ? {totp} : {})};
    try {
      if (mode === "create") {await api.request("/auth/external-accounts", {method: "POST", signal, body: JSON.stringify({...credentials, username: name.trim(), password, policy})}); notify("外部账号已创建");}
      if (mode === "edit" && selected) {await api.request(`/auth/external-accounts/${selected.id}`, {method: "PATCH", signal, body: JSON.stringify({...credentials, username: name.trim(), ...(password ? {new_password: password} : {}), policy, enabled, reset_totp: reset})}); notify("账号已更新，该账号的设备需重新登录");}
      if (mode === "delete" && selected) {await api.request(`/auth/external-accounts/${selected.id}`, {method: "DELETE", signal, body: JSON.stringify({...credentials, confirmation})}); notify("账号及其个人数据已删除");}
      if (mode === "export" && selected) {
        const ticket = await api.request<{api_path: string}>(`/auth/external-accounts/${selected.id}/export-ticket`, {method: "POST", signal, body: JSON.stringify(credentials)});
        const path = ticket.api_path.replace(/^\/?api\/v1/, "");
        if (isNative) await saveApiFile(api, path, `rmusic-${selected.username}.jsonl`);
        else {const anchor = document.createElement("a"); anchor.href = api.url(`/api/v1${path}`); anchor.download = `rmusic-${selected.username}.jsonl`; anchor.click();}
        notify("账号档案导出已开始");
      }
      if (mode === "import") {
        if (!job || ["created", "invalid"].includes(job.state)) {
          if (!file || file.size > 2 * 1024 ** 3) throw new Error("请选择不超过 2 GiB 的 R-Music 账号档案");
          const created = job
            ? await api.request<AccountImportJob>(`/auth/account-imports/${job.id}/authorize-upload`, {method: "POST", signal, body: JSON.stringify(credentials)})
            : await api.request<AccountImportJob>("/auth/account-imports", {method: "POST", signal, body: JSON.stringify(credentials)});
          setJob(created);
          const uploaded = await api.uploadAccountArchive(created.id, file, signal); setJob(uploaded); setName(uploaded.manifest?.username || ""); setFile(undefined);
        } else {
          const result = await api.request<AccountImportJob>(`/auth/account-imports/${job.id}/apply`, {method: "POST", signal, body: JSON.stringify({...credentials, mode: job.target_user || merge ? "merge" : "new", ...(job.target_user || merge ? {target_user: job.target_user || merge} : {username: name.trim(), password, policy})})}); setJob(result);
        }
        void reloadJobs(); return;
      }
      setMode(null); await reload();
    } catch (error) {if (!signal.aborted) onError(error);}
    finally {setCurrent(""); setTotp(""); setPassword(""); working.current = false; setBusy(false);}
  };
  const permissionForm = <fieldset className="external-policy"><legend>可用音源</legend>
    {Array.from(new Set([...plugins.map(plugin => plugin.id), ...policy.providers])).map(id => ({id, name: plugins.find(plugin => plugin.id === id)?.name || id})).map(plugin => <label key={plugin.id}><input type="checkbox" checked={policy.providers.includes(plugin.id)} onChange={event => setPolicy({...policy, providers: event.target.checked ? [...policy.providers, plugin.id] : policy.providers.filter(id => id !== plugin.id)})}/>{plugin.name || plugin.id}</label>)}
    <label><input type="checkbox" checked={policy.local_cache} onChange={event => setPolicy({...policy, local_cache: event.target.checked})}/>在线平台已缓存的音乐</label>
    <label><input type="checkbox" checked={policy.local_library} onChange={event => setPolicy({...policy, local_library: event.target.checked})}/>自有本地曲库</label>
    <label>账号总带宽（Mbps，0 为不限）<input type="number" min="0" max="8589" step="any" value={policy.bandwidth_bytes_per_second / 125000} onChange={event => setPolicy({...policy, bandwidth_bytes_per_second: Math.round(Number(event.target.value) * 125000)})}/></label>
    <p className="muted">多台设备共用带宽额度。同账号设备可以遥控和流转播放，家庭音区同步独立保留。</p>
  </fieldset>;
  return <section className="settings-section external-accounts">
    <div className="section-heading"><h2>外部账号</h2><div className="button-row"><button onClick={() => open("import")}>导入账号数据</button><button onClick={() => open("create")}>创建外部账号</button></div></div>
    <p className="muted">每个账号独立保存歌单、聆听记录、统计和推荐。音源默认不授权。</p>
    {accounts.length === 0 && <p className="muted">尚无外部账号</p>}
    {accounts.map(account => <div className="external-account-row" key={account.id}><div><strong>{account.username}</strong><small>{account.enabled ? "已启用" : "已停用"} · {account.totp_enabled ? "两步验证已启用" : "未设置两步验证"} · {account.policy.bandwidth_bytes_per_second ? `${account.policy.bandwidth_bytes_per_second / 125000} Mbps` : "不限速"}</small></div><div className="button-row"><button onClick={() => open("edit", account)}>管理</button><button onClick={() => open("export", account)}>导出数据</button><button onClick={() => open("delete", account)}>删除</button></div></div>)}
    {cursor && <button onClick={() => void reload(cursor).catch(onError)}>加载更多账号</button>}
    {jobs.length > 0 && <div className="external-import-jobs"><h3>账号导入任务</h3>{jobs.map(item => <div key={item.id} className="external-account-row"><span>{item.manifest?.username || "待上传档案"} · {item.state} · {item.applied || 0}/{item.records || 0}</span><div className="button-row"><button onClick={() => {open("import"); void api.get<AccountImportJob>(`/auth/account-imports/${item.id}`).then(value => {setJob(value); setName(value.manifest?.username || ""); setMerge(value.target_user || "");}).catch(onError);}}>查看／继续</button>{!["applying", "interrupted"].includes(item.state) && <button onClick={() => void api.request(`/auth/account-imports/${item.id}`, {method: "DELETE"}).then(reloadJobs).catch(onError)}>移除任务</button>}</div></div>)}</div>}
    {mode && createPortal(<Modal title={{create: "创建外部账号", edit: "管理外部账号", delete: "删除外部账号", export: "导出账号数据", import: "导入账号数据"}[mode]} onClose={close}><form onSubmit={event => void submit(event)} className="external-account-form">
      {mode === "import" && (!job || ["created", "invalid"].includes(job.state)) && <label>账号档案<input type="file" accept=".jsonl,.ndjson,application/x-ndjson" required onChange={event => setFile(event.target.files?.[0])}/></label>}
      {mode === "import" && job && <><p>{job.manifest?.username} · {job.records || 0} 条资料 · {job.applied || 0} 条已导入</p>{job.error && <p role="alert">{job.error}</p>}<p className="muted">{Object.entries(job.counts || {}).map(([kind, count]) => `${kind}: ${count}`).join(" · ")}</p>{["uploaded", "interrupted"].includes(job.state) && <label>导入方式<select value={merge} disabled={!!job.target_user} onChange={event => setMerge(event.target.value)}><option value="">按新用户导入</option>{accounts.map(account => <option key={account.id} value={account.id}>合并到 {account.username}</option>)}</select></label>}{!!job.merge_candidates?.length && <p>发现相同来源账号：{job.merge_candidates.map(item => item.username).join("、")}。请选择合并，或为新账号设置名称。</p>}</>}
      {(mode === "create" || mode === "edit" || mode === "import" && !!job && !merge && !job.target_user && ["uploaded", "interrupted"].includes(job.state)) && <><label>用户名<input value={name} maxLength={80} autoComplete="off" required onChange={event => setName(event.target.value)}/></label><label>{mode === "edit" ? "新密码（留空保留）" : "初始密码"}<input type="password" value={password} autoComplete="new-password" required={mode !== "edit"} minLength={10} maxLength={1024} onChange={event => setPassword(event.target.value)}/></label>{permissionForm}</>}
      {mode === "edit" && <><label><input type="checkbox" checked={enabled} onChange={event => setEnabled(event.target.checked)}/>启用账号</label><label><input type="checkbox" checked={reset} onChange={event => setReset(event.target.checked)}/>重置两步验证和恢复码</label></>}
      {mode === "delete" && <><p>删除 {selected?.username} 及其歌单、历史、统计和设备记录。</p><label>输入完整用户名确认<input required value={confirmation} onChange={event => setConfirmation(event.target.value)}/></label></>}
      {mode === "export" && <p>导出 {selected?.username} 的歌单、偏好、历史和统计，不包含密码、两步验证密钥或平台登录凭据。</p>}
      {!(mode === "import" && job && ["applying", "completed", "uploading"].includes(job.state)) && <><label>当前管理员密码<input type="password" required value={current} autoComplete="current-password" onChange={event => setCurrent(event.target.value)}/></label><label>两步验证码或恢复码（已启用时填写）<input value={totp} autoComplete="one-time-code" onChange={event => setTotp(event.target.value)}/></label><button className="primary" disabled={busy || mode === "delete" && confirmation !== selected?.username}>{busy ? "正在处理…" : mode === "import" ? job ? "确认导入／继续" : "上传并预览" : "确认"}</button></>}
    </form></Modal>, document.body)}
  </section>;
}
