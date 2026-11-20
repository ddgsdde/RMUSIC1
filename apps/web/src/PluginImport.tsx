import { useRef, useState } from "react";
import { Upload } from "lucide-react";
import type { RMusicClient } from "@r-music/sdk";
import { isNative } from "./native";
import { Modal } from "./components";
export function PluginImport({
  api,
  notify,
  onError,
}: {
  api: RMusicClient;
  notify: (message: string) => void;
  onError: (error: unknown) => void;
}) {
  const [open, setOpen] = useState(false);
  const [trusted, setTrusted] = useState(false);
  const [dependencies, setDependencies] = useState(false);
  const [busy, setBusy] = useState(false);
  const file = useRef<HTMLInputElement>(null);
  return (
    <>
      <button
        disabled={isNative}
        title={
          isNative
            ? "原生 ZIP 上传接口尚未开放，请在 WebUI 导入"
            : "导入或更新插件"
        }
        onClick={() => setOpen(true)}
      >
        <Upload size={16} />
        导入 / 更新插件
      </button>
      {open && (
        <Modal
          title="导入插件"
          onClose={() => {
            if (!busy) setOpen(false);
          }}
        >
          <p>
            插件是可执行的 Python 代码。请只安装信任来源的插件
            ZIP，核心会先检查兼容性与健康状态，成功后切换，保留旧版本以供回滚。
          </p>
          <label className="setting-row">
            <span>我信任这个插件的代码来源</span>
            <input
              type="checkbox"
              checked={trusted}
              onChange={(e) => setTrusted(e.target.checked)}
            />
          </label>
          <label className="setting-row">
            <span>
              <strong>同时安装插件依赖</strong>
              <small>pip 安装可能执行依赖包代码；默认关闭。</small>
            </span>
            <input
              type="checkbox"
              checked={dependencies}
              onChange={(e) => setDependencies(e.target.checked)}
            />
          </label>
          <input
            ref={file}
            type="file"
            accept=".zip,application/zip"
            disabled={!trusted || busy}
            onChange={async (e) => {
              const selected = e.target.files?.[0];
              if (!selected) return;
              setBusy(true);
              try {
                if (selected.size > 16 * 1024 ** 2)
                  throw new Error("插件 ZIP 不能超过 16 MiB");
                await api.request(
                  `/plugins/install?install_dependencies=${dependencies}`,
                  {
                    method: "POST",
                    body: selected,
                    headers: { "Content-Type": "application/zip" },
                  },
                );
                notify("插件已通过检查并启用");
                setOpen(false);
                setTrusted(false);
              } catch (error) {
                onError(error);
              } finally {
                setBusy(false);
                if (file.current) file.current.value = "";
              }
            }}
          />
          {busy && <p className="fine-print">正在安装并验证插件，请勿关闭。</p>}
        </Modal>
      )}
    </>
  );
}
