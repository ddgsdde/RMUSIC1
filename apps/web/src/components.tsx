import {createPortal} from "react-dom";
import {
  useEffect,
  useRef,
  useState,
  type ButtonHTMLAttributes,
  type ReactNode,
} from "react";
import { Disc3, LoaderCircle, X } from "lucide-react";
import type { RMusicClient, Track } from "@r-music/sdk";
import {acquireArtwork,type ArtworkLease} from "./artworkCache";
export function IconButton({
  label,
  children,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  label: string;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      className="icon-button"
      title={label}
      aria-label={label}
      {...props}
    >
      {children}
    </button>
  );
}
export function Cover({
  track,
  api,
  className = "",
  onImageLoad,
  onImageUnavailable,
}: {
  track?: Pick<Track, "cover_url" | "album" | "title"> | null;
  api: RMusicClient;
  className?: string;
  onImageLoad?:(image:HTMLImageElement)=>void;
  onImageUnavailable?:()=>void;
}) {
  type HeldArtwork={lease:ArtworkLease;controller:AbortController};
  const [picture,setPicture]=useState<{src:string;previous:string;alt:string;api:RMusicClient;token:string}|null>(null);
  const [repair,setRepair]=useState(0);
  const host=useRef<HTMLDivElement>(null),current=useRef<HeldArtwork|null>(null),retiring=useRef<HeldArtwork|null>(null),fade=useRef<ReturnType<typeof setTimeout>|undefined>(undefined);
  const release=(item:HeldArtwork|null)=>{item?.controller.abort();item?.lease.release();};
  useEffect(()=>()=>{clearTimeout(fade.current);release(current.current);release(retiring.current);current.current=null;retiring.current=null;},[api,api.token]);
  useEffect(()=>{setRepair(0);},[api,track?.cover_url]);
  useEffect(()=>{
    const controller=new AbortController();let observer:IntersectionObserver|undefined,started=false,stopped=false,accepted=false,acquired:ArtworkLease|undefined,retryTimer:ReturnType<typeof setTimeout>|undefined;
    const show=(value:ArtworkLease|null)=>{
      clearTimeout(fade.current);release(retiring.current);
      retiring.current=current.current;current.current=value?{lease:value,controller}:null;accepted=!!value;
      const src=value?.src||"",previous=retiring.current?.lease.src||"";
      setPicture({src,previous:previous===src?"":previous,alt:track?.album||track?.title||"专辑封面",api,token:api.token});
      if(!value)onImageUnavailable?.();
      fade.current=setTimeout(()=>{release(retiring.current);retiring.current=null;setPicture(frame=>frame?.src===src?{...frame,previous:""}:frame);},350);
    };
    const load=()=>{
      if(started||!track?.cover_url)return;started=true;
      void (async()=>{
        try{
          const value=await acquireArtwork(api,track.cover_url!,controller.signal,repair===1);acquired=value;
          const decoded=new Image();decoded.src=value.src;await decoded.decode();
          if(stopped){controller.abort();value.release();return;}
          show(value);
        }catch{
          if(stopped)return;
          acquired?.invalidate();controller.abort();show(null);
          if(repair===0)retryTimer=setTimeout(()=>{if(!stopped)setRepair(1);},16000);
        }
      })();
    };
    if(!track?.cover_url)show(null);
    else if(typeof IntersectionObserver==='undefined')load();
    else if(host.current){observer=new IntersectionObserver(rows=>{if(rows.some(row=>row.isIntersecting)){observer?.disconnect();load();}},{rootMargin:'160px'});observer.observe(host.current);}
    return()=>{stopped=true;clearTimeout(retryTimer);observer?.disconnect();if(!accepted)controller.abort();};
  },[track?.cover_url,api,api.token,api.artworkRevision,repair]);
  const frame=picture?.api===api&&picture.token===api.token?picture:null;
  return (
    <div ref={host} className={`cover ${className}`} title={!frame?.src ? (track?.cover_url ? "封面暂时无法加载" : "尚未取得封面资料，可刷新重试") : undefined}>
      {frame?.previous&&<img className={`cover-previous ${frame.src?"":"cover-fade-out"}`} src={frame.previous} alt="" aria-hidden="true" decoding="async"/>}
      {frame?.src?<img key={frame.src} className="cover-current" src={frame.src} alt={frame.alt} onLoad={event=>onImageLoad?.(event.currentTarget)} decoding="async" onError={()=>{current.current?.lease.invalidate();release(current.current);current.current=null;setPicture(null);onImageUnavailable?.();if(repair===0)setRepair(1);}}/>:<Disc3 aria-hidden="true"/>}
    </div>
  );
}

export function Modal({
  title,
  children,
  onClose,
  wide = false,
  active = true,
  portal = false,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
  wide?: boolean;
  active?: boolean;
  portal?: boolean;
}) {
  const dialog = useRef<HTMLElement>(null);
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    if (!active) return;
    const element = dialog.current!;
    const previous = document.activeElement as HTMLElement | null;
    const overflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    const topmost = () => [...document.querySelectorAll('[role="dialog"][aria-modal="true"]')].filter(item=>!item.closest('[hidden]')).at(-1) === element;
    const focusable = () => [...element.querySelectorAll<HTMLElement>('button, input, select, textarea, a[href], summary, [tabindex]')].filter(item =>
      item.tabIndex >= 0 && !item.matches(':disabled') && !item.closest('[hidden], [inert]') && getComputedStyle(item).display !== "none" && getComputedStyle(item).visibility !== "hidden" &&
      (!item.closest('details:not([open])') || item.closest('details:not([open])')?.querySelector('summary')?.contains(item)),
    );
    element.focus();
    const keydown = (event: KeyboardEvent) => {
      if (!topmost()) return;
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        close.current();
      } else if (event.key === "Tab") {
        const items = focusable(), first = items[0], last = items.at(-1);
        if (!first) { event.preventDefault(); element.focus(); }
        else if (event.shiftKey && (document.activeElement === first || document.activeElement === element)) { event.preventDefault(); last!.focus(); }
        else if (!event.shiftKey && (document.activeElement === last || !element.contains(document.activeElement))) { event.preventDefault(); first.focus(); }
      }
    };
    const focusin = (event: FocusEvent) => {
      if (topmost() && !element.contains(event.target as Node)) element.focus();
    };
    document.addEventListener("keydown", keydown);
    document.addEventListener("focusin", focusin);
    return () => {
      document.removeEventListener("keydown", keydown);
      document.removeEventListener("focusin", focusin);
      document.body.style.overflow = overflow;
      if (previous?.isConnected) previous.focus();
    };
  }, [active]);
  const overlay = (
    <div
      className="modal-backdrop"
      role="presentation"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <section
        ref={dialog}
        tabIndex={-1}
        className={`modal ${wide ? "wide" : ""}`}
        role="dialog"
        aria-modal={active ? true : undefined}
        aria-label={title}
      >
        <header>
          <h2>{title}</h2>
          <IconButton label="关闭" onClick={onClose}>
            <X />
          </IconButton>
        </header>
        {children}
      </section>
    </div>
  );
  return portal ? createPortal(overlay, document.body) : overlay;
}
export function Empty({
  icon,
  headline,
  description,
  action,
}: {
  icon?: ReactNode;
  headline: string;
  description?: string;
  action?: ReactNode;
}) {
  return (
    <div className="empty">
      {icon || <Disc3 />}
      <h3>{headline}</h3>
      {description && <p>{description}</p>}
      {action}
    </div>
  );
}
export function Busy() {
  return (
    <div className="busy">
      <LoaderCircle className="spin" />
      正在载入
    </div>
  );
}
export function quality(track?: Track | null) {
  if (!track) return "等待播放";
  const observed=track.provider_data?.source_audio as {verified?:unknown;format?:unknown;sample_rate?:unknown;bit_depth?:unknown}|undefined;
  if(track.source!=="local"&&observed?.verified===true&&typeof observed.format==="string") {
    return [observed.format.toUpperCase(),typeof observed.bit_depth==="number"&&observed.bit_depth>0?`${observed.bit_depth} bit`:null,typeof observed.sample_rate==="number"&&observed.sample_rate>0?`${observed.sample_rate/1000} kHz`:null].filter(Boolean).join(" · ");
  }
  const hint=track.provider_data?.quality_hint as {label?:unknown}|undefined;
  if(!track.sample_rate&&(!track.format||track.format==="unknown")&&typeof hint?.label==="string")return `${hint.label} · 平台声明`;
  if(!track.sample_rate&&(!track.format||track.format==="unknown")&&track.source!=="local")return "在线音源";
  const bits = track.bit_depth ? `${track.bit_depth} bit · ` : "";
  const rate = track.sample_rate ? `${track.sample_rate / 1000} kHz` : "";
  const format=track.format&&track.format!=="unknown"?track.format.toUpperCase():"格式未报告";
  return `${format} ${bits}${rate}${!bits&&!rate&&track.bitrate?` · ${Math.round(track.bitrate/1000)} kbps`:""}`.trim();
}
