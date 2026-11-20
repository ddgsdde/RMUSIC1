import {PersonalRecommendations,DailyPlaylist,type Picks} from "./PersonalRecommendations";
import {DailyPlaylistPage} from "./LocalPlaylistView";
import {listen} from "@tauri-apps/api/event";
import {remoteOutputPlaying} from "./remotePlayback";
import {usePresentationVisible,presentationVisible} from "./presentationVisibility";
import {applySkin,activeSkin,type Skin} from "./studio/skins";
import {ProfilePage,StudioShell,routeTitle} from "./studio/StudioShell";
import {ListeningDeck} from "./studio/ListeningDeck";
import {CoreConnections} from "./CoreConnections";
import {ConnectionQuality} from "./ConnectionQuality";
import {CoreWebClient} from "./CoreWebClient";
import {presentedZones} from "./deviceGroupModel";
import {ExternalAccountContext} from "./AccountAccess";
import {PlaybackQueueTracks} from "./PlaybackQueueTracks";
import {PageMemory} from "./PageMemory";
import {loadLyrics} from "./lyricResource";
import {AndroidQuality} from "./AndroidQuality";
import {mobileUI,type MobileUI} from "./mobileAppearance";
import {transitionPlayer,dismissQueue,useAndroidPlayerSheet} from "./mobileMotion";
import {useMobileLayout,MobileSearchBrowse} from "./MobileShell";
import {TrackContextMenuProvider} from "./TrackContextMenu";
import {SearchPage} from "./Search";
import {CatalogNavigation} from "./CatalogLinks";
import {outputLabel,useOutputLabels,zoneDisplayName} from "./outputLabels";
import { clientId } from "./clientId";
import {useGroupTransport} from "./useGroupTransport";
import { Suspense, lazy, type ComponentProps, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  WifiOff,
  Download,
  ChevronRight,
  Plus,
  PanelRightClose,
  PanelRightOpen,
  Link,
  Monitor,
  X,
} from "lucide-react";
import {
  ApiError,
  normalizeDsp,
  RMusicClient,
  type AudioInfo,
  type AuthResult,
  type ConnectionProfile,
  type Endpoint,
  type ProviderManifest,
  type Playlist,
  type Track,
  type User,
  type Zone,
} from "@r-music/sdk";
import { Auth } from "./Auth";
import { Empty, IconButton,Modal } from "./components";
import {NativeOffline,NativeCacheSettings} from "./NativeCache";
import {BrowserOffline} from "./BrowserOffline";
import {forgetListenOwner,observeListenCore} from "./offlineStats";
import {useOfflineStats} from "./useOfflineStats";
import {browserPcmAvailable} from "./audio/usePcmAudio";
import {invalidatePlaylistPages} from "./playlistPageCache";
import {ZonePicker} from "./ZonePicker";
import { Transport } from "./Transport";
import { Library } from "./Library";
import { CollectionBrowser } from "./CollectionBrowser";
import { StatsView } from "./Stats";
const SettingsPage=lazy(()=>import("./Settings").then(module=>({default:module.SettingsView})));
function SettingsView(props:ComponentProps<typeof SettingsPage>){return <Suspense fallback={<div className="page-loading" role="status">正在打开设置…</div>}><SettingsPage {...props}/></Suspense>;}
import {useOutputVisualization} from "./useOutputVisualization";
const PlayerPage=lazy(()=>import("./Player").then(module=>({default:module.Player})));
function Player(props:ComponentProps<typeof PlayerPage>){return <Suspense fallback={<div className="page-loading" role="status">正在打开播放页…</div>}><PlayerPage {...props}/></Suspense>;}
import { ZonesView } from "./Zones";
import {LibraryHome,LibraryManagement} from "./LibraryHub";
import { PlaylistsView } from "./Playlists";
import { DiscoverView } from "./Discover";
import {SongCollectionProvider} from "./SongCollection";
import { AutoplaySettings } from "./AutoplaySettings";
import {SignalPanel} from "./SignalPanel";
import {DspEditor} from "./DspWorkbench";
import {signalGrade} from "./signalGrade";
import {signalResolution} from "./signalResolution";
import {TrackDetails} from "./TrackDetails";
import { useAudio } from "./useAudio";
import { useOutputOwner } from "./useOutputOwner";
import {readClientPreferences,migrateClientPreferences,saveClientPreferences} from "./clientPreferences";
import {preferredZone,browserDeviceName,clientDeviceName} from "./deviceIdentity";
import type { ThemeManifest } from "@r-music/sdk";
import { browserCache } from "./browserCache";
import {
  isNative,
  isAndroidClient,
  nativeBufferedEnd,
  restoreWindowAppearance,
  invokeNative,
  NativeClient,
  nativeProfile,
  nativeOutput,
  nativeProfiles,
  saveNativeProfile,
} from "./native";
export type Route =
  | "quality"
  | "search"
  | "discover"
  | "daily-playlist"
  | "library"
  | "albums"
  | "artists"
  | "playlists"
  | "favorites"
  | "library-home"
  | "library-manage"
  | "stats"
  | "zones"
  | "dsp"
  | "settings"
  | "profile"
  | "core-connection"
  | "player";
export interface Preferences {
  studio_skin?:string;
  studio_skins?:Skin[];
  mobile_ui?:MobileUI;
  mobile_themes?:Partial<Record<MobileUI,string>>;
  player_skin?:"silk"|"midnight"|"paper";
  player_view?:"album"|"lyrics"|"custom";
  player_rotate?:boolean;
  player_atmosphere?:boolean;
  theme: string;
  mode: "dark" | "light" | "system";
  low_performance: boolean;
  custom_themes: ThemeManifest[];
  layouts?: Record<string, unknown>;
  lyric_font?: number;
  lyric_offset?: number;
  lyric_translation?: boolean;
  lyric_mode?: "original"|"bilingual"|"translation";
  lyric_language?: string;
  lyric_romanization?:boolean;
  mini_player_title?:"translation"|"original"|"song";
  mini_player_word_follow?:boolean;
  library_sections?:string[];
  library_columns?:Record<string,number>;
  library_hidden?:string[];
  client_cache_limit?: number;
}
const defaultPreferences: Preferences = {
  theme: "press",
  mode: "light",
  low_performance: false,
  custom_themes: [],
  client_cache_limit: 5 * 1024 ** 3,
};
function readStorage<T>(key: string, fallback: T): T {
  try {
    return JSON.parse(localStorage.getItem(key) || "null") ?? fallback;
  } catch {
    return fallback;
  }
}
function currentBase() {
  if (isNative) return "http://127.0.0.1:7800";
  const pathname = window.location.pathname
    .replace(/\/index\.html$/, "")
    .replace(/\/$/, "");
  return window.location.origin + pathname;
}
export function App() {
  const mobile=useMobileLayout();
  const [base, setBase] = useState(() =>
    isNative ? readStorage("rmusic.core", currentBase()) : currentBase(),
  );
  const [token, setToken] = useState(
    () => sessionStorage.getItem("rmusic.token") || "",
  );
  const api = useMemo(
    () =>
      isNative ? new NativeClient(base, token) : new CoreWebClient(base, token),
    [base, token],
  );
  const [initialized, setInitialized] = useState(true);
  const [user, setUser] = useState<User | null>(null);
  const [coreId,setCoreId]=useState("");
  const [coreCapabilities,setCoreCapabilities]=useState<{browser_pcm?:boolean;measured_listens?:boolean;measured_listens_v2?:boolean;measured_listens_online_v2?:boolean;measured_listens_socket_v2?:boolean;continuous_recovery?:boolean;recovery_controls?:boolean;smart_playlists?:boolean;group_start_v1?:boolean;group_start_list_v1?:boolean;group_queue_v1?:boolean;group_output_v2?:boolean;group_transport_v1?:boolean;group_resync_v1?:boolean;group_queue_jump_v1?:boolean;group_membership_v1?:boolean;group_clock_diagnostics_v1?:boolean;automatic_group_rate_v1?:boolean;group_rate_adjustment_v1?:boolean;group_manual_recovery_v2?:boolean;output_groups_v1?:boolean;device_groups_v1?:boolean;device_purge_v1?:boolean;group_sync_policy_v1?:boolean;dsp_workbench_v1?:boolean;dsp_modules_v1?:boolean}>({});
  const [ready, setReady] = useState(false);
  const [startupUnavailable, setStartupUnavailable] = useState(false);
  useEffect(()=>{const notice=(event:Event)=>notify(String((event as CustomEvent).detail));window.addEventListener("rmusic-notice",notice);return()=>window.removeEventListener("rmusic-notice",notice);},[]);
  const [route, setRoute] = useState<Route>("discover");
  const [dailyPage,setDailyPage]=useState<{picks:Picks;from:"discover"|"library-home"}|null>(null);
  const [dspZoneId,setDspZoneId]=useState("");
  function openDsp(id:string){setDspZoneId(id);setRoute("dsp");}
  const beforePlayer=useRef<Route>("library-home"),catalogOrigin=useRef<Route>("library-home");
  useEffect(()=>{if(route!=="player")beforePlayer.current=route;},[route]);
  const [query, setQuery] = useState("");
  const [libraryAlbumStatus,setLibraryAlbumStatus]=useState("");
  const [catalogHeading,setCatalogHeading]=useState<{id:string;name?:string}|null>(null);
  const [albumId, setAlbumId] = useState("");
  const [artistId, setArtistId] = useState("");
  const [source, setSource] = useState("");
  const [playlistEdit,setPlaylistEdit]=useState<{api:RMusicClient;playlist:Playlist}|null>(null);
  const editingPlaylist=playlistEdit?.api===api?playlistEdit.playlist:undefined;
  const [preferences, setPreferences] = useState<Preferences>(() =>
    readClientPreferences(isAndroidClient?{...defaultPreferences,theme:"graphite",mode:"dark"}:defaultPreferences),
  );
  const [profiles, setProfiles] = useState<ConnectionProfile[]>(() =>
    readStorage("rmusic.profiles", []),
  );
  const visible=usePresentationVisible();
  const portableNative=isAndroidClient;
  const legacyAndroid=portableNative&&mobile;
  const androidSheet=useAndroidPlayerSheet(preferences.low_performance);
  useEffect(()=>{if(!mobile)androidSheet.reset();},[mobile,androidSheet.reset]);
  const hasSavedAndroidCore=portableNative&&readStorage("rmusic.core","")===base;
  const blockedAndroidRestore=useRef<string|undefined>(undefined);
  const startupEpoch=useRef(0);
  const [androidSession,setAndroidSession]=useState<{base:string;available:boolean}>();
  const androidRestoring=hasSavedAndroidCore&&(androidSession?.base!==base||androidSession.available);
  const restoringSession=portableNative?androidRestoring:!!token;
  const [rawZones, setZones] = useState<Zone[]>([]);
  const zones=useMemo(()=>rawZones.map(zone=>({...zone,name:zoneDisplayName(zone),dsp:normalizeDsp(zone.dsp)})),[rawZones]);
  const [endpoints, setEndpoints] = useState<Endpoint[]>([]);
  const [selectedZone, setSelectedZone] = useState("");
  const playbackZones=useMemo(()=>presentedZones(zones,endpoints),[zones,endpoints]);
  useEffect(()=>{const group=endpoints.find(e=>e.kind==="group"&&e.group?.zone_ids.includes(selectedZone));const logical=group&&zones.find(z=>z.endpoint_id===group.id);if(logical)setSelectedZone(logical.id);},[zones,endpoints,selectedZone]);
  const [baseEndpointId] = useState(() => {
    const saved = localStorage.getItem("rmusic.endpoint-id");
    if (saved) return saved;
    const id = `browser-${clientId()}`;
    localStorage.setItem("rmusic.endpoint-id", id);
    return id;
  });
  const endpointId = user?.role === "external" ? `${baseEndpointId}-u-${user.id.replace(/[^a-zA-Z0-9-]/g, "")}` : baseEndpointId;
  const outputOwner = useOutputOwner(endpointId);
  useEffect(()=>{if(isNative&&user)void nativeProfile(api.baseUrl).then(profile=>localStorage.setItem(`rmusic.profile-endpoint:${profile.id}`,endpointId)).catch(()=>{});},[api,user,endpointId]);
  const [deviceName,setDeviceName]=useState(()=>clientDeviceName(browserDeviceName(endpointId)));
  const deviceNameRef=useRef(deviceName);deviceNameRef.current=deviceName;
  useEffect(()=>{
    if((!isNative&&!api.token)||!user?.id)return;
    void api.put("/auth/sessions/current-device",{id:baseEndpointId,name:deviceName,type:portableNative?"android":isNative&&/Windows/i.test(navigator.userAgent)?"windows":"browser"}).catch(()=>{});
  },[api,user?.id,baseEndpointId,deviceName,portableNative]);
  useEffect(()=>{document.title="R-Music · "+deviceName;},[deviceName]);
  const currentUser=useRef(user);currentUser.current=user;
  const selectionKey=()=>"rmusic.selected-zone:"+JSON.stringify([api.baseUrl,currentUser.current?.id]);
  const chooseZone=(id:string)=>{localStorage.setItem(selectionKey(),id);setSelectedZone(id);};
  useEffect(()=>{
    const changed=()=>{const name=clientDeviceName(localStorage.getItem("rmusic.detected-device-name")||browserDeviceName(endpointId));setDeviceName(name);if(isNative)void invokeNative("native_identity",{name}).catch(()=>{});};
    window.addEventListener("rmusic-device-name",changed);
    if(!isNative)localStorage.setItem("rmusic.detected-device-name",browserDeviceName(endpointId));
    if(isNative)invokeNative<{name:string}>("native_identity",{name:localStorage.getItem("rmusic.device-name")}).then(value=>{if(value?.name){localStorage.setItem("rmusic.detected-device-name",value.name);setDeviceName(clientDeviceName(value.name));}}).catch(()=>{});
    return()=>window.removeEventListener("rmusic-device-name",changed);
  },[endpointId]);
  const [nativeOutputRevision,setNativeOutputRevision]=useState(0);
  useEffect(()=>{const changed=()=>setNativeOutputRevision(value=>value+1);window.addEventListener("rmusic-output",changed);return()=>window.removeEventListener("rmusic-output",changed);},[]);
  const [clock, setClock] = useState(Date.now());
  const [plugins, setPlugins] = useState<ProviderManifest[]>([]);
  const [revision, setRevision] = useState(0);
  const [statsRevision, setStatsRevision] = useState(0);
  const [toast, setToast] = useState("");
  const queueKey = "rmusic.queue-open:" + JSON.stringify([api.baseUrl, user?.id]);
  const [queueOpen, setQueueOpen] = useState(false);
  const rememberQueue = useCallback((open: boolean) => {
    setQueueOpen(open);
    if (!portableNative && user) localStorage.setItem(queueKey, JSON.stringify(open));
  }, [portableNative, queueKey, user]);
  const [railHidden, setRailHidden] = useState(() => readStorage("rmusic.rail-hidden", false));
  const [expandedPlayer, setExpandedPlayer] = useState(false);
  useEffect(() => {
    if (route !== "player" || mobile) setExpandedPlayer(false);
  }, [route, mobile]);
  const closeQueue=useCallback(()=>{
    dismissQueue(()=>rememberQueue(false),mobile&&!preferences.low_performance);
  },[mobile,preferences.low_performance,rememberQueue]);
  const [signalOpen,setSignalOpen]=useState(false);
  const [zonePicker,setZonePicker]=useState(false),[zonesPage,setZonesPage]=useState<"devices"|"groups">("devices"),[groupPreset,setGroupPreset]=useState<{ids:string[];key:number}>();
  const [detailTrack,setDetailTrack]=useState<Track|null>(null);
  const [mobileNav, setMobileNav] = useState(false);
  const mobileRoutes=useRef<Route[]>(["discover"]);
  function openDaily(picks:Picks,from:"discover"|"library-home"){
    setDailyPage({picks,from});
    setRoute("daily-playlist");
  }
  function closeDaily(){
    const origin=dailyPage?.from??"discover";
    if(mobileRoutes.current.at(-1)==="daily-playlist"&&mobileRoutes.current.at(-2)===origin)mobileRoutes.current.pop();
    setRoute(origin);
  }
  useEffect(()=>{if(mobile&&mobileRoutes.current.at(-1)!==route)mobileRoutes.current=[...mobileRoutes.current,route].slice(-32);},[mobile,route]);
  useEffect(()=>{
    if(!mobile||!isNative)return;
    const back=(event:Event)=>{
      const overlay=[...document.querySelectorAll('[role="dialog"][aria-modal="true"], [role="menu"]')].some(element=>!element.closest('[hidden]'));
      if(overlay){event.preventDefault();document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true}));return;}
      if(mobileNav){event.preventDefault();setMobileNav(false);return;}
      if(queueOpen){event.preventDefault();closeQueue();return;}
      const coverBack=document.querySelector<HTMLButtonElement>('.listening-room[data-android-lyrics="true"] .mobile-player-top button[aria-label="返回封面"]');
      if(coverBack){event.preventDefault();coverBack.click();return;}
      if(androidSheet.mounted){event.preventDefault();androidSheet.close();return;}
      if(mobileRoutes.current.length>1){event.preventDefault();const player=mobileRoutes.current.at(-1)==="player";mobileRoutes.current.pop();transitionPlayer(()=>setRoute(mobileRoutes.current.at(-1)!),player&&!preferences.low_performance);}
    };
    window.addEventListener('rmusic-system-back',back);return()=>window.removeEventListener('rmusic-system-back',back);
  },[mobile,mobileNav,queueOpen,preferences.low_performance,closeQueue,androidSheet.mounted,androidSheet.close]);
  const [online, setOnline] = useState(false);
  const [offlineOpen,setOfflineOpen]=useState(false),[offlineActive,setOfflineActive]=useState(false),[offlineError,setOfflineError]=useState("");
  const [connectionRevision,setConnectionRevision]=useState(0);
  const currentApi=useRef(api);currentApi.current=api;
  const zoneRead=useRef<{api:RMusicClient;controller:AbortController;promise:Promise<void>}|undefined>(undefined);
  const restoreOffline=useCallback(()=>{setOfflineOpen(true);setOfflineActive(true);},[]);
  const localError=useCallback((error:unknown)=>setOfflineError(error instanceof Error?error.message:String(error)),[]);
  const identityScope=JSON.stringify([api.baseUrl,user?.id||""]);
  const previousIdentity=useRef(identityScope);
  useLayoutEffect(()=>{
    if(previousIdentity.current===identityScope)return;
    previousIdentity.current=identityScope;
    zoneRead.current?.controller.abort();zoneRead.current=undefined;
    setZones([]);setEndpoints([]);setSelectedZone("");setPlugins([]);
    setDetailTrack(null);setPlaylistEdit(null);setCatalogHeading(null);setAlbumId("");setArtistId("");setQuery("");setSource("");setLibraryAlbumStatus("");
    setQueueOpen(!portableNative && !!user && readStorage(queueKey, false));setSignalOpen(false);setZonePicker(false);setGroupPreset(undefined);setZonesPage("devices");setMobileNav(false);
    setOfflineOpen(false);setOfflineActive(false);setOfflineError("");setToast("");
    setDailyPage(null);androidSheet.reset();setRoute("discover");beforePlayer.current="discover";mobileRoutes.current=["discover"];
  },[identityScope,androidSheet.reset,portableNative,queueKey,user]);
  const [systemDark, setSystemDark] = useState(
    () => matchMedia("(prefers-color-scheme: dark)").matches,
  );
  const [busy, setBusy] = useState(false);
  const toastTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const emptyAnalyser = useRef<AnalyserNode | null>(null);
  const notify = useCallback((message: string) => {
    setToast(message);
    clearTimeout(toastTimer.current);
    toastTimer.current = setTimeout(() => setToast(""), 6500);
  }, []);
  useEffect(()=>()=>{clearTimeout(toastTimer.current);},[]);
  const onError = useCallback(
    (error: unknown) => {
      if (error instanceof ApiError && error.status === 401) {
        if(!isNative){browserCache.bind(currentBase(),null);forgetListenOwner(currentBase());}
        sessionStorage.removeItem("rmusic.token");
        setToken("");
        setUser(null);
        if(portableNative)setAndroidSession({base,available:false});
      }
      const message = error instanceof Error ? error.message : "操作失败";
      if(/Failed to fetch|NetworkError|Load failed|error sending request/i.test(message))setOnline(false);
      notify(
        /Failed to fetch|NetworkError|Load failed|error sending request/i.test(
          message,
        )
          ? "暂时无法连接核心，请检查网络与核心服务；本机离线播放可继续。"
          : message,
      );
    },
    [notify,portableNative,base],
  );
  const refreshZones = useCallback(
    () => {
      if(zoneRead.current?.api===api)return zoneRead.current.promise;
      zoneRead.current?.controller.abort();
      const pending={api,controller:new AbortController(),promise:Promise.resolve()};zoneRead.current=pending;
      const timeout=setTimeout(()=>{if(zoneRead.current===pending&&currentApi.current===api){setOnline(false);pending.controller.abort();}},10000);
      pending.promise=api
        .request<{ items: Zone[]; core_reachable?: boolean | null }>("/zones",{signal:pending.controller.signal})
        .then((result) => {
          if(currentApi.current!==api||pending.controller.signal.aborted||zoneRead.current!==pending)return;
          if(!portableNative || typeof result.core_reachable==="boolean") {
            setOnline(!portableNative || result.core_reachable===true);
          }
          setZones(result.items);
          setSelectedZone(current=>preferredZone(result.items,current,endpointId,localStorage.getItem("rmusic.selected-zone:"+JSON.stringify([api.baseUrl,currentUser.current?.id]))||""));
        })
        .catch(error=>{if(currentApi.current===api&&!pending.controller.signal.aborted)onError(error);})
        .finally(()=>{clearTimeout(timeout);if(zoneRead.current===pending)zoneRead.current=undefined;});
      return pending.promise;
    },
    [api, onError,endpointId,portableNative],
  );
  useOfflineStats(api,user,coreId,online,isNative,onError,coreCapabilities.measured_listens===true);
  useEffect(()=>{if(!isNative&&user&&coreId)browserCache.bind(api.baseUrl,{core_id:coreId,user_id:user.id});},[api,user,coreId]);
  useEffect(()=>()=>{if(zoneRead.current?.api===api){zoneRead.current.controller.abort();zoneRead.current=undefined;}},[api]);
  const refreshLibrary = useCallback(() => {
    setRevision((value) => value + 1);
  }, []);
  useEffect(() => {
    let active = true;
    const epoch=startupEpoch.current;
    const current=()=>active&&startupEpoch.current===epoch;
    let controller:AbortController|undefined,timer:ReturnType<typeof setTimeout>|undefined;
    let delay=1000;
    const restoreAndroid=hasSavedAndroidCore&&blockedAndroidRestore.current!==base;
    let androidAvailable:boolean|undefined;
    setReady(false);
    const check=async()=>{
      controller=new AbortController();const timeout=setTimeout(()=>controller?.abort(),10000);
      const options={signal:controller.signal};
      try{
        const statusRequest=api.request<{initialized:boolean;core_id:string;capabilities?:typeof coreCapabilities}>("/status",options);
        // The native connection resolver coalesces route selection for both requests.
        const restoring=portableNative&&androidAvailable!==false&&restoreAndroid;
        const [status,restored]=await Promise.all([statusRequest,restoring?api.request<User>("/auth/me",options):undefined]);
        if(!current())return;
        setCoreId(status.core_id);observeListenCore(api.baseUrl,status.core_id);
        setCoreCapabilities(status.capabilities||{});
        setInitialized(status.initialized);
        if(restored){setUser(restored);setAndroidSession({base,available:true});}
        else if(!portableNative&&(token||isNative)){const value=await api.request<User>("/auth/me",options);if(current())setUser(value);}
        if(current()){setOnline(true);setStartupUnavailable(false);}
        delay=1000;
      }catch(error){
        if(!current())return;
        if(error instanceof ApiError&&error.status===401){
          if(!isNative){browserCache.bind(api.baseUrl,null);forgetListenOwner(api.baseUrl);}
          if(portableNative)setAndroidSession({base,available:false});
          sessionStorage.removeItem("rmusic.token");setToken("");setUser(null);setOnline(false);
        }else {setOnline(false);setStartupUnavailable(true);timer=setTimeout(check,delay);delay=Math.min(delay*2,30000);}
      }finally{clearTimeout(timeout);if(current())setReady(true);}
    };
    const start=async()=>{
      if(restoreAndroid){
        try{
          const profile=await nativeProfile(api.baseUrl);
          if(!current())return;
          androidAvailable=await invokeNative<boolean>("native_session_available",{profileId:profile.id});
          if(!current())return;
          setAndroidSession({base,available:androidAvailable});
        }catch{/* Allow an authenticated network response to restore the session. */}
      }
      if(current())void check();
    };
    let wake:ReturnType<typeof setTimeout>|undefined;
    const networkChanged=()=>{if(currentUser.current)return;clearTimeout(wake);wake=setTimeout(()=>{if(current())setConnectionRevision(v=>v+1);},300);};
    if(isNative)window.addEventListener("rmusic-network-change",networkChanged);
    void start();
    return () => {
      clearTimeout(wake);window.removeEventListener("rmusic-network-change",networkChanged);
      active = false;clearTimeout(timer);controller?.abort();
    };
  }, [api, token, connectionRevision, portableNative, hasSavedAndroidCore, base]);
  useEffect(() => {
    if (!user) return;
    let active=true;
    refreshZones();
    refreshLibrary();
    api
      .get<{ items: Endpoint[] }>("/endpoints")
      .then((r) => {if(active)setEndpoints(r.items);})
      .catch(error=>{if(active)onError(error);});
    api
      .get<{ items: ProviderManifest[] }>("/plugins")
      .then((r) => {if(active){setPlugins(r.items);}})
      .catch(error=>{if(active)onError(error);});
    api
      .get<Partial<Preferences>>("/preferences")
      .then((value) => {if(active)setPreferences(migrateClientPreferences(value,defaultPreferences));})
      .catch(() => {});
    return()=>{active=false;};
  }, [api, user, refreshZones, refreshLibrary, onError]);
  useEffect(() => {
    if (!user||!outputOwner) return;
    let active = true;
    const recovering=new Set<string>();
    const registering=new Set<string>();let registrationId=clientId();let pageClosed=false;
    const browserContinuous=coreCapabilities.browser_pcm===true&&browserPcmAvailable();
    const capabilities:Promise<{continuous_pcm?:boolean;continuous_recovery?:boolean;recovery_controls?:boolean;measured_listens_v2?:boolean;measured_listens_recovery_v2?:boolean;native_group_v1?:boolean}>=isNative?invokeNative<{continuous_pcm?:boolean;continuous_recovery?:boolean;recovery_controls?:boolean;measured_listens_v2?:boolean;measured_listens_recovery_v2?:boolean;native_group_v1?:boolean}>("native_capabilities").catch(()=>({continuous_pcm:false,continuous_recovery:false,recovery_controls:false})):Promise.resolve({continuous_pcm:browserContinuous,continuous_recovery:browserContinuous&&coreCapabilities.continuous_recovery===true,recovery_controls:browserContinuous&&coreCapabilities.continuous_recovery===true&&coreCapabilities.recovery_controls===true});
    const register = () =>
      capabilities.then(async value=>{
        if(!active||pageClosed)return;
        if(isNative&&(portableNative||value.native_group_v1===true)){
          const profile=await nativeProfile(api.baseUrl);if(!active)return;
          const requestId=clientId();registering.add(requestId);
          try{return await invokeNative<Endpoint>("native_endpoint_register",{requestId,registrationId,profileId:profile.id,endpointId,name:deviceNameRef.current,output:nativeOutput()});}
          finally{registering.delete(requestId);}
        }
        return api.post<Endpoint>("/endpoints/browser", {
          id: endpointId,
          registration_id: isNative?undefined:registrationId,
          output_config:isNative?nativeOutput():undefined,
          platform:isNative&&/Android/i.test(navigator.userAgent)?"android":"browser",
          playback_instance:true,
          visualization_v1:isNative||typeof AudioContext!=="undefined",
          continuous_pcm:value?.continuous_pcm===true,
          continuous_recovery:value?.continuous_recovery===true,
          measured_listens_v2:value?.measured_listens_v2===true,
          measured_listens_recovery_v2:value?.measured_listens_recovery_v2===true,
          recovery_controls:value?.recovery_controls===true,
          name: deviceNameRef.current,
        });})
        .then(async (endpoint) => {
          if (!active||pageClosed||!endpoint) return;
          if(isNative&&endpoint.output_config)localStorage.setItem("rmusic.output",JSON.stringify(endpoint.output_config));
          if(isNative&&endpoint.capabilities.native_group_v1===true){
            const snapshot=await api.get<{items:Endpoint[]}>("/endpoints");if(!active)return;
            setEndpoints(snapshot.items);
          }else setEndpoints((old) => [
            ...old.filter((e) => e.id !== endpoint.id),
            endpoint,
          ]);
          if(!isNative)window.dispatchEvent(new CustomEvent("rmusic-browser-registered",{detail:endpoint.id}));
          if(isNative&&endpoint.capabilities.continuous_recovery===true){
            const profile=await nativeProfile(api.baseUrl);if(!active)return;
            const requestId=clientId();recovering.add(requestId);
            try{
              const result=await invokeNative<{zone?:Zone}|null>("native_sequence_restore",{requestId,profileId:profile.id,endpointId: endpoint.id,output:nativeOutput()});
              if(active&&result?.zone)setZones(old=>old.map(zone=>zone.id===result.zone!.id?result.zone!:zone));
            }finally{recovering.delete(requestId);}
          }
        })
        .catch(error=>{if(active)onError(error);});
    register().then(() => {
      if (active) refreshZones();
    });
    const timer = setInterval(()=>{if(!portableNative||presentationVisible())void register();},portableNative?300000:isNative?10000:30000);
    const disconnect=()=>{if(!isNative)void api.request(`/endpoints/browser/${encodeURIComponent(endpointId)}/disconnect?registration_id=${encodeURIComponent(registrationId)}`,{method:"POST",keepalive:true}).catch(()=>{});};
    const closing=()=>{pageClosed=true;disconnect();};
    const reopened=(event:PageTransitionEvent)=>{if(event.persisted&&active){pageClosed=false;registrationId=clientId();void register().then(()=>{if(active)refreshZones();});}};
    if(!isNative){window.addEventListener("pagehide",closing);window.addEventListener("pageshow",reopened);}
    return () => {
      active = false;
      clearInterval(timer);
      for(const requestId of recovering)invokeNative("native_sequence_restore_cancel",{requestId}).catch(()=>{});
      for(const requestId of registering)invokeNative("native_request_cancel",{requestId}).catch(()=>{});
      if(isNative)invokeNative("native_endpoint_disconnect",{registrationId}).catch(()=>{});
      else {window.removeEventListener("pagehide",closing);window.removeEventListener("pageshow",reopened);disconnect();}
    };
  }, [api, user, endpointId, outputOwner,nativeOutputRevision,coreCapabilities.browser_pcm,coreCapabilities.continuous_recovery,coreCapabilities.recovery_controls,portableNative,onError, refreshZones]);
  useEffect(() => {
    if (!user) return;
    if (isNative) {
      let active=true;
      const timer = setInterval(() => {
        if(presentationVisible())void refreshZones();
      },portableNative?300000:2000);
      const endpoints=setInterval(()=>{if(!presentationVisible())return;void api.get<{items:Endpoint[]}>("/endpoints").then(value=>{if(active)setEndpoints(value.items);}).catch(()=>{});},portableNative?300000:5000);
      let wake:ReturnType<typeof setTimeout>|undefined;
      let remoteRefresh:ReturnType<typeof setTimeout>|undefined;
      const refreshRemoteView=()=>{
        if(!active||!presentationVisible()||remoteRefresh!==undefined)return;
        remoteRefresh=setTimeout(()=>{
          remoteRefresh=undefined;
          if(!active||!presentationVisible())return;
          void refreshZones();
          void api.get<{items:Endpoint[]}>("/endpoints").then(value=>{if(active)setEndpoints(value.items);}).catch(()=>{});
        },50);
      };
      const networkChanged=()=>{
        clearTimeout(wake);
        wake=setTimeout(()=>{
          if(!active)return;
          if(zoneRead.current?.api===api){zoneRead.current.controller.abort();zoneRead.current=undefined;}
          void refreshZones();
        },300);
      };
      window.addEventListener("rmusic-network-change",networkChanged);
      let removed=false;const listeners:(()=>void)[]=[];
      if(portableNative) {
        window.addEventListener("rmusic-activity-visibility",refreshRemoteView);
        document.addEventListener("visibilitychange",refreshRemoteView);
        void listen<Partial<Zone>&{id:string}>("native-client-zone",event=>{if(active&&presentationVisible())setZones(old=>old.some(z=>z.id===event.payload.id)?old.map(z=>z.id===event.payload.id?{...z,...event.payload}:z):event.payload.queue?[...old,event.payload as Zone]:old);}).then(remove=>{if(removed)remove();else listeners.push(remove);}).catch(()=>{});
        void listen("native-client-remote",refreshRemoteView).then(remove=>{if(removed)remove();else listeners.push(remove);}).catch(()=>{});
        void listen<{type:string;data:unknown}>("native-core-event",({payload:event})=>{
          if(!active||!presentationVisible())return;
          if(event.type==="connection_ready"){setOnline(true);refreshRemoteView();}
          if(event.type==="connection_interrupted")setOnline(false);
          if(event.type==="zone_progress"){
            const progress=event.data as Partial<Zone>&{id:string};
            setZones(old=>old.map(z=>z.id===progress.id&&(!("playback_id"in progress)||progress.playback_id===z.playback_id)?{...z,...progress}:z));
          }
          if(event.type==="autoplay")window.dispatchEvent(new Event("rmusic-radio-ready"));
          if(event.type==="device_groups")window.dispatchEvent(new Event("rmusic-device-groups-changed"));
          if(event.type==="devices"||event.type==="endpoints")window.dispatchEvent(new Event("rmusic-devices-changed"));
          if(event.type==="library")refreshLibrary();
          if(event.type==="stats")setStatsRevision(value=>value+1);
          if(event.type==="playlists"||event.type==="connection_ready"){invalidatePlaylistPages(api);window.dispatchEvent(new CustomEvent("rmusic-playlists-changed",{detail:api}));}
          if(event.type==="endpoint_signal"){const data=event.data as {endpoint_id:string;playback:Endpoint["playback"]};setEndpoints(old=>old.map(e=>e.id===data.endpoint_id?{...e,playback:data.playback}:e));}
          if(event.type==="device_volume"){const data=event.data as {endpoint_id:string;state:Endpoint["device_volume"]};setEndpoints(old=>old.map(e=>e.id===data.endpoint_id?{...e,device_volume:data.state}:e));}
        }).then(remove=>{if(removed)remove();else listeners.push(remove);}).catch(()=>{});
      }
      const library = setInterval(()=>{if(presentationVisible())refreshLibrary();},portableNative?300000:30000);
      return () => {
        active=false;removed=true;listeners.forEach(remove=>remove());clearTimeout(wake);window.removeEventListener("rmusic-network-change",networkChanged);clearInterval(endpoints);
        clearTimeout(remoteRefresh);
        window.removeEventListener("rmusic-activity-visibility",refreshRemoteView);
        document.removeEventListener("visibilitychange",refreshRemoteView);
        clearInterval(timer);
        clearInterval(library);
      };
    }
    let active = true;
    let socket: WebSocket | undefined;
    let retry: ReturnType<typeof setTimeout> | undefined;
    let delay = 1000;
    let generation=0;
    let ticketRequest:AbortController|undefined;
    const connect = () => {
      const attempt=++generation;
      const controller=new AbortController();ticketRequest=controller;
      const timeout=setTimeout(()=>controller.abort(),10000);
      api
        .events((event) => {
          if(!active||attempt!==generation)return;
          setOnline(true);
          if(event.type==="browser_visualization_lease")window.dispatchEvent(new CustomEvent("rmusic-visualization-lease",{detail:event.data}));
          if(event.type==="sequence_invalidated")window.dispatchEvent(new CustomEvent("rmusic-sequence-invalidated",{detail:event.data}));
          if (event.type === "zones" || event.type === "zones_changed") {
            const data = event.data as { items?: Zone[] };
            if (Array.isArray(data?.items)) setZones(data.items);
            else refreshZones();
          }
          if (event.type === "zone_progress") {
            const progress = event.data as Partial<Zone> & { id: string };
            setZones((old) =>
              old.map((zone) =>
                zone.id === progress.id && (!("playback_id" in progress)||progress.playback_id===zone.playback_id) ? { ...zone, ...progress } : zone,
              ),
            );
          }
          if(event.type==="playlists"){invalidatePlaylistPages(api);window.dispatchEvent(new CustomEvent("rmusic-playlists-changed",{detail:api}));}
          if(event.type==="autoplay")window.dispatchEvent(new Event("rmusic-radio-ready"));
          if(event.type==="device_groups")window.dispatchEvent(new Event("rmusic-device-groups-changed"));
          if (event.type === "library") refreshLibrary();
          if (event.type === "stats") setStatsRevision((value) => value + 1);
          if(event.type==="endpoint_signal"){
            const data=event.data as {endpoint_id:string;playback:Endpoint["playback"]};
            setEndpoints(old=>old.map(endpoint=>endpoint.id===data.endpoint_id?{...endpoint,playback:data.playback}:endpoint));
          }
          if(event.type==="device_volume") {const data=event.data as {endpoint_id?:string;state?:Endpoint["device_volume"]};if(data?.state)setEndpoints(current=>current.map(endpoint=>endpoint.id===data.endpoint_id?{...endpoint,device_volume:data.state}:endpoint));}
          if(event.type==="endpoints")api.get<{items:Endpoint[]}>("/endpoints").then(value=>{if(active&&attempt===generation)setEndpoints(value.items);}).catch(()=>{});
          if (event.type === "plugins")
            api
              .get<{ items: ProviderManifest[] }>("/plugins")
              .then((r) => {if(active&&attempt===generation)setPlugins(r.items);})
              .catch(() => {});
        },controller.signal)
        .then((ws) => {
          if (!active||attempt!==generation) {
            ws.close();
            return;
          }
          socket = ws;
          ws.onopen = () => {
            if(!active||attempt!==generation)return;
            setOnline(true);
            delay = 1000;
            refreshZones();
            invalidatePlaylistPages(api);
            window.dispatchEvent(new CustomEvent("rmusic-playlists-changed",{detail:api}));
          };
          ws.onclose = () => {
            if(!active||attempt!==generation)return;
            generation++;
            setOnline(false);
            retry = setTimeout(connect, delay);
            delay = Math.min(delay * 2, 30000);
          };
        })
        .catch((error) => {
          if(!active||attempt!==generation)return;
          generation++;
          setOnline(false);
          if(error instanceof ApiError&&(error.status===401||error.status===403)){onError(error);return;}
          retry = setTimeout(connect, delay);
          delay = Math.min(delay * 2, 30000);
        }).finally(()=>{clearTimeout(timeout);if(ticketRequest===controller)ticketRequest=undefined;});
    };
    connect();
    return () => {
      active = false;
      clearTimeout(retry);
      ticketRequest?.abort();
      socket?.close();
    };
  }, [api, user, refreshZones, refreshLibrary,onError,connectionRevision]);
  useEffect(() => {
    const media = matchMedia("(prefers-color-scheme: dark)");
    const change = () => setSystemDark(media.matches);
    media.addEventListener("change", change);
    return () => media.removeEventListener("change", change);
  }, []);
  useEffect(() => {
    browserCache.register(currentBase()).catch(() => {});
  }, []);
  useEffect(()=>{
    const start=()=>setOfflineActive(true),stop=()=>setOfflineActive(false);
    window.addEventListener("rmusic-offline",start);window.addEventListener("rmusic-online",stop);window.addEventListener("rmusic-offline-stop",stop);
    return()=>{window.removeEventListener("rmusic-offline",start);window.removeEventListener("rmusic-online",stop);window.removeEventListener("rmusic-offline-stop",stop);};
  },[]);
  useEffect(() => {
    if (isNative)
      nativeProfiles()
        .then((items) =>
          setProfiles(
            items.map((p) => ({ id: p.id, name: p.name, url: p.address })),
          ),
        )
        .catch(() => {});
  }, []);
  useEffect(() => {
    applySkin(preferences,preferences.mode==="system"?(systemDark?"dark":"light"):preferences.mode,mobile,legacyAndroid);
    restoreWindowAppearance();
    document.documentElement.dataset.performance = preferences.low_performance
      ? "low"
      : "normal";
  }, [preferences, systemDark,mobile,legacyAndroid]);
  const savePreferences = useCallback(
    async (patch: Partial<Preferences>) => {
      const value = { ...preferences, ...patch };
      saveClientPreferences(value);
      setPreferences(value);
    },
    [preferences],
  );
  const studioTabs=useMemo(()=>activeSkin(preferences,legacyAndroid).tabs,[preferences,legacyAndroid]);
  const zone = zones.find((z) => z.id === selectedZone);
  const nextLyricTrack=zone&&!zone.shuffle&&zone.repeat!=="one"?zone.queue[zone.queue_index+1]:undefined;
  useEffect(()=>{
    if(!user||!zone?.track)return;let active=true;
    void loadLyrics(api,zone.track.id).then(()=>{if(active&&!portableNative&&nextLyricTrack)void loadLyrics(api,nextLyricTrack.id).catch(()=>{});}).catch(()=>{});
    return()=>{active=false;};
  },[api,user?.id,zone?.track?.id,nextLyricTrack?.id,portableNative]);
  const localZones = zones.filter((z) => z.endpoint_id === endpointId);
  const playbackZone = outputOwner
    ? localZones.find((z) => z.status === "playing") || localZones[0]
    : undefined;
  const recoveredZone=useCallback((value:Zone)=>setZones(old=>old.map(zone=>zone.id===value.id?value:zone)),[]);
  const audio = useAudio(api, playbackZone, endpoints, notify,recoveredZone,online,onError);
  useOutputVisualization(api,playbackZone,audio.analyserRef,audio.outputPosition||audio.position);
  const selectedLocal = zone?.id === playbackZone?.id;
  const [remoteInfo, setRemoteInfo] = useState<AudioInfo | null>(null);
  const [remoteError, setRemoteError] = useState("");
  useEffect(() => {
    let active = true;
    setRemoteInfo(null);setRemoteError("");
    if (!selectedLocal && zone?.track) api.get<AudioInfo>(`/library/tracks/${encodeURIComponent(zone.track.id)}/audio-info`)
      .then(info => {if (active) setRemoteInfo(info);})
      .catch(error => {if (active) setRemoteError(error.message);});
    return () => {active = false;};
  }, [api, selectedLocal, zone?.track?.id]);
  const selectedSourceInfo = selectedLocal ? audio.sourceInfo : remoteInfo;
  const selectedSourceError = selectedLocal ? audio.sourceError : remoteError;
  const labels=useOutputLabels(api,!!user,JSON.stringify([zonePicker,route==="zones",endpoints.map(e=>[e.id,e.name,e.online])]));
  const selectedEndpoint=endpoints.find(endpoint=>endpoint.id===zone?.endpoint_id);
  const androidMediaRegistered=portableNative&&endpoints.some(item=>item.id===endpointId&&item.capabilities.native_client_v1===true);
  const remoteMediaZone=zone&&zone.endpoint_id!==endpointId?zone.id:null;
  const remoteMediaLegacyGroups=user?.role!=="external"&&coreCapabilities.group_start_list_v1===true;
  useEffect(()=>{
    if(!androidMediaRegistered||!user)return;
    let current=true;
    void nativeProfile(api.baseUrl).then(profile=>{
      if(current)void invokeNative("native_select_media_zone",{profileId:profile.id,zoneId:remoteMediaZone,legacyGroups:remoteMediaLegacyGroups}).catch(()=>{});
    }).catch(()=>{});
    return()=>{current=false;};
  },[api,androidMediaRegistered,user?.id,remoteMediaZone,remoteMediaLegacyGroups]);
  const needsRemoteClock=visible&&!selectedLocal&&zone?.status==="playing"&&selectedEndpoint?.online!==false;
  useEffect(()=>{
    if(!needsRemoteClock)return;
    const tick=()=>setClock(Date.now());
    tick();const timer=setInterval(tick,1000);
    return()=>clearInterval(timer);
  },[needsRemoteClock,zone?.playback_id,zone?.id]);
  const selectedPlaying=selectedEndpoint?.online!==false&&(selectedLocal?audio.playing:remoteOutputPlaying(zone,selectedEndpoint,clock));
  const selectedWaiting=!!zone?.track&&zone.status==="playing"&&!selectedPlaying;
  const selectedOutputLabel=outputLabel(zone,selectedEndpoint,labels)+(selectedEndpoint?.online===false?" · 已离线":"");
  const selectedPath=selectedLocal&&audio.nativeState.track_id===zone?.track?.id?audio.nativeState.signal_path:selectedEndpoint?.playback?.track_id===zone?.track?.id&&(!zone?.playback_id||selectedEndpoint?.playback?.core_playback_id===zone.playback_id)?selectedEndpoint?.playback?.signal_path:undefined;
  const grade=signalGrade(zone,selectedSourceInfo,selectedEndpoint,selectedPath,selectedLocal?audio.adaptive:0);
  const zoneTime = zone
    ? typeof zone.updated_at === "number"
      ? zone.updated_at
      : Date.parse(zone.updated_at)
    : clock;
  const remoteElapsed =
    selectedPlaying
      ? Math.min(selectedEndpoint?.capabilities.http_pull===true?1.2:Infinity,Math.max(0, (clock - zoneTime) / 1000) || 0)
      : 0;
  const groupTransport=useGroupTransport({api,coreId,userId:user?.id,list:user?.role!=="external"&&coreCapabilities.group_start_list_v1===true,supported:user?.role!=="external"&&coreCapabilities.group_transport_v1===true,jump:coreCapabilities.group_queue_jump_v1===true,recovery:coreCapabilities.group_manual_recovery_v2===true,onError,onChange:refreshZones});
  const displayedPosition = selectedLocal&&zone?.status==="paused"&&audio.nativeState.group_managed&&audio.nativeState.zone_id===zone.id?zone.position:selectedLocal
    ? audio.position
    : Math.min(
        (zone?.position || 0) + remoteElapsed,
        zone?.track?.duration || Infinity,
      );
  const bufferedEnd = portableNative && selectedLocal ? nativeBufferedEnd(audio.nativeState, zone) : undefined;
  async function command(action: string, body?: unknown) {
    if (!zone) return;
    const changing=selectedLocal&&selectedEndpoint?.capabilities.native_client_v1!==true&&(["next","previous","stop"].includes(action)||(action==="play"&&!!(body as {track_id?:string})?.track_id));
    try {
      const grouped=groupTransport.route(zone,action,body,isNative||!selectedLocal);
      if(grouped!==false&&await grouped)return;
      if(currentApi.current!==api)return;
      if(action==="seek"&&selectedLocal&&selectedEndpoint?.capabilities.native_client_v1!==true)audio.seek((body as {position:number}).position);
      if(changing)await audio.interrupt();
      if (action === "play" && selectedLocal)
        await audio.unlock(!changing).catch(() => {});
      const result = await api.post<Zone>(`/zones/${zone.id}/${action}`, body);
      setZones((current) =>
        current.map((z) => (z.id === result.id ? {...z,...result} : z)),
      );
      if(changing)audio.finishChange();
    } catch (error) {
      if(changing)audio.finishChange();
      onError(error);
    }
  }
  async function patchZone(patch: Partial<Zone>) {
    if (!zone) return;
    try {
      const mode=selectedEndpoint?.output_config?.volume_control;
      if (patch.volume!==undefined&&mode==="device") {
        const state=await api.put<{supported:boolean;value:number}>(`/outputs/${encodeURIComponent(zone.endpoint_id)}/volume`,{value:patch.volume});
        setEndpoints(current=>current.map(endpoint=>endpoint.id===zone.endpoint_id?{...endpoint,device_volume:state}:endpoint));return;
      }
      if ((patch.volume!==undefined||patch.muted!==undefined)&&mode==="fixed")return;
      if (patch.muted!==undefined&&mode==="device")return;
      const result = await api.patch<Zone>(`/zones/${zone.id}`, patch);
      setZones((current) =>
        current.map((z) => (z.id === result.id ? {...z,...result} : z)),
      );
    } catch (error) {
      onError(error);
    }
  }
  async function play(track: Track, list?: Track[], exactVersion=false) {
    if (!track.available) {
      notify("当前音源不可播放，请检查源插件或文件状态。");
      return;
    }
    if (!zone) {
      notify("请先在音区页面创建一个播放音区。");
      return;
    }
    setBusy(true);
    try {
      if (selectedLocal&&selectedEndpoint?.capabilities.native_client_v1!==true) {await audio.interrupt();await audio.unlock(false).catch(() => {});}
      const queue=list?.length?list:[track],index=Math.max(0,queue.findIndex(item=>item.id===track.id));
      await api.put(`/zones/${zone.id}/queue`, {track_ids:queue.map(item=>item.id),queue_index:index,shuffle:false});
      await command("play", exactVersion?{track_id:track.id,exact_version:true}:{queue_index:index});
    } catch (error) {
      if(selectedLocal)audio.finishChange();
      onError(error);
    } finally {
      setBusy(false);
    }
  }
  async function startRadio(track:Track) {
    if(!zone)return;
    try {
      await api.patch("/zones/"+encodeURIComponent(zone.id),{autoplay:{enabled:true,source:"local",fallback_local:true}});
      await play(track,[track]);
    } catch(error) {onError(error);}
  }
  async function enqueue(track: Track) {
    if (!zone) return;
    try {
      const updated=await api.post<Zone>(`/zones/${zone.id}/queue`, { track_ids: [track.id] });
      setZones(current=>current.map(item=>item.id===updated.id?updated:item));
      notify("已添加到播放队列");
    } catch (error) {
      onError(error);
    }
  }
  async function playNext(track: Track) {
    if (!zone) { notify("请先选择播放设备"); return; }
    try {
      const updated=await api.post<Zone>(`/zones/${encodeURIComponent(zone.id)}/queue`,{track_ids:[track.id],play_next:true});
      setZones(current=>current.map(item=>item.id===updated.id?updated:item));
      notify("已设为下一首播放");
    } catch(error) { onError(error); }
  }
  async function connect(url: string, fingerprint?: string, alternatives?:import("@r-music/sdk").CoreAddress[]) {
    if (!isNative) return;
    const clean = url.replace(/\/+$/, "");
    try {
      if (isNative) await saveNativeProfile(clean, fingerprint, alternatives);
      localStorage.setItem("rmusic.core", JSON.stringify(clean));
      const value = [
        ...profiles.filter((p) => p.url !== clean),
        { id: clientId(), name: new URL(clean).hostname, url: clean, alternatives },
      ];
      setProfiles(value);
      localStorage.setItem("rmusic.profiles", JSON.stringify(value));
      setUser(null);
      setCoreId("");
      if(portableNative){blockedAndroidRestore.current=clean;setAndroidSession({base:clean,available:false});}
      sessionStorage.removeItem("rmusic.token");
      setToken("");
      setBase(clean);
    } catch (error) {
      onError(error);
      throw error;
    }
  }
  function authenticated(result: AuthResult) {
    const value = isNative ? "native-session" : result.token;
    sessionStorage.setItem("rmusic.token", value);
    setToken(value);
    setUser(result.user);
    if(portableNative){blockedAndroidRestore.current=undefined;setAndroidSession({base,available:true});}
    setInitialized(true);
    setOnline(true);
  }
  async function logout() {
    if(!isNative)browserCache.bind(api.baseUrl,null);
    forgetListenOwner(api.baseUrl);
    const revoke=api.post("/auth/logout").catch(() => {});
    startupEpoch.current++;
    if(portableNative){blockedAndroidRestore.current=base;setAndroidSession({base,available:false});}
    setConnectionRevision(value=>value+1);
    sessionStorage.removeItem("rmusic.token");
    setToken("");
    setUser(null);
    await revoke;
  }
  const seek = (position: number) => {
    void command("seek", { position });
  };
  const openOffline=()=>{setOfflineError("");setOfflineOpen(true);};
  const offlineDialog=<div hidden={!offlineOpen}><Modal active={offlineOpen} wide title="离线聆听" onClose={()=>setOfflineOpen(false)}>
    <div className="offline-connection"><span>{online?"核心已连接":"核心未连接"}</span><button className="text-button" onClick={()=>setConnectionRevision(value=>value+1)}>重试连接</button></div>
    {offlineError&&<p role="alert" className="form-error">{offlineError}</p>}
    {isNative?<NativeOffline active={offlineOpen} api={api} onError={localError} onRestore={restoreOffline}/>:<BrowserOffline onError={localError}/>}
  </Modal></div>;
  const layout=mobileUI(preferences.mobile_ui,legacyAndroid);
  const studioClass=`studio-app ${railHidden?"rail-hidden":""} ${expandedPlayer&&route==="player"&&!mobile?"expanded-player":""} ${legacyAndroid&&(layout==="classic"||layout==="liser")?"legacy-mobile":""} ${legacyAndroid&&layout==="compact"?"compact-mobile":""} app-shell ${mobile?"mobile-app":""} ${queueOpen?"with-queue":""} ${route==="player"?"player-route":""}`;
  const studioNavigation=<StudioShell railHidden={railHidden} toggleRail={()=>{
    setRailHidden(!railHidden);localStorage.setItem("rmusic.rail-hidden",JSON.stringify(!railHidden));
  }} inert={androidSheet.presented} layout={layout} legacyAndroid={legacyAndroid} tabs={studioTabs} mobile={mobile} route={route} user={user?.username??""} online={online} device={deviceName} output={selectedOutputLabel} showZones={()=>setZonePicker(true)} query={query}
    menu={mobileNav} setMenu={setMobileNav} offline={openOffline} logout={()=>void logout()}
    search={value=>{setQuery(value);setAlbumId("");setArtistId("");setRoute("search");}}
    navigate={next=>{setRoute(next);setAlbumId("");setArtistId("");setQuery("");setPlaylistEdit(null);setMobileNav(false);if(mobile)setQueueOpen(false);}}/>;
  const title = routeTitle(route);
  if (!user && restoringSession && startupUnavailable) return <><main className="loading-screen">
    <WifiOff aria-hidden="true"/>
    <h1>无法连接 Core</h1>
    <p>请检查网络与 Core 服务。已保存的登录仍可用于重连。</p>
    <div className="connection-start-actions">
      <button className="primary" disabled={!ready} onClick={()=>setConnectionRevision(value=>value+1)}>{ready?"重试连接":"正在连接…"}</button>
      <button className="text-button" onClick={()=>void logout()}>{isNative?"退出登录并选择 Core":"退出登录"}</button>
      <button className="text-button" onClick={openOffline}>离线聆听</button>
    </div>
  </main>{offlineDialog}</>;
  if (!user && restoringSession) return <><div className={studioClass} data-route={route} data-mobile-style={layout}>
    {studioNavigation}
    <main className="main-content" aria-label={title}>
      <header className="studio-page-title"><h1>{route==="discover"?"发现音乐":title}</h1></header>
      {route==="discover"&&<section className="studio-feature">
        <ListeningDeck api={api} tracks={[]} canPlay={false} play={play} details={setDetailTrack} setup={openOffline} loading/>
      </section>}
      <div className="connection-recovery" role="status"><div><span>正在连接 Core…</span></div><button onClick={openOffline}>离线聆听</button></div>
    </main>
  </div>{offlineDialog}</>;
  if (!user && !ready && !isNative) return <main className="loading-screen" role="status">正在连接 Core…</main>;
  if (!user)
    return (
      <><Auth
        api={api}
        initialized={initialized}
        loginClient={{id:baseEndpointId,name:deviceName,type:portableNative?"android":isNative&&/Windows/i.test(navigator.userAgent)?"windows":"browser"}}
        onAuth={authenticated}
        profiles={profiles}
        onConnect={connect}
        onOffline={openOffline}
      />{offlineDialog}</>
    );
  const playerView=(
    <Player
      expanded={expandedPlayer}
      toggleExpanded={mobile?undefined:()=>setExpandedPlayer(value=>!value)}
      onDsp={zone?()=>{if(mobile)setQueueOpen(false);androidSheet.close();openDsp(zone.id);}:undefined}
      outputLabel={selectedOutputLabel}
      api={api}
      zone={zone}
      position={displayedPosition}
      bufferedEnd={bufferedEnd}
      androidCoverLyrics={mobile}
      legacyAndroid={legacyAndroid}
      playing={selectedPlaying}
      waiting={selectedWaiting}
      preferences={preferences}
      savePreferences={savePreferences}
      seek={seek}
      command={command}
      patchZone={patchZone}
      busy={busy||groupTransport.busy}
      analyser={selectedLocal ? audio.analyserRef : emptyAnalyser}
      adaptive={selectedLocal ? audio.adaptive : 0}
      nativeState={selectedLocal ? audio.nativeState : {}}
      onClose={()=>{
        if(mobile&&queueOpen){closeQueue();return;}
        if(androidSheet.mounted){androidSheet.close();return;}
        if(mobile)setQueueOpen(false);
        transitionPlayer(()=>setRoute(beforePlayer.current),mobile&&!preferences.low_performance);
      }}
      onSceneReady={androidSheet.mounted?androidSheet.ready:undefined}
      appearanceActive={!androidSheet.mounted||androidSheet.presented&&androidSheet.phase!=="closing"}
      showQueue={()=>queueOpen?closeQueue():rememberQueue(true)}
      showZones={()=>setZonePicker(true)}
      sourceInfo={selectedSourceInfo}
      sourceError={selectedSourceError}
      contextRate={selectedLocal?audio.contextSampleRate:0}
      localOutput={selectedLocal}
      endpoint={endpoints.find(endpoint => endpoint.id === zone?.endpoint_id)}
      onError={onError}
    />
  );
  return (
    <ExternalAccountContext.Provider value={user.role==="external"}><CatalogNavigation api={api} location={route} onError={onError} navigate={(kind,id,name)=>{androidSheet.close();if(route!=="library"||!albumId&&!artistId)catalogOrigin.current=route==="player"?beforePlayer.current:route;setCatalogHeading({id,name});setPlaylistEdit(null);setDetailTrack(null);setAlbumId(kind==="album"?id:"");setArtistId(kind==="artist"?id:"");setSource("");setQuery("");if(mobile)setQueueOpen(false);setRoute("library");}}><PersonalRecommendations key={JSON.stringify([coreId,user.id,api.baseUrl])} api={api} canPlay={!!zone} play={play} onError={onError}><SongCollectionProvider api={api} enqueue={enqueue} playNext={playNext} plugins={plugins} notify={notify} onError={onError}><TrackContextMenuProvider currentTrackId={zone?.track?.id} currentPlaying={zone?.status==="playing"} onCurrentToggle={()=>command(zone?.status==="playing"?"pause":"play")} api={api} location={route} play={play} startRadio={zone?startRadio:undefined} enqueue={enqueue} playNext={playNext} details={setDetailTrack} onError={onError} downloadOffline={isNative?async track=>{notify("正在下载，完成后可在已下载中播放");await api.post("/downloads",{track_id:track.id});notify(/Android/i.test(navigator.userAgent)?"已加入下载任务，可在已下载中查看进度":"已下载，可在已下载中离线播放");}:async track=>{notify("正在下载原文件到此浏览器");await browserCache.rememberTrack(track,api.baseUrl);await browserCache.cacheFile(await api.mediaUrl(track.id),track.id);notify("已保存，可在本机离线音乐中播放");}}><div
      data-route={route}
      data-mobile-style={layout}
      data-catalog-detail={!!(albumId||artistId)}
      data-mobile-player={mobile||undefined}
      className={studioClass}
    >
      {studioNavigation}
      <main className="main-content" aria-label={title} inert={androidSheet.presented||undefined}>
        <PageMemory scope={JSON.stringify([coreId,user.id,api.baseUrl])} page={JSON.stringify([route,...(["library","search","albums","artists"].includes(route)?[query,source,albumId,artistId,libraryAlbumStatus]:[])])} active={route!=="player"} cacheable={!["settings","quality","zones","profile","core-connection","dsp"].includes(route)}>
        {!["player","discover","daily-playlist","profile","core-connection","search","favorites","dsp"].includes(route)&&!albumId&&!artistId&&<header className="studio-page-title"><h1>{routeTitle(route)}</h1></header>}
        {!online&&<div className="connection-recovery" role="status"><div><strong>与核心的连接已中断</strong><span>正在重连，页面内容已保留。恢复连接后请检查播放状态。</span></div><button onClick={()=>setConnectionRevision(v=>v+1)}>立即重连</button><button onClick={openOffline}>离线聆听</button></div>}
        {["library-home","library","albums","artists","library-manage"].includes(route)&&(!mobile||!albumId&&!artistId)&&<div className="page-heading page-toolbar library-toolbar">
          <nav className="library-tabs" aria-label="曲库浏览方式">{([['library-home','首页'],['albums','专辑'],['artists','艺术家'],['library','歌曲'],['library-manage','库管理']] as const).map(([id,label])=><button key={id} aria-current={route===id?'page':undefined} onClick={()=>{setLibraryAlbumStatus("");setRoute(id);setQuery("");setAlbumId("");setArtistId("");}}>{mobile&&id==='library-home'?'概览':label}</button>)}</nav>
          <div className="heading-actions">
              {user.role!=="external"&&<IconButton
                label="扫描与音乐目录"
                onClick={() => setRoute("settings")}
              >
                <Plus />
              </IconButton>}
            <IconButton
              label={queueOpen ? "收起队列" : "打开队列"}
              onClick={() => rememberQueue(!queueOpen)}
            >
              {queueOpen ? <PanelRightClose /> : <PanelRightOpen />}
            </IconButton>
          </div>
        </div>}
        {mobile&&route==="search"&&!query.trim()&&<MobileSearchBrowse navigate={setRoute} offline={openOffline}/>}
        {route==="quality"&&(isNative?<><AndroidQuality onError={onError} notify={notify}/><NativeCacheSettings api={api} onError={onError} notify={notify}/></>:<Empty headline="安卓音质设置" description="在安卓客户端中分别选择 Wi-Fi 与移动流量音质。"/>)}
        {route === "search" && <SearchPage api={api} query={query} play={play} details={setDetailTrack} browse={(kind,id,name)=>{catalogOrigin.current=route;setCatalogHeading({id,name});setAlbumId(kind==="album"?id:"");setArtistId(kind==="artist"?id:"");setQuery("");setSource("");setRoute("library");}}/>}
        {route === "discover" && (
          <DiscoverView openDaily={picks=>openDaily(picks,"discover")} key={JSON.stringify([coreId,user?.id])} external={user.role==="external"} coverActions={legacyAndroid} unifiedSources={portableNative}
            api={api}
            cacheScope={JSON.stringify([coreId,user?.id])}
            plugins={plugins}
            zone={zone}
            play={play}
            startRadio={zone?startRadio:undefined}
            enqueue={enqueue}
            refreshZones={refreshZones}
            onError={onError}
            goSettings={() => setRoute("settings")}
            goDevices={legacyAndroid?undefined:()=>setZonePicker(true)}
            goAlbums={legacyAndroid?undefined:()=>{setAlbumId("");setArtistId("");setSource("local");setRoute("albums");}}
            showDetails={setDetailTrack}
          />
        )}
        {route==="daily-playlist"&&dailyPage&&<DailyPlaylistPage api={api} picks={dailyPage.picks} play={play}
          canPlay={!!zone} onError={onError} back={closeDaily} backLabel={dailyPage.from==="discover"?"返回发现":"返回音乐库"}/>}
        {(route === "library" || route === "albums" || route === "artists") && (!(albumId||artistId)||!!editingPlaylist) && (
            <>{editingPlaylist&&<section className="settings-section">
              <div className="section-heading"><h2>正在修改：{editingPlaylist.name}</h2><button onClick={()=>{setPlaylistEdit(null);setRoute("playlists");}}>取消修改</button></div>
              <p className="muted">调整搜索、来源、专辑或艺术家，预览已入库的匹配曲目，再更新原歌单。</p>
              {(albumId||artistId)&&<button onClick={()=>{setAlbumId("");setArtistId("");}}>取消专辑／艺术家限制</button>}
            </section>}
            <label className="source-filter">来源<select value={source} aria-label="曲库来源" onChange={e=>setSource(e.target.value)}>
              {[
                ["", "全部"],
                ["local", "本地"],
                ...plugins.map((plugin) => [plugin.id, plugin.name]),
              ].map(([id, label]) => (
                <option key={id} value={id}>{label}</option>
              ))}
              {editingPlaylist&&source&&source!=="local"&&!plugins.some(p=>p.id===source)&&<option value={source}>原歌单音源（不可用）</option>}
              </select>{!source&&<span className="muted">同版本优先本地</span>}</label></>
        )}
        {route === "library" && (
            <Library restricted={user.role==="external"}
              canEdit={user.role==="admin"}
              onCloseCatalog={()=>{setAlbumId("");setArtistId("");setCatalogHeading(null);setRoute(catalogOrigin.current);}}
              catalogTitle={catalogHeading?.id===(albumId||artistId)?catalogHeading.name:undefined}
              key={`${source}:${libraryAlbumStatus}`}
              api={api}
              query={query}
              albumStatus={libraryAlbumStatus}
              source={source}
              albumId={albumId}
              artistId={artistId}
              revision={revision}
              play={play}
              startRadio={zone?startRadio:undefined}
              enqueue={enqueue}
              onError={onError}
              goSettings={() => setRoute("settings")}
              provider={plugins.find((p) => p.id === source)}
              playingId={zone?.track?.id}
              showDetails={setDetailTrack}
              browseAlbum={id=>{setAlbumId(id);setArtistId("");setQuery("");}}
              smartPlaylists={coreCapabilities.smart_playlists===true}
              editingPlaylist={editingPlaylist}
              onPlaylistSaved={()=>{if(editingPlaylist){setPlaylistEdit(null);setRoute("playlists");notify("已更新智能歌单");}else notify("已保存智能歌单，可在播放列表中播放");}}
            />
        )}
        {(route === "albums" || route === "artists") && (
          <CollectionBrowser
            key={`${route}:${source}`}
            api={api}
            kind={route}
            selectUnassigned={()=>{setLibraryAlbumStatus("unassigned");setQuery("");setAlbumId("");setArtistId("");setRoute("library");}}
            source={source}
            revision={revision}
            onError={onError}
            select={(id) => {
              catalogOrigin.current=route;
              setQuery("");
              setAlbumId(route === "albums" ? id : "");
              setArtistId(route === "artists" ? id : "");
              setRoute("library");
            }}
          />
        )}
        {(route === "playlists"||route==="favorites") && (
          <PlaylistsView key={route} favoritesOnly={route==="favorites"} backToPlaylists={()=>setRoute("playlists")}
            api={api}
            zone={zone}
            refreshZones={refreshZones}
            onError={onError}
            notify={notify}
            plugins={plugins}
            play={play}
            enqueue={enqueue}
            goSettings={() => setRoute("settings")}
            editSmart={item=>{const rule=item.smart_query!;setPlaylistEdit({api,playlist:item});setQuery(rule.q||"");setSource(rule.source||"");setAlbumId(rule.album_id||"");setArtistId(rule.artist_id||"");setRoute("library");}}
          />
        )}
        {route === "profile" && <ProfilePage user={user.username} online={online} device={deviceName}
          navigate={setRoute} offline={openOffline} logout={()=>void logout()}/>}
        {route === "core-connection" && <section className="mobile-core-page">
          <button type="button" className="text-button mobile-core-back" onClick={()=>setRoute("profile")}>‹ 返回我的</button>
          <h1>Core 连接</h1>
          <ConnectionQuality api={api}/>
          {isNative&&<CoreConnections api={api} profiles={profiles} onConnect={connect}/>}
        </section>}
        {route === "settings" && (
          <SettingsView
            onUserChange={setUser}
            logout={logout}
            api={api}
            preferences={preferences}
            savePreferences={savePreferences}
            plugins={plugins}
            user={user}
            notify={notify}
            onError={onError}
            refreshLibrary={refreshLibrary}
            profiles={profiles}
            connect={connect}
            zone={zone} endpoint={endpoints.find(e=>e.id===zone?.endpoint_id)} refreshZones={refreshZones}
            openZones={()=>setRoute("zones")}
          />
        )}
        {route === "dsp" && (() => {
          const held = new Set(endpoints.flatMap(endpoint => endpoint.group?.zone_ids ?? []));
          const candidates = zones.filter(candidate => !held.has(candidate.id));
          const target = candidates.find(candidate => candidate.id === (dspZoneId || selectedZone)) ?? candidates[0];
          const targetEndpoint = endpoints.find(endpoint => endpoint.id === target?.endpoint_id);
          return <div className="dsp-page">
            <header className="dsp-page-heading"><h1>DSP 工作区</h1>{target&&<label>应用到音区<select value={target.id} onChange={event=>setDspZoneId(event.target.value)}>{candidates.map(candidate=><option key={candidate.id} value={candidate.id}>{candidate.name}</option>)}</select></label>}</header>
            {target?<DspEditor key={target.id} api={api} zone={target} capabilities={targetEndpoint?.capabilities} advanced={coreCapabilities.dsp_workbench_v1===true} modular={coreCapabilities.dsp_modules_v1===true} onSaved={()=>void refreshZones()} onError={onError}/>:<Empty headline="请选择播放设备" description="设备连接后即可设置 DSP。"/>}
          </div>;
        })()}
        {route === "zones" && (
          <ZonesView compact={mobile} deviceGroups={coreCapabilities.device_groups_v1===true} familySync={user.role!=="external"} canPurge={user.role==="admin"&&coreCapabilities.device_purge_v1===true} page={zonesPage} onPageChange={setZonesPage} groupPreset={groupPreset}
            api={api}
            groupSyncPolicy={coreCapabilities.group_sync_policy_v1===true} groupStart={coreCapabilities.group_start_v1===true} groupList={coreCapabilities.group_start_list_v1===true} groupQueue={coreCapabilities.group_queue_v1===true} groupOutputTiming={coreCapabilities.group_output_v2===true&&coreCapabilities.measured_listens_v2===true&&coreCapabilities.measured_listens_online_v2===true&&coreCapabilities.measured_listens_socket_v2===true} groupAutomaticRate={coreCapabilities.automatic_group_rate_v1===true&&coreCapabilities.group_rate_adjustment_v1===true} groupTransport={coreCapabilities.group_transport_v1===true} groupResync={coreCapabilities.group_resync_v1===true} groupQueueJump={coreCapabilities.group_queue_jump_v1===true} groupMembership={coreCapabilities.group_membership_v1===true} groupClockDiagnostics={coreCapabilities.group_clock_diagnostics_v1===true} groupRecovery={coreCapabilities.group_manual_recovery_v2===true} outputGroups={coreCapabilities.output_groups_v1===true} coreId={coreId} userId={user.id}
            zones={zones}
            endpoints={endpoints}
            selected={selectedZone}
            localEndpointId={endpointId}
            onSelect={chooseZone}
            onDsp={openDsp}
            refresh={refreshZones}
            onError={onError}
          />
        )}
        {route === "library-home"&&<DailyPlaylist api={api} canPlay={!!zone} play={play} onError={onError} onOpen={picks=>openDaily(picks,"library-home")}/>}
        {route === "library-home"&&<LibraryHome api={api} revision={revision} preferences={preferences} savePreferences={savePreferences} play={play} onError={onError} browse={(kind,id)=>{catalogOrigin.current=route;setQuery("");setAlbumId(kind==="album"?id:"");setArtistId(kind==="artist"?id:"");setSource("");setRoute("library");}}/>}
        {route === "library-manage"&&<LibraryManagement external={user.role==="external"} api={api} revision={revision} play={play} enqueue={enqueue} onError={onError} showDetails={setDetailTrack} openOffline={openOffline} goSettings={()=>setRoute("settings")}/>}

        {route === "stats" && (
          <StatsView key={JSON.stringify([coreId,user.id])}
            plugins={plugins}
            api={api}
            revision={statsRevision}
            play={play}
            onError={onError}
          />
        )}
        </PageMemory>
        {route==="player"&&playerView}
      </main>
      {androidSheet.mounted&&<>
        <div className="android-player-scrim" data-player-phase={androidSheet.phase} aria-hidden="true"/>
        <div ref={androidSheet.layer} className="android-player-sheet" data-player-phase={androidSheet.phase}
          tabIndex={-1} role="region" aria-label="正在播放">
          {playerView}
        </div>
      </>}
      {queueOpen && (
        <aside className="queue-panel" aria-label="播放队列">
          {!legacyAndroid&&<div className="record-kicker queue-kicker"><i/>{selectedOutputLabel||"当前播放设备"}</div>}
          <div className="queue-heading">
            <h2>
              播放队列
              {legacyAndroid&&<span>{zone?.queue.length||0} 首</span>}
            </h2>
            <button
              className="text-button"
              disabled={!zone?.queue.length}
              onClick={() =>
                zone &&
                api
                  .delete(`/zones/${zone.id}/queue`)
                  .then(refreshZones)
                  .catch(onError)
              }
            >
              清空
            </button>
            <IconButton label="收起队列" onClick={closeQueue}><X size={18}/></IconButton>
          </div>
          {!legacyAndroid&&<p className="queue-summary">{zone?.queue.length||0} 首歌曲</p>}
          {mobile&&<label className="zone-select">
            <Monitor />
            <select
              value={selectedZone}
              onChange={(e) => chooseZone(e.target.value)}
              aria-label={legacyAndroid?"播放设备":"播放音区"}
            >
              {playbackZones.map((z) => (
                <option key={z.id} value={z.id}>
                  {outputLabel(z,endpoints.find(e=>e.id===z.endpoint_id),labels)}
                </option>
              ))}
            </select>
          </label>}
          {zone?.queue.length ? <PlaybackQueueTracks api={api} zone={zone} play={index=>void command("play",{queue_index:index})} updated={updated=>setZones(current=>current.map(item=>item.id===updated.id?updated:item))} onError={error=>{onError(error);void refreshZones();}}/> : <div className="empty-state">队列为空</div>}

          {zone && (
            <AutoplaySettings key={zone.id}
              api={api}
              zone={zone}
              refresh={refreshZones}
              onError={onError}
            />
          )}
          <button className="signal-link" onClick={()=>setSignalOpen(true)}><Link size={20}/>查看信号链<ChevronRight size={18}/></button>
        </aside>
      )}
      {route==="player"&&mobile?null:offlineActive?<footer className="offline-transport"><button onClick={openOffline}><Download size={20}/>离线聆听<ChevronRight size={18}/></button><button onClick={async()=>{try{await audio.interrupt();window.dispatchEvent(new Event("rmusic-offline-stop"));}catch(error){localError(error);setOfflineOpen(true);}}}>停止</button></footer>:<Transport
        inert={androidSheet.presented}
        mobile={mobile&&!legacyAndroid}
        outputLabel={selectedOutputLabel}
        endpoint={selectedEndpoint}
        api={api}
        zone={zone}
        position={displayedPosition}
        bufferedEnd={bufferedEnd}
        miniLyrics={portableNative&&preferences.mini_player_title!=="song"?{
          mode:preferences.mini_player_title==="original"?"original":"translation",
          language:preferences.lyric_language,
          follow:preferences.mini_player_word_follow!==false,
          offset:(preferences.lyric_offset||0)/1000,
          remote:!selectedLocal,
        }:undefined}
        playing={selectedPlaying}
        waiting={selectedWaiting}
        command={command}
        patchZone={patchZone}
        seek={seek}
        showPlayer={() => {
          if(mobile)androidSheet.open();
          else transitionPlayer(()=>setRoute("player"),mobile&&!preferences.low_performance);
          if(legacyAndroid)setQueueOpen(false);
        }}
        showQueue={() => queueOpen?closeQueue():rememberQueue(true)}
        notify={notify}
        onError={onError}
        busy={busy||groupTransport.busy}
        showZones={()=>setZonePicker(true)}
        showSignal={()=>setSignalOpen(value=>!value)}
        grade={grade}
        resolution={signalResolution(zone?.track, selectedSourceInfo, selectedPath, selectedLocal&&!isNative?audio.contextSampleRate:0)}
        showDetails={()=>zone?.track&&setDetailTrack(zone.track)}
      />}
      {zonePicker&&<ZonePicker compact={mobile} familySync={user.role!=="external"} labels={labels} api={api} zones={zones} endpoints={endpoints} selected={selectedZone} definitions={coreCapabilities.output_groups_v1===true} onSelect={id=>{chooseZone(id);setZonePicker(false);}} onGroup={ids=>{setGroupPreset({ids,key:Date.now()});setZonesPage("groups");setRoute("zones");setZonePicker(false);}} onManage={()=>{setZonesPage("groups");setRoute("zones");setZonePicker(false);}} close={()=>setZonePicker(false)}/>}
      {signalOpen&&<SignalPanel zone={zone} endpoint={endpoints.find(e=>e.id===zone?.endpoint_id)} info={selectedSourceInfo} error={selectedSourceError} adaptive={selectedLocal?audio.adaptive:0} nativeState={selectedLocal?audio.nativeState:{}} localOutput={selectedLocal} contextRate={selectedLocal?audio.contextSampleRate:0} close={()=>setSignalOpen(false)} onDsp={zone?()=>{setSignalOpen(false);openDsp(zone.id);}:undefined}/>}
      {detailTrack&&<TrackDetails api={api} track={detailTrack} play={play} playExact={track=>play(track,undefined,true)} enqueue={enqueue} close={()=>setDetailTrack(null)}/>}
      {(toast||groupTransport.pending) && (
        <div className="toast" role="status">
          <span>{groupTransport.pending?(groupTransport.pending.error||`${groupTransport.pending.title}：正在确认整组操作…`):toast}</span>
          {groupTransport.pending&&!groupTransport.pending.confirmed&&!groupTransport.pending.rejected&&<button onClick={groupTransport.retry} disabled={groupTransport.busy}>重试整组操作</button>}
          {groupTransport.pending?.rejected&&<button onClick={groupTransport.dismiss}>返回当前状态</button>}
          {groupTransport.pending&&<button onClick={()=>{setZonesPage("groups");setRoute("zones");}}>查看分组</button>}
          {!groupTransport.pending&&<IconButton label="关闭提示" onClick={() => setToast("")}>
            <X size={17} />
          </IconButton>}
        </div>
      )}
    </div>{offlineDialog}</TrackContextMenuProvider></SongCollectionProvider></PersonalRecommendations></CatalogNavigation></ExternalAccountContext.Provider>
  );
}

