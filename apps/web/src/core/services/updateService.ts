import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getAppRuntime } from "../runtime/appRuntime";
import { createPreviewUpdater } from "../runtime/previewUpdater";
import { INITIAL_UPDATE_STATUS, type UpdateGateway, type UpdateStatus } from "./updateProtocol";
export { INITIAL_UPDATE_STATUS, type UpdateGateway, type UpdateStatus } from "./updateProtocol";

export function createUpdateGateway(): UpdateGateway {
  const runtime = getAppRuntime();
  if (runtime.mode === "preview") return createPreviewUpdater(runtime.preview);
  if (runtime.mode === "desktop") return {
    status: () => invoke("application_update_status"),
    subscribe: listener => listen<UpdateStatus>("application-update-status", event => listener(event.payload)),
    check: () => invoke("application_update_check"),
    download: () => invoke("application_update_download"),
    pause: paused => invoke("application_update_pause", { paused }),
    install: () => invoke("application_update_install"),
  };
  let revision = 0;
  return {
    status: async () => INITIAL_UPDATE_STATUS,
    subscribe: async () => () => undefined,
    check: async () => ({ ...INITIAL_UPDATE_STATUS, revision: ++revision, phase: "browserOnly",
      message: "浏览器不执行桌面更新，请在桌面应用中检查。" }),
    download: async () => INITIAL_UPDATE_STATUS,
    pause: async () => INITIAL_UPDATE_STATUS,
    install: async () => INITIAL_UPDATE_STATUS,
  };
}
