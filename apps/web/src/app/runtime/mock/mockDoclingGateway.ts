import type {
  DoclingGateway,
  DoclingParseTaskStatus,
  DoclingSettings,
  DoclingServiceStatus,
  DoclingParseRunDetail,
} from "../../../core/services/doclingGateway";
import { runDoclingParse } from "../../../core/services/doclingParseTask";
import { getLiteratureGateway } from "@research-assistant/literature-library-plugin/literatureGateway";
import { currentMockDoclingRun, mockDoclingIdentity } from "./mockParsedPaperSources";
import { isVisionByLogicAttachment, visionByLogicEvidence } from "./visionByLogicFixture";
import type { PreviewControls } from "../../../core/runtime/appRuntime";

export function createMockDoclingGateway(storage: Storage, controls?: Pick<PreviewControls, "beforeOperation">): DoclingGateway {
  let settings: DoclingSettings = (JSON.parse(
    storage.getItem("docling-settings") ?? "null",
  ) as DoclingSettings | null) ?? {
    enabled: true,
    modelManifestUrl: "http://127.0.0.1:4174/models/docling/latest.json",
    defaultPort: 5001,
    fallbackStartPort: 5002,
    fallbackEndPort: 5010,
    idleShutdownMs: 60000,
    requestTimeoutMs: 60000,
    updatedAt: new Date().toISOString(),
  };
  settings = {
    ...settings,
    modelManifestUrl:
      settings.modelManifestUrl ??
      "http://127.0.0.1:4174/models/docling/latest.json",
  };
  let status: DoclingServiceStatus = { state: "disconnected" };
  const runs = new Map<string, DoclingParseRunDetail>(
    JSON.parse(storage.getItem("docling-runs") ?? "[]") as [
      string,
      DoclingParseRunDetail,
    ][],
  );
  const installed = () => storage.getItem("docling-model-installed") !== "false";
  const modelStatus = () => ({ state: installed() ? "ready" : "not_installed",
    ...(installed() ? { installedVersion: "preview-fp32", installedIdentity: mockDoclingIdentity(storage) } : {}), downloadedBytes: 0, totalBytes: 0 });
  const modelStorage = () => ({ totalBytes: installed() ? 430299653 : 0,
    ...(installed() ? { installedVersion: "preview-fp32" } : {}), downloading: false });
  const tasks = new Map<string, { status: DoclingParseTaskStatus; itemId: string; attachmentId: string; started: number; saving?: Promise<DoclingParseRunDetail> }>();
  const flights = new Map<string, Promise<DoclingParseRunDetail>>();
  const convertMockPdf = async (itemId: string, attachmentId: string): Promise<DoclingParseRunDetail> => {
      await controls?.beforeOperation("docling");
      const paper = await getLiteratureGateway().get(itemId);
      const attachment = paper.attachments.find((item) => item.id === attachmentId);
      if (!attachment) throw new Error("模拟附件不存在");
      const timestamp = new Date().toISOString();
      const value: DoclingParseRunDetail = {
        id: crypto.randomUUID(),
        itemId,
        attachmentId,
        contentSha256: attachment.contentSha256,
        serviceBaseUrl: "mock://docling",
        doclingStatus: "success",
        parseStatus: "success",
        createdAt: timestamp,
        finishedAt: timestamp,
        processingTimeSeconds: 0.1,
        jsonContent: {
          texts: (isVisionByLogicAttachment(attachment) ? visionByLogicEvidence(paper, attachment) : [{quoteExact: paper.title, pageIndex: 0}])
            .map((source,index) => ({self_ref: `#/texts/${index}`, text: source.quoteExact, prov: [{page_no: source.pageIndex + 1}]})),
        },
        rawResponse: { status: "success", mock: true, runtime: {identity: mockDoclingIdentity(storage)} },
      };
      runs.set(value.id, value);
      storage.setItem("docling-runs", JSON.stringify([...runs]));
      return value;
  };
  const gateway: DoclingGateway = {
    startParseTask: async (itemId, attachmentId, options) => {
      if (!installed()) throw {code: "models_unavailable", message: "尚未安装模型，请在设置中下载 FP32 模型"};
      if (!settings.enabled) throw {code: "disabled", message: "Docling 服务未启用"};
      const file = (await getLiteratureGateway().get(itemId)).attachments.find(value => value.id === attachmentId);
      if (!file) throw new Error("模拟附件不存在");
      const saved = !options?.force ? currentMockDoclingRun(storage, itemId, attachmentId, file.contentSha256) : undefined;
      if (saved) return {id: crypto.randomUUID(), state: "succeeded", stage: "saving", parseRunId: saved.id, completedPages: 0, totalPages: 0};
      const status: DoclingParseTaskStatus = { id: crypto.randomUUID(), state: "running", stage: "connecting", completedPages: 0, totalPages: 0 };
      tasks.set(status.id, { status, itemId, attachmentId, started: Date.now() });
      return status;
    },
    getParseTask: async id => {
      const task = tasks.get(id);
      if (!task) throw new Error("模拟解析任务不存在");
      if (task.status.state !== "running") return task.status;
      const elapsed = Date.now() - task.started;
      if (elapsed >= 4_000) {
        try {
          const result = await (task.saving ??= convertMockPdf(task.itemId, task.attachmentId));
          if (task.status.state === "running") task.status = { ...task.status, state: "succeeded", stage: "saving", parseRunId: result.id, completedPages: 8, totalPages: 8 };
        } catch (reason) {
          task.status = {...task.status, state: "failed", error: {code: "parse_failed", message: reason instanceof Error ? reason.message : String(reason)}};
        }
      } else {
        task.status = { ...task.status, stage: elapsed < 500 ? "connecting" : elapsed < 1_000 ? "queued" : "parsing",
          completedPages: Math.max(0, Math.floor((elapsed - 1_000) / 400)), totalPages: elapsed < 1_000 ? 0 : 8 };
      }
      return task.status;
    },
    cancelParseTask: async id => {
      const task = tasks.get(id); if (!task) throw new Error("模拟解析任务不存在");
      if (task.status.state === "running") task.status = { ...task.status, state: "cancelled" };
      return task.status;
    },
    getModelStorage: async () => modelStorage(),
    deleteModels: async () => { storage.setItem("docling-model-installed", "false"); status = { state: "disconnected" }; return modelStorage(); },
    getModelStatus: async () => modelStatus(),
    checkModelUpdate: async () => ({ ...modelStatus(), availableVersion: "preview-fp32", availableIdentity: "preview-models" }),
    downloadModels: async () => { storage.setItem("docling-model-installed", "true"); return modelStatus(); },
    getSettings: async () => settings,
    saveSettings: async (draft) => {
      settings = { ...draft, updatedAt: new Date().toISOString() };
      storage.setItem("docling-settings", JSON.stringify(settings));
      return settings;
    },
    getStatus: async () => status,
    pingService: async () => ({ status, latencyMs: 5 }),
    connectService: async () => {
      if (!installed()) throw { code: "models_unavailable", message: "尚未安装模型，请在设置中下载 FP32 模型" };
      status = {
        state: "connected",
        baseUrl: "mock://docling",
        activeClientCount: 1,
        activeRequestCount: 0,
      };
      return status;
    },
    disconnectService: async () => {
      status = { state: "disconnected" };
      return status;
    },
    parseAttachment: async (itemId, attachmentId, options) => {
      const item = await getLiteratureGateway().get(itemId);
      const fileName = item.attachments.find(attachment => attachment.id === attachmentId)?.displayName ?? "PDF";
      const file = item.attachments.find(attachment => attachment.id === attachmentId);
      const key = `${itemId}:${attachmentId}:${file?.contentSha256}:${mockDoclingIdentity(storage)}`;
      const existing = !options?.force ? flights.get(key) : undefined;
      if (existing) return existing;
      const operation = runDoclingParse(gateway, itemId, attachmentId, { paperTitle: item.title, fileName }, options?.force ?? false).then(result => {
        if (!result) throw Object.assign(new Error("Docling 解析已终止"), {code: "cancelled"});
        return result;
      }).finally(() => {if (flights.get(key) === operation) flights.delete(key);});
      flights.set(key, operation);
      return operation;
    },
    listParseRuns: async (id) =>
      [...runs.values()].filter((run) => run.itemId === id),
    getParseRun: async (id) => {
      const value = runs.get(id);
      if (!value) throw new Error("模拟解析记录不存在");
      return value;
    },
    getPageOverlay: async (attachmentId, pageIndex, parseRunId) => ({
      attachmentId,
      pageIndex,
      parseRunId:
        parseRunId ??
        [...runs.values()].find((run) => run.attachmentId === attachmentId)
          ?.id ??
        "preview",
      pageSize: { width: 612, height: 792 },
      items: [
        {
          id: "preview-title",
          kind: "title",
          label: "标题",
          textPreview: "Synthetic Research Preview",
          regions: [
            { pageIndex: 0, rect: { x: 60, y: 45, width: 300, height: 25 } },
          ],
          rawRef: "#/texts/0",
        },
      ],
    }),
  };
  return gateway;
}
