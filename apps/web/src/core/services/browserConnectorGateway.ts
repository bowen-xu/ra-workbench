import { listen } from "@tauri-apps/api/event";

import { invoke, isDesktopRuntime } from "../runtime/platformCore";

export interface BrowserConnectorStatus {
  readonly endpoint: string;
  readonly available: boolean;
  readonly message: string;
}

export interface BrowserConnectorGateway {
  getStatus(): Promise<BrowserConnectorStatus>;
}

export interface BrowserCaptureEvent {
  readonly kind: "item" | "attachment" | "tags";
}

function decodeStatus(value: unknown): BrowserConnectorStatus {
  if (typeof value !== "object" || value === null
    || !("endpoint" in value) || typeof value.endpoint !== "string"
    || !("available" in value) || typeof value.available !== "boolean"
    || !("message" in value) || typeof value.message !== "string") {
    throw new Error("无法读取 Chrome 插件连接状态，请刷新重试。");
  }
  return { endpoint: value.endpoint, available: value.available, message: value.message };
}

const unavailableGateway: BrowserConnectorGateway = {
  async getStatus() {
    return { endpoint: "", available: false, message: "请在桌面应用中连接 Chrome 插件。" };
  },
};

const desktopGateway: BrowserConnectorGateway = {
  async getStatus() {
    return decodeStatus(await invoke<unknown>("get_browser_connector_status"));
  },
};

export function getBrowserConnectorGateway(): BrowserConnectorGateway {
  return isDesktopRuntime() ? desktopGateway : unavailableGateway;
}

/** The caller owns unsubscription, including a subscription that resolves late. */
export async function listenBrowserCaptures(onCapture: (event: BrowserCaptureEvent) => void): Promise<() => void> {
  if (!isDesktopRuntime()) return () => undefined;
  return listen<unknown>("research-assistant:browser-capture", ({ payload }) => {
    if (typeof payload !== "object" || payload === null || !("kind" in payload)) return;
    if (payload.kind === "item" || payload.kind === "attachment" || payload.kind === "tags") {
      onCapture({ kind: payload.kind });
    }
  });
}
