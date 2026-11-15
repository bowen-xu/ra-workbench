import { INITIAL_UPDATE_STATUS, type UpdateGateway, type UpdateStatus } from "../services/updateProtocol";
import type { AppRuntime } from "./appRuntime";

/** Mock platform I/O only; sidebar controls and state transitions are production UI. */
export function createPreviewUpdater(preview: AppRuntime["preview"]): UpdateGateway {
  let status = INITIAL_UPDATE_STATUS;
  let installed = false;
  const listeners = new Set<(value: UpdateStatus) => void>();
  let resume: (() => void) | undefined;
  const publish = (change: Partial<UpdateStatus>): UpdateStatus => {
    status = { ...status, ...change, revision: status.revision + 1 };
    listeners.forEach(listener => listener(status));
    return status;
  };
  return {
    status: async () => status,
    subscribe: async listener => { listeners.add(listener); return () => listeners.delete(listener); },
    check: async () => {
      publish({ phase: "checking", message: "正在模拟检查更新…" });
      await preview?.beforeOperation("platform");
      return publish(!installed
        ? { phase: "available", version: "0.0.0-preview.1", message: "下载模拟更新 0.0.0-preview.1" }
        : { phase: "upToDate", message: "当前已是最新版本。（浏览器模拟检查）" });
    },
    download: async () => {
      publish({ phase: "downloading", downloaded: 0, total: 100, message: "正在模拟下载，点击暂停" });
      while (status.downloaded < 100) {
        await new Promise(resolve => setTimeout(resolve, 50));
        if (status.phase === "paused") await new Promise<void>(resolve => { resume = resolve; });
        publish({ downloaded: status.downloaded + 1 });
      }
      return publish({ phase: "downloaded", readyToInstall: true, message: "模拟下载完成，准备安装" });
    },
    pause: async paused => {
      const next = publish({ phase: paused ? "paused" : "downloading", message: paused ? "模拟下载已暂停，点击继续" : "正在模拟下载，点击暂停" });
      if (!paused) { resume?.(); resume = undefined; }
      return next;
    },
    install: async () => {
      publish({ phase: "installing", message: "正在模拟安装…" });
      await new Promise(resolve => setTimeout(resolve, 500));
      installed = true;
      return publish({ phase: "upToDate", readyToInstall: false, version: null, message: "模拟更新完成，当前已是最新版本。" });
    },
  };
}
