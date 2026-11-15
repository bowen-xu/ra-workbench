import { useEffect, useState, useSyncExternalStore } from "react";
import { Download, Pause, Play, RotateCw } from "lucide-react";
import { getReaderCloseCoordinator } from "../workspace/DocumentCloseCoordinator";
import { notify } from "../services/notifications";
import { createUpdateGateway, type UpdateGateway } from "../services/updateService";
import { UpdateController } from "../services/UpdateController";
import { ControlHint } from "./ControlHint";
import "./AppUpdateControl.css";

export function AppUpdateControl({ gateway }: { readonly gateway?: UpdateGateway }): React.JSX.Element {
  const [controller] = useState(() => new UpdateController(gateway ?? createUpdateGateway(),
    () => getReaderCloseCoordinator().prepareAll(), (message, error) => { if (error) notify.error(message); else notify.info(message); }));
  const status = useSyncExternalStore(controller.subscribe, controller.snapshot);
  useEffect(() => {
    let disposed = false, cleanup: (() => void) | undefined;
    void controller.connect().then(stop => { if (disposed) stop(); else cleanup = stop; })
      .catch((error: unknown) => notify.error(`无法连接更新器（update_listener_failed）：${String(error)}`));
    return () => { disposed = true; cleanup?.(); };
  }, [controller]);
  const transferring = status.phase === "downloading" || status.phase === "paused";
  const busy = status.phase === "checking" || status.phase === "installing" || status.phase === "downloaded";
  const downloadable = status.phase === "available" || (status.phase === "failed" && status.version && !status.readyToInstall);
  const label = transferring ? (status.phase === "paused" ? "继续下载" : "暂停下载")
    : busy ? (status.phase === "checking" ? "正在检查更新" : "正在安装更新")
      : status.phase === "failed" && status.readyToInstall ? "重试安装" : downloadable ? "下载更新" : "检查更新";
  const progress = status.total && status.total > 0 ? Math.min(100, status.downloaded / status.total * 100) : undefined;
  const hint = `${label}${status.version ? ` ${status.version}` : ""}${transferring ? ` · ${progress === undefined ? "已下载 " + Math.round(status.downloaded / 1024) + " KB" : Math.floor(progress) + "%"}` : ""}${status.phase === "failed" ? `：${status.message}` : ""}`;
  return <ControlHint content={hint} wrap={busy}>
    <button aria-label={label} className="sidebar-utility-button app-update-control" disabled={busy}
      onClick={() => void controller.activate()} type="button">
      {transferring ? <>
        <svg aria-hidden="true" className={`update-progress${progress === undefined && status.phase !== "paused" ? " indeterminate" : ""}`} viewBox="0 0 30 30">
          <circle className="update-progress-track" cx="15" cy="15" r="13" />
          <circle className="update-progress-value" cx="15" cy="15" r="13" pathLength="100" strokeDasharray={`${progress ?? 20} 100`} />
        </svg>
        {status.phase === "paused" ? <Play aria-hidden size={12} /> : <Pause aria-hidden size={12} />}
        <span className="update-progress-accessible" role="progressbar" aria-label="更新下载进度" aria-valuemin={0} aria-valuemax={100} aria-valuenow={progress === undefined ? undefined : Math.floor(progress)} />
      </> : downloadable ? <Download aria-hidden size={15} strokeWidth={1.8} />
        : <RotateCw aria-hidden className={busy ? "update-icon spinning" : "update-icon"} size={15} strokeWidth={1.8} />}
    </button>
  </ControlHint>;
}
