import { resolveAppGateway } from "../runtime/appRuntime";
import { invoke, isDesktopRuntime } from "../runtime/platformCore";
import { runDoclingParse } from "./doclingParseTask";
import { doclingErrorMessage } from "./doclingError";

import type { DoclingAttachmentMetadataPort, DoclingGateway, DoclingSettings } from "../contracts/docling";
export type {
  DoclingParseTaskStatus, DoclingSettings, DoclingSettingsDraft, DoclingServiceStatus,
  DoclingPingResult, DoclingParseRunSummary, DoclingParseRunDetail, DoclingOverlayRect,
  DoclingOverlayRegion, DoclingOverlayItem, DoclingPageOverlay, DoclingGateway,
  DoclingModelStatus, DoclingModelStorage,
} from "../contracts/docling";

const DEFAULT_SETTINGS: DoclingSettings = {
  enabled: false,
  modelManifestUrl: "http://127.0.0.1:4174/models/docling/latest.json",
  defaultPort: 5001,
  fallbackStartPort: 5010,
  fallbackEndPort: 5099,
  idleShutdownMs: 1_200_000,
  requestTimeoutMs: 300_000,
  updatedAt: "",
};

const unavailableMetadata: DoclingAttachmentMetadataPort = {
  async describe() {
    throw Object.assign(new Error("Docling 来源元数据服务未接入，请先启用对应资料来源。"), {
      code: "DOCLING_SOURCE_PROVIDER_UNAVAILABLE",
    });
  },
};

function desktopOnly(): Error {
  return new Error("Docling 结构化 PDF 解析只在 Tauri 桌面模式中可用");
}

const browserGateway: DoclingGateway = {
  async startParseTask() { throw desktopOnly(); },
  async getParseTask() { throw desktopOnly(); },
  async cancelParseTask() { throw desktopOnly(); },
  async getModelStorage() { return { totalBytes: 0, downloading: false }; },
  async deleteModels() { throw desktopOnly(); },
  async getModelStatus() { return { state: "browser_unavailable", downloadedBytes: 0, totalBytes: 0 }; },
  async checkModelUpdate() { throw desktopOnly(); },
  async downloadModels() { throw desktopOnly(); },
  async getSettings() {
    return DEFAULT_SETTINGS;
  },
  async saveSettings() {
    throw desktopOnly();
  },
  async getStatus() {
    return { state: "browser_unavailable", lastError: "浏览器预览无法管理本地 Docling 服务" };
  },
  async pingService() {
    throw desktopOnly();
  },
  async connectService() {
    throw desktopOnly();
  },
  async disconnectService() {
    throw desktopOnly();
  },
  async parseAttachment() {
    throw desktopOnly();
  },
  async listParseRuns() {
    return [];
  },
  async getParseRun() {
    throw desktopOnly();
  },
  async getPageOverlay(attachmentId, pageIndex) {
    return {
      attachmentId,
      parseRunId: "browser-unavailable",
      pageIndex,
      items: [],
    };
  },
};

async function invokeDocling<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (reason) {
    throw new Error(doclingErrorMessage(reason), { cause: reason });
  }
}

const desktopGateway: DoclingGateway = {
  startParseTask: (itemId, attachmentId, options) => invokeDocling("start_docling_parse_task", { itemId, attachmentId, force: options?.force ?? false }),
  getParseTask: taskId => invokeDocling("get_docling_parse_task", { taskId }),
  cancelParseTask: taskId => invokeDocling("cancel_docling_parse_task", { taskId }),
  getModelStorage: () => invokeDocling("get_docling_model_storage"),
  deleteModels: () => invokeDocling("delete_docling_models"),
  getModelStatus: () => invokeDocling("get_docling_model_status"),
  checkModelUpdate: () => invokeDocling("check_docling_model_update"),
  downloadModels: () => invokeDocling("download_docling_models"),
  getSettings: () => invokeDocling("get_docling_settings"),
  saveSettings: (draft) => invokeDocling("save_docling_settings", { draft }),
  getStatus: () => invokeDocling("get_docling_service_status"),
  pingService: () => invokeDocling("ping_docling_service"),
  connectService: () => invokeDocling("connect_docling_service"),
  disconnectService: () => invokeDocling("disconnect_docling_service"),
  parseAttachment: async (itemId, attachmentId, options) => {
    const metadata = await resolveAppGateway("doclingAttachmentMetadata", unavailableMetadata).describe(itemId, attachmentId);
    const result = await runDoclingParse(desktopGateway, itemId, attachmentId, metadata, options?.force ?? false);
    if (!result) throw Object.assign(new Error("Docling 解析已终止"), { code: "cancelled" });
    return result;
  },
  listParseRuns: (itemId) => invokeDocling("list_docling_parse_runs", { itemId }),
  getParseRun: (runId) => invokeDocling("get_docling_parse_run", { runId }),
  getPageOverlay: (attachmentId, pageIndex, parseRunId) => invokeDocling("get_docling_page_overlay", { attachmentId, pageIndex, parseRunId }),
};

export function getDoclingGateway(): DoclingGateway {
  return resolveAppGateway("getDoclingGateway", isDesktopRuntime() ? desktopGateway : browserGateway);
}
