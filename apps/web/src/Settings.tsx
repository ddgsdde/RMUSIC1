import {ArtworkMaintenance} from "./ArtworkMaintenance";
import {ProviderAccount} from "./ProviderAccount";
import {SkinStudio} from "./studio/SkinStudio";
import {mobileQuery} from "./MobileShell";
import {CatalogScrapeSettings} from "./CatalogScrapeSettings";
import {MetadataPriority} from "./MetadataPriority";
import {LlmSettings} from "./music/LlmSettings";
import {PluginWorkbench} from "./music/PluginWorkbench";
import { useEffect, useRef, useState } from "react";
import {
  Check,
  FolderPlus,
  KeyRound,
  LoaderCircle,
  Plus,
  RefreshCw,
  Trash2,
} from "lucide-react";
import type {
  ConnectionProfile,
  Endpoint,
  ProviderManifest,
  ProviderLoginPoll,
  ProviderLoginType,
  RMusicClient,
  User,
  Zone,
} from "@r-music/sdk";
import type { Preferences } from "./App";
import { IconButton, Modal } from "./components";
import { browserCache, formatBytes } from "./browserCache";
import {DirectoryPicker} from "./DirectoryPicker";
import {BackupSettings} from "./BackupSettings";
import { ConnectionQuality } from "./ConnectionQuality";
import { CoreConnections } from "./CoreConnections";
import type { CoreAddress } from "@r-music/sdk";
import { isNative } from "./native";
import { NativeSettings } from "./NativeSettings";
import { NativeCacheSettings } from "./NativeCache";
import { AccountSecurity } from "./AccountSecurity";
import { AccountManagement } from "./AccountManagement";
import { ExternalAccounts } from "./ExternalAccounts";
import {CoreExtensions,PluginConfiguration} from "./CoreExtensions";
import { PluginImport } from "./PluginImport";
import {ProviderQuality} from "./ProviderQuality";
import {CoreStatisticsSettings} from "./CoreStatisticsSettings";
import {RemoteAccessSettings} from "./RemoteAccessSettings";
import {BackgroundTasks,CoreResourceSettings} from "./BackgroundTasks";
import { saveApiFile } from "./fileDownload";
import {
  mergeProviderLoginPoll,
  providerLoginCopy,
  providerLoginFinished,
  providerLoginStatusLabel,
  requestProviderLogin,
  type ActiveProviderLogin,
} from "./providerLogin";
interface Root {
  id: string;
  path: string;
}
interface Token {
  id: string;
  name: string;
  scopes: string[];
  created_at: string;
}
interface Cache {
  total?:number;
  limit_bytes: number;
  used_bytes: number;
  directory?: string;
  portable_directory?:string;
  eviction?: string;
  portable_bytes?: number;
  retired_bytes?:number;
  capacity_blocked?:boolean;
  entries: {
    id: string;
    track_id: string;
    size: number;
    complete: boolean;
    pinned: boolean;
    playlist_protected?:boolean;
    canonical?:boolean;
    format: string;
    title?: string;
    artist?: string;
    play_count?: number;
    exports?: {
      status: string;
      path?: string;
      size?: number;
      error?: string;
    } | null;
  }[];
}
export function SettingsView({
  api,
  preferences,
  savePreferences,
  plugins,
  user,
  notify,
  onError,
  refreshLibrary,
  profiles,
  connect,
  openZones,onUserChange,
}: {
  api: RMusicClient;
  preferences: Preferences;
  savePreferences: (patch: Partial<Preferences>) => Promise<void>;
  plugins: ProviderManifest[];
  user: User;
  onUserChange?:(user:User)=>void;
  notify: (message: string) => void;
  onError: (error: unknown) => void;
  refreshLibrary: () => void;
  profiles: ConnectionProfile[];
  connect: (url: string, fingerprint?: string, alternatives?:CoreAddress[]) => void;
  zone?:Zone;endpoint?:Endpoint;refreshZones:()=>Promise<unknown>;
  openZones?:()=>void;
  logout?:()=>Promise<void>;
}) {
  const mobileLayout=window.matchMedia?.(mobileQuery).matches;
  const [pluginConfig,setPluginConfig]=useState<string|null>(null);
  const [scope,setScope]=useState<"core"|"client">(()=>window.matchMedia?.(mobileQuery).matches?"client":"core");
  const [tab, setTab] = useState(()=>window.matchMedia?.(mobileQuery).matches?"appearance":"sources");
  const [deviceAlias,setDeviceAlias]=useState(()=>localStorage.getItem("rmusic.device-name")||localStorage.getItem("rmusic.detected-device-name")||"");
  const [roots, setRoots] = useState<Root[]>([]);
  const [path, setPath] = useState("");
  const [cache, setCache] = useState<Cache | null>(null);
  const [cacheOffset,setCacheOffset]=useState(0);
  const cacheGeneration=useRef(0);
  const cacheReading=useRef<number|null>(null);
  const cacheLifetime=useRef<AbortController|null>(null);
  const cacheContext=useRef({api,scope,tab,cacheOffset});
  const [cacheLimit, setCacheLimit] = useState(100);
  const [cacheDirectory, setCacheDirectory] = useState("");
  const [portableDirectory,setPortableDirectory]=useState("");
  const [clientLimit, setClientLimit] = useState(
    (preferences.client_cache_limit || 5 * 1024 ** 3) / 1024 ** 3,
  );
  const [clientCache, setClientCache] = useState<
    { id: string; size: number; complete: boolean; title?: string }[]
  >([]);
  const [tokens, setTokens] = useState<Token[]>([]);
  const [tokenName, setTokenName] = useState("Home Assistant");
  const [newToken, setNewToken] = useState("");
  const [tokenScopes, setTokenScopes] = useState(["read", "control"]);
  const [login, setLogin] = useState<ActiveProviderLogin | null>(null);
  const loginGeneration = useRef(0);
  const [working, setWorking] = useState("");

  const loadRoots = () =>
    api.get<{ items: Root[] }>("/library/roots").then((r) => setRoots(r.items));
  const loadTokens = () =>
    api
      .get<Token[] | { items: Token[] }>("/auth/tokens")
      .then((r) => setTokens(Array.isArray(r) ? r : r.items));
  const loadCache = async (drafts=true) => {
    const current=()=>{const context=cacheContext.current;return context.api===api&&context.scope===scope&&context.tab===tab&&context.cacheOffset===cacheOffset;};
    if(!current())return;
    const generation=++cacheGeneration.current;cacheReading.current=generation;
    try {
      const result=await api.request<Cache>(`/cache?limit=50&offset=${cacheOffset}`,{signal:cacheLifetime.current?.signal});
      if(!current()||generation!==cacheGeneration.current)return;
      setCache(result);
      if(drafts){setCacheLimit(result.limit_bytes/1024**3);setCacheDirectory(result.directory||"");setPortableDirectory(result.portable_directory||"");}
    } finally {if(cacheReading.current===generation)cacheReading.current=null;}

  };
  useEffect(()=>{
    const controller=new AbortController();cacheLifetime.current=controller;cacheContext.current={api,scope,tab,cacheOffset};
    return()=>{controller.abort();cacheGeneration.current++;cacheReading.current=null;};
  },[api,scope,tab,cacheOffset]);
  useEffect(()=>{const lifetime=cacheLifetime.current;if(scope==="core"&&user.role==="admin"&&tab==="cache")void loadCache(cacheOffset===0).catch(error=>{if(!lifetime?.signal.aborted)onError(error);});},[api,scope,tab,cacheOffset]);
  useEffect(() => {
    if(scope==="core"&&user.role==="admin"){if(tab==="sources")loadRoots().catch(onError);if(tab==="account")loadTokens().catch(onError);}
    if(scope==="client"&&tab==="cache")browserCache
      .list()
      .then(setClientCache)
      .catch(() => {});
  }, [api,scope,tab]);
  useEffect(() => {
    if (tab !== "cache") return;
    let active = true;
    let inFlight = false;
    const poll = async () => {
      if (document.hidden || inFlight || cacheReading.current!==null) return;
      inFlight = true;
      await Promise.all([
        scope === "core" && user.role === "admin" ? loadCache(false) : Promise.resolve(),
        scope === "client" ? browserCache.list().then((result) => {
          if (active) setClientCache(result);
        }) : Promise.resolve(),
      ]).catch(() => {});
      inFlight = false;
    };
    const timer = setInterval(poll, 3000);
    return () => {
      active = false;
      clearInterval(timer);
    };
  }, [api, tab,scope,cacheOffset]);
  useEffect(() => {
    if (!login || providerLoginFinished(login.status)) return;
    let active = true;
    let inFlight = false;
    const generation = loginGeneration.current;
    const poll = () => {
      if (!active || inFlight || generation !== loginGeneration.current) return;
      inFlight = true;
      api
        .get<ProviderLoginPoll>(
          `/providers/${encodeURIComponent(login.provider)}/login/${encodeURIComponent(login.session_id)}`,
        )
        .then((r) => {
          if (active && generation === loginGeneration.current) {
            setLogin((old) =>
              mergeProviderLoginPoll(old, login.provider, login.session_id, r),
            );
            if (
              ["done", "success", "authenticated", "logged_in"].includes(
                r.status,
              )
            )
              notify("音源账号登录成功");
          }
        })
        .catch((error) => {
          if (active && generation === loginGeneration.current)
            setLogin((old) =>
              mergeProviderLoginPoll(old, login.provider, login.session_id, {
                status: "error",
                error: error.message,
              }),
            );
        })
        .finally(() => {
          inFlight = false;
        });
    };
    const timer = setInterval(poll, 1800);
    return () => {
      active = false;
      clearInterval(timer);
    };
  }, [api, login?.provider, login?.session_id, login?.status, notify]);
  async function act(id: string, task: () => Promise<unknown>) {
    setWorking(id);
    try {
      await task();
    } catch (error) {
      onError(error);
    } finally {
      setWorking("");
    }
  }
  async function startLogin(provider: string, type?: ProviderLoginType) {
    const generation = ++loginGeneration.current;
    setLogin(null);
    await act(`login-${provider}`, async () => {
      const result = await requestProviderLogin(api, provider, type);
      if (generation === loginGeneration.current)
        setLogin({
          provider,
          ...result,
          login_type:
            result.login_type ||
            type,
        });
    });
  }
  const sections=(scope==="core"?(user.role==="external"?[["sources","可用音源"],["account","账号与安全"]]:[
          ["sources","曲库与音乐源"],["extensions","插件扩展"],["llm","推荐算法"],["statistics","聆听统计"],["jobs","后台任务"],["artwork","图片维护"],["cache","服务端缓存"],["remote","连接与远程访问"],["backup","备份与恢复"],["account","账号与访问"]
        ]):[
          ["appearance","外观与行为"],["cache","本机缓存"],["connection",isNative?"核心连接与输出":"核心连接"]
        ]).filter(([id])=>id!=="remote"||user.role==="admin");
  return (
    <div className="settings-view studio-settings"><aside className="studio-settings-index">
      <div className="settings-scope" aria-label="设置范围">
        {([['core','核心设置'],['client','客户端设置']] as const).map(([id,label])=><button key={id} aria-pressed={scope===id} onClick={()=>{if(scope!==id){setScope(id);setTab(id==='core'?'sources':'appearance');}}}>{label}<small>{id==='core'?'曲库、音源与共享资源':'外观、连接与本机行为'}</small></button>)}
      </div>
      <p className="scope-description">{scope==='core'?`当前核心 · ${api.baseUrl} · 影响连接此核心的设备`:`当前客户端 · ${deviceAlias||'此设备'} · 仅影响本机`}</p>
      <div className="settings-tabs" role="navigation" aria-label={scope==='core'?'核心设置分类':'客户端设置分类'}>
        {mobileLayout?<label className="studio-category-select">设置分类<select aria-label="设置分类" value={tab} onChange={e=>setTab(e.target.value)}>{sections.map(([id,label])=><option key={id} value={id}>{label}</option>)}</select></label>:sections.map(([id, label]) => (
          <button
            className={tab === id ? "selected" : ""}
            aria-current={tab === id ? "page" : undefined}
            onClick={() => setTab(id)}
            key={id}
          >
            {label}
          </button>
        ))}
      </div>
      </aside><div className="studio-settings-page" key={scope+":"+tab}>
      {scope==="core"&&tab==="artwork"&&user.role==="admin"&&<ArtworkMaintenance api={api}/>}
      {scope==='client'&&tab==='appearance'&&<section className="settings-section client-identity"><label>客户端名称<input value={deviceAlias} maxLength={80} onChange={event=>setDeviceAlias(event.target.value)}/></label><button onClick={()=>{const value=deviceAlias.trim();if(!value){notify('请输入客户端名称');return;}localStorage.setItem('rmusic.device-name',value);window.dispatchEvent(new Event('rmusic-device-name'));notify('客户端名称已保存');}}>保存名称</button></section>}
      {scope==="core"&&tab==="sources"&&user.role==="admin"&&<><CatalogScrapeSettings api={api}/><MetadataPriority api={api}/></>}
      {scope==="core"&&tab==="backup"&&<BackupSettings api={api} admin={user.role==="admin"} onError={onError}/>}
      {scope==="core"&&tab==="extensions"&&<><CoreExtensions key={api.baseUrl} api={api}/>{user.role==="admin"&&<PluginWorkbench api={api} plugins={plugins}/>}</>}
      {scope==="core"&&tab==="llm"&&<LlmSettings api={api} admin={user.role==="admin"}/>}
      {pluginConfig&&<PluginConfiguration key={pluginConfig} api={api} id={pluginConfig} close={()=>setPluginConfig(null)}/>}
      {tab === "sources" && (
        <>
          {user.role === "admin" && <section className="settings-section">
            <div className="section-heading">
              <div>
                <h2>本地音乐目录</h2>
                <p>目录路径相对于核心；Docker 内请使用已挂载的路径。</p>
              </div>
              <button
                disabled={working === "scan" || !roots.length}
                onClick={() =>
                  act("scan", async () => {
                    await api.post("/library/scan");
                    notify("已启动增量扫描");
                    refreshLibrary();
                  })
                }
              >
                <RefreshCw
                  size={16}
                  className={working === "scan" ? "spin" : ""}
                />
                扫描曲库
              </button>
            </div>
            <form
              className="inline-form"
              onSubmit={(e) => {
                e.preventDefault();
                act("root", async () => {
                  await api.post("/library/roots", { path });
                  setPath("");
                  await loadRoots();
                });
              }}
            >
              <FolderPlus size={20} />
              <DirectoryPicker api={api} value={path} onChange={setPath} label="选择音乐目录"/>
              <button className="primary" disabled={working === "root"||!path}>
                添加目录
              </button>
            </form>
            <div className="settings-list">
              {roots.map((root) => (
                <div key={root.id}>
                  <span>{root.path}</span>
                  <IconButton
                    label={`移除目录 ${root.path}`}
                    onClick={() =>
                      act(root.id, async () => {
                        await api.delete(`/library/roots/${root.id}`);
                        await loadRoots();
                      })
                    }
                  >
                    <Trash2 size={17} />
                  </IconButton>
                </div>
              ))}
            </div>
          </section>}
          <section className="settings-section">
            <div className="section-heading">
              <h2>音源插件</h2>
              {user.role === "admin" && (
                <PluginImport api={api} notify={notify} onError={onError} />
              )}
            </div>
            {plugins.map((plugin) => (
              <div className="plugin-row" key={plugin.id}>
                <div className="plugin-logo">
                  {plugin.id === "qq" ? "Q" : plugin.name[0]}
                </div>
                <div className="plugin-info">
                  <strong>{plugin.name}</strong>
                  <span>
                    {plugin.enabled?'已启用':'已停用'} · v{plugin.version}
                    {plugin.error ? ` · ${plugin.error}` : ""}
                  </span>
                  <details className="plugin-capabilities"><summary>插件信息</summary><small>{plugin.status} · {plugin.capabilities.join(" · ")}</small></details>
                  {user.role==="admin"&&<ProviderAccount api={api} plugin={plugin} notify={notify}/>}
                  {user.role==="admin"&&<ProviderQuality api={api} provider={plugin} onError={onError}/>}
                  {(plugin.login_types?.length||0)>1 && (
                    <small>请选择对应音源账号的登录方式。</small>
                  )}
                </div>
                {user.role === "admin" && <div className="plugin-actions">
                  {plugin.configuration_v1&&<button onClick={()=>setPluginConfig(plugin.id)}>插件配置</button>}
                  <button
                    onClick={() =>
                      act(`rollback-${plugin.id}`, () =>
                        api
                          .post(`/plugins/${plugin.id}/rollback`)
                          .then(() => notify("插件已回滚到上一版本")),
                      )
                    }
                  >
                    回滚
                  </button>
                  <button
                    onClick={() =>
                      act(`enable-${plugin.id}`, () =>
                        api
                          .patch(`/plugins/${plugin.id}`, {
                            enabled: !plugin.enabled,
                          })
                          .then(() => notify("插件状态已更新")),
                      )
                    }
                  >
                    {plugin.enabled ? "停用" : "启用"}
                  </button>
                  {plugin.enabled &&
                    plugin.capabilities.some((c) =>
                      ["login", "qr_login", "auth"].includes(c),
                    ) &&
                    ((plugin.login_types?.length?plugin.login_types:[undefined]).map((type,index)=><button key={type||"default"} className={index===0?"primary":undefined} disabled={working===`login-${plugin.id}`} onClick={()=>startLogin(plugin.id,type)}>{type?(plugin.login_labels?.[type]||type):"扫码登录"}</button>))}
                  {plugin.enabled &&
                    plugin.capabilities.some((c) =>
                      ["sync", "library", "playlists"].includes(c),
                    ) && (
                      <button
                        onClick={() =>
                          act(`sync-${plugin.id}`, () =>
                            api
                              .post(`/providers/${plugin.id}/sync`)
                              .then(() => notify("音源同步已开始")),
                          )
                        }
                      >
                        同步曲库
                      </button>
                    )}
                </div>}
              </div>
            ))}
          </section>

        </>
      )}
      {scope==="core"&&tab==="statistics"&&user.role==="admin"&&<CoreStatisticsSettings api={api} onError={onError} notify={notify}/>}
      {scope==="core"&&tab==="remote"&&user.role==="admin"&&<RemoteAccessSettings api={api} user={user} notify={notify} onError={onError}/>}
      {scope==="core"&&tab==="jobs"&&<>{user.role==="admin"&&<CoreResourceSettings api={api} onError={onError} notify={notify}/>}<BackgroundTasks api={api} onError={onError}/></>}
      {tab === "appearance" && (
        <>
          <SkinStudio preferences={preferences} save={savePreferences} notify={notify}/>
          <section className="settings-section">
            <label className="setting-row">
              <span>
                <strong>显示模式</strong>
                <small>跟随系统会自动响应深色模式切换</small>
              </span>
              <select
                aria-label="显示模式"
                value={preferences.mode}
                onChange={(e) =>
                  savePreferences({
                    mode: e.target.value as Preferences["mode"],
                  })
                }
              >
                <option value="system">跟随系统</option>
                <option value="dark">深色</option>
                <option value="light">浅色</option>
              </select>
            </label>
            <label className="setting-row">
              <span><strong>{mobileLayout?"背景动态":"封面旋转"}</strong><small>{mobileLayout?"封面氛围随播放缓慢流动，暂停时停留":"播放时缓慢旋转，暂停时保留角度"}</small></span>
              <input className="toggle" type="checkbox" checked={preferences.player_rotate!==false} onChange={e=>savePreferences({player_rotate:e.target.checked})}/>
            </label>
            <label className="setting-row">
              <span><strong>播放页封面氛围</strong><small>关闭后保留主题纯色背景</small></span>
              <input className="toggle" type="checkbox" checked={preferences.player_atmosphere!==false} onChange={e=>savePreferences({player_atmosphere:e.target.checked})}/>
            </label>
            {isNative&&/Android/i.test(navigator.userAgent)&&<>
              <label className="setting-row">
                <span><strong>底部播放栏标题</strong><small>优先显示当前歌词，缺译文时显示原文</small></span>
                <select aria-label="底部播放栏标题" value={preferences.mini_player_title||"translation"}
                  onChange={e=>savePreferences({mini_player_title:e.target.value as Preferences["mini_player_title"]})}>
                  <option value="translation">译文优先</option><option value="original">歌词原文</option><option value="song">歌名</option>
                </select>
              </label>
              {preferences.mini_player_title!=="song"&&<label className="setting-row">
                <span><strong>底部歌词逐字跟随</strong><small>有逐字时间轴时高亮，长句随播放位置滚动</small></span>
                <input className="toggle" type="checkbox" checked={preferences.mini_player_word_follow!==false}
                  onChange={e=>savePreferences({mini_player_word_follow:e.target.checked})}/>
              </label>}
            </>}
            <label className="setting-row">
              <span>
                <strong>低性能模式</strong>
                <small>{mobileLayout?"关闭背景动态和模糊，降低歌词刷新率":"关闭封面旋转和背景模糊，降低可视化刷新率"}</small>
              </span>
              <input
                className="toggle"
                type="checkbox"
                checked={preferences.low_performance}
                onChange={(e) =>
                  savePreferences({ low_performance: e.target.checked })
                }
              />
            </label>
            <p className="fine-print">动画会自动遵循系统“减少动态效果”设置。</p>
          </section>
        </>
      )}
      {tab === "cache" && (
        <>
          {scope==="client"&&isNative && (
            <NativeCacheSettings api={api} notify={notify} onError={onError} />
          )}
          {scope==="core"&&user.role === "admin" && <section className="settings-section">
            <CoreResourceSettings api={api} onError={onError} notify={notify}/>
            <h2>核心共享缓存</h2>
            <div className="storage-summary">
              <strong>{formatBytes(cache?.used_bytes || 0)}</strong>
              <span>
                / {formatBytes(cache?.limit_bytes || 100 * 1024 ** 3)}
              </span>
            </div>
            <label className="inline-form">
              <span>容量上限 (GiB)</span>
              <input
                type="number"
                min={0.1}
                step={0.5}
                max={100000}
                value={cacheLimit}
                onChange={(e) => setCacheLimit(Number(e.target.value))}
              />
              <button
                onClick={() =>
                  act("cache", async () => {
                    await api.patch("/cache", {
                      limit_bytes: Math.round(cacheLimit * 1024 ** 3),
                      ...(portableDirectory.trim()?{portable_directory:portableDirectory.trim()}:{}),
                      ...(cache?.directory && cacheDirectory.trim()
                        ? { directory: cacheDirectory.trim() }
                        : {}),
                    });
                    await loadCache();
                    notify("核心缓存容量已更新");
                  })
                }
              >
                保存
              </button>
            </label>
            <label className="field cache-directory-field">
              <span>核心缓存独立目录</span>
              <DirectoryPicker api={api} value={cacheDirectory} disabled={!cache?.directory} onChange={setCacheDirectory} label="选择核心缓存目录"/>
            </label>
            <label className="field cache-directory-field"><span>完整歌曲输出文件夹</span><DirectoryPicker api={api} value={portableDirectory} onChange={setPortableDirectory} label="选择完整歌曲输出目录"/></label>
            <p className="fine-print">
              默认 100GiB，不预分配。
              {cache?.eviction?.startsWith("least_played")
                ? "按播放频率优先保留，低频歌曲先淘汰。"
                : "淘汰策略由核心配置决定。"}
              临时播放缓冲保存在内存；磁盘长期只保留完整音乐文件。完整歌曲整理封面、歌词和标签后加入本地曲库，后续直接读取该文件。保存歌单中的歌曲、固定保留与正在使用的文件不参与自动淘汰。
            </p>
            <p className="fine-print">
              新缓存写入此目录；已有文件继续可读并按规则淘汰。Docker
              需要先挂载目标目录并授予核心写入权限。完整文件整理后用于播放与导出，保留一份音频。大文件生成时可能有短暂的输出临时文件，中断后丢弃未提交部分，补齐任务保留。整理资料及历史副本占用{" "}
              {formatBytes(cache?.portable_bytes || 0)}。
            </p>
            {cache?.capacity_blocked&&<p className="notice">缓存容量已满，新增缓存暂停；受保护的歌曲保留，播放继续。</p>}
            {!!cache?.retired_bytes&&<p className="fine-print">待释放文件仍计入占用：{formatBytes(cache.retired_bytes)}。</p>}
            <div className="pagination-controls" aria-label="核心缓存分页">
              <button disabled={cacheOffset===0} onClick={()=>setCacheOffset(Math.max(0,cacheOffset-50))}>上一页</button>
              <span>第 {Math.floor(cacheOffset/50)+1} / {Math.max(1,Math.ceil((cache?.total??0)/50))} 页 · {cache?.total??0} 首</span>
              <button disabled={cacheOffset+50>=(cache?.total??0)} onClick={()=>setCacheOffset(cacheOffset+50)}>下一页</button>
            </div>
            <div className="cache-list">
              {cache?.entries.map((entry) => (
                <div key={entry.id}>
                  <span>{entry.title || entry.track_id}</span>
                  <small>
                    {entry.complete ? "完整" : "部分"} · {entry.format} ·{" "}
                    {formatBytes(entry.size)}
                    {entry.playlist_protected?" · 歌单保留":""}{entry.canonical?" · 已整理":""}
                    {entry.play_count != null
                      ? ` · 播放 ${entry.play_count} 次`
                      : ""}
                    {entry.exports
                      ? ` · ${{pending:"等待整理",writing:"正在整理",ready:"可导出",failed:"整理失败"}[entry.exports.status]||entry.exports.status}${entry.exports.error ? `：${entry.exports.error}` : ""}`
                      : ""}
                  </small>
                  {entry.exports?.status!=="ready"&&<button
                    disabled={
                      !entry.complete ||
                      ["pending", "writing"].includes(
                        entry.exports?.status || "",
                      )
                    }
                    onClick={() =>
                      act(`export-${entry.id}`, async () => {
                        await api.post(`/cache/${entry.id}/export`);
                        await loadCache();
                        notify("已加入缓存资料整理队列");
                      })
                    }
                  >
                    完善缓存资料
                  </button>}
                  <button
                    disabled={
                      !["ready", "complete", "completed"].includes(
                        entry.exports?.status || "",
                      )
                    }
                    title="封面、歌词与元数据已写入或随文件保留"
                    onClick={() =>
                      saveApiFile(
                        api,
                        `/cache/${entry.id}/file`,
                        entry.title || entry.track_id,
                      )
                        .then(() => notify("音乐文件已导出"))
                        .catch(onError)
                    }
                  >
                    导出音乐文件
                  </button>
                  <IconButton
                    label="删除缓存"
                    disabled={entry.pinned}
                    onClick={() =>
                      api
                        .delete(`/cache/${entry.id}`)
                        .then(()=>loadCache())
                        .catch(onError)
                    }
                  >
                    <Trash2 size={16} />
                  </IconButton>
                </div>
              ))}
            </div>
          </section>}
          {scope==="client"&&<section className="settings-section">
            <h2>{isNative ? "WebView 临时缓存" : "此设备缓存"}</h2>
            <label className="inline-form">
              <span>容量上限 (GiB)</span>
              <input
                type="number"
                min={0.1}
                max={1000}
                step={0.5}
                value={clientLimit}
                onChange={(e) => setClientLimit(Number(e.target.value))}
              />
              <button
                onClick={() =>
                  act("client-cache", async () => {
                    const bytes = Math.round(clientLimit * 1024 ** 3);
                    await browserCache.setLimit(bytes);
                    await savePreferences({ client_cache_limit: bytes });
                    setClientCache(await browserCache.list());
                    notify("此设备缓存容量已更新");
                  })
                }
              >
                保存
              </button>
            </label>
            <p className="fine-print">
              浏览器缓存受系统存储额度限制。完整缓存可从“下载”页面在此设备播放。浏览器无法可靠识别非计费
              Wi-Fi，不自动升级音质。
            </p>
            <div className="cache-list">
              {clientCache.map((entry) => (
                <div key={entry.id}>
                  <span>{entry.title || entry.id}</span>
                  <small>
                    {entry.complete ? "完整 · 可离线播放" : "部分"} ·{" "}
                    {formatBytes(entry.size)}
                  </small>
                  <IconButton
                    label="删除此设备缓存"
                    onClick={() =>
                      browserCache
                        .remove(entry.id)
                        .then(() => browserCache.list())
                        .then(setClientCache)
                        .catch(onError)
                    }
                  >
                    <Trash2 size={16} />
                  </IconButton>
                </div>
              ))}
            </div>
          </section>}
        </>
      )}
      {tab === "account" && (
        <>
          <AccountSecurity api={api} user={user} onUserChange={onUserChange} notify={notify} onError={onError}/>
          {user.role==="admin"&&<ExternalAccounts key={user.id} api={api} plugins={plugins} notify={notify} onError={onError}/>}
          <AccountManagement
            api={api}
            user={user}
            notify={notify}
            onError={onError}
          />
          {user.role === "admin" && <section className="settings-section">
            <h2>外部控制凭证</h2>
            <p className="muted">
              为 Home Assistant 或其他控制器创建独立凭证，随时撤销。
            </p>
            <form
              onSubmit={(e) => {
                e.preventDefault();
                act("token", async () => {
                  const result = await api.post<{ token: string }>(
                    "/auth/tokens",
                    { name: tokenName, scopes: tokenScopes },
                  );
                  setNewToken(result.token);
                  await loadTokens();
                });
              }}
            >
              <label>
                凭证名称
                <input
                  required
                  value={tokenName}
                  onChange={(e) => setTokenName(e.target.value)}
                />
              </label>
              <div className="scope-options">
                {[
                  ["read", "读取状态与曲库"],
                  ["control", "控制播放"],
                  ["library", "管理曲库"],
                  ["admin", "管理核心"],
                ].map(([id, label]) => (
                  <label key={id}>
                    <input
                      type="checkbox"
                      checked={tokenScopes.includes(id)}
                      onChange={(e) =>
                        setTokenScopes((old) =>
                          e.target.checked
                            ? [...old, id]
                            : old.filter((s) => s !== id),
                        )
                      }
                    />
                    {label}
                  </label>
                ))}
              </div>
              <button disabled={!tokenScopes.length || working === "token"}>
                <Plus size={16} />
                创建凭证
              </button>
            </form>
            <div className="settings-list">
              {tokens.map((item) => (
                <div key={item.id}>
                  <div>
                    <strong>{item.name}</strong>
                    <small>{item.scopes?.join(" · ")}</small>
                  </div>
                  <IconButton
                    label={`撤销 ${item.name}`}
                    onClick={() =>
                      api
                        .delete(`/auth/tokens/${item.id}`)
                        .then(loadTokens)
                        .catch(onError)
                    }
                  >
                    <Trash2 size={17} />
                  </IconButton>
                </div>
              ))}
            </div>
            <button
              className="text-button"
              onClick={() =>
                act("openapi", async () => {
                  const data = await api.get("/openapi.json");
                  const url = URL.createObjectURL(
                    new Blob([JSON.stringify(data, null, 2)], {
                      type: "application/json",
                    }),
                  );
                  const anchor = document.createElement("a");
                  anchor.href = url;
                  anchor.download = "r-music-openapi.json";
                  anchor.click();
                  setTimeout(() => URL.revokeObjectURL(url), 1000);
                })
              }
            >
              下载 OpenAPI 文档
            </button>
          </section>}
        </>
      )}
      {tab === "connection" && (
        <>
          {isNative&&(/Android/i.test(navigator.userAgent)?<NativeSettings api={api} onError={onError} notify={notify}/>:<section className="settings-section"><h2>本机音频输出</h2><p className="muted">在「播放设备」展开当前电脑，为每个 WASAPI／ASIO 输出分别设置。</p>{openZones&&<button onClick={openZones}>打开播放设备</button>}</section>)}
          <section className="settings-section">
            <h2>核心连接</h2>
            <ConnectionQuality api={api}/>
            {isNative && <CoreConnections api={api} profiles={profiles} onConnect={connect}/>}
          </section>
        </>
      )}
      {newToken && (
        <Modal title="API 凭证已创建" onClose={() => setNewToken("")}>
          <p>凭证只显示一次。将它填入 Home Assistant 的 R-Music 集成。</p>
          <code className="secret">{newToken}</code>
          <button
            onClick={() =>
              navigator.clipboard
                .writeText(newToken)
                .then(() => notify("API 凭证已复制"))
                .catch(onError)
            }
          >
            <KeyRound size={17} />
            复制凭证
          </button>
        </Modal>
      )}
      {login && (
        <Modal
          title={providerLoginCopy(login).title}
          onClose={() => {
            loginGeneration.current++;
            setLogin(null);
          }}
        >
          <div className="qr-login">
            {login.qr_data_url ? (
              <img
                src={login.qr_data_url}
                alt={providerLoginCopy(login).qrLabel}
                title={providerLoginCopy(login).qrLabel}
              />
            ) : ["done", "success", "authenticated", "logged_in"].includes(
                login.status,
              ) ? (
              <Check size={48} />
            ) : [
                "expired",
                "error",
                "timeout",
                "refused",
                "cancelled",
              ].includes(login.status) ? (
              <RefreshCw size={42} />
            ) : (
              <LoaderCircle className="spin" />
            )}
            <strong>{providerLoginStatusLabel(login)}</strong>
            <span>{login.error || login.status}</span>
          </div>
          <p className="fine-print">

            音质由账号实际权限与音源可用版本决定。登录失效后需重新扫码。
          </p>
        </Modal>
      )}
    </div>
    </div>
  );
}
