import {CatalogLink} from "./CatalogLinks";
import {useEffect,useState} from "react";
import {Download,RefreshCw,Trash2} from "lucide-react";
import type {RMusicClient} from "@r-music/sdk";
import {formatBytes} from "./browserCache";
import {Empty,IconButton} from "./components";
import {isNative} from "./native";
import {saveApiFile} from "./fileDownload";
interface DownloadTask {
  id: string;
  track_id: string;
  status: string;
  size: number;
  error?: string;
  title?: string;
  artist?: string;
  format?: string;
}
export function DownloadsView({api,onError,notify,openOffline}:{api:RMusicClient;onError:(error:unknown)=>void;notify:(message:string)=>void;openOffline:()=>void}){
  const [items,setItems]=useState<DownloadTask[]>([]);
  const refresh=()=>api.get<{items:DownloadTask[]}>("/downloads").then(result=>setItems(result.items)).catch(onError);
  useEffect(()=>{let active=true;const read=()=>api.get<{items:DownloadTask[]}>("/downloads").then(result=>{if(active)setItems(result.items);}).catch(error=>{if(active)onError(error);});read();const timer=setInterval(read,4000);return()=>{active=false;clearInterval(timer);};},[api,onError]);
  async function save(item:DownloadTask){try{await saveApiFile(api,`/downloads/${item.id}/file`,item.title||item.track_id);notify("已保存到设备下载目录");}catch(error){onError(error);}}
  return <div className="downloads-view"><button className="primary" onClick={openOffline}><Download size={18}/>本机离线音乐</button>
      <section className="settings-section">
        <div className="section-heading">
          <h2>手动下载</h2>
          <IconButton label="刷新下载" onClick={refresh}>
            <RefreshCw size={18} />
          </IconButton>
        </div>
        <p className="muted">主动下载完整歌曲，独立于自动缓存淘汰。</p>
        {items.length ? (
          <div className="download-list">
            {items.map((item) => (
              <div key={item.id}>
                <Download size={20} />
                <div>
                  <strong title={item.title || item.track_id}>
                    {item.title || item.track_id}
                  </strong>
                  <span>
                    {item.artist && <><CatalogLink track={{id:item.track_id,artist:item.artist,album:""}}/> · </>}
                    {item.format?.toUpperCase()} {item.status} ·{" "}
                    {formatBytes(item.size)}
                    {item.error ? ` · ${item.error}` : ""}
                  </span>
                </div>
                <button
                  disabled={
                    isNative ||
                    !["complete", "completed", "ready"].includes(item.status)
                  }
                  title={
                    isNative
                      ? "核心成品请在 WebUI 保存；本机下载在下方离线音乐中管理"
                      : undefined
                  }
                  onClick={() => save(item)}
                >
                  保存到设备
                </button>
                <IconButton
                  label="移除下载"
                  onClick={() =>
                    api
                      .delete(`/downloads/${item.id}`)
                      .then(refresh)
                      .catch(onError)
                  }
                >
                  <Trash2 size={17} />
                </IconButton>
              </div>
            ))}
          </div>
        ) : (
          <Empty
            icon={<Download />}
            headline="把音乐带在身边"
            description="在曲目旁点击下载，完整文件会保存在核心。"
          />
        )}
      </section>
  </div>;
}
