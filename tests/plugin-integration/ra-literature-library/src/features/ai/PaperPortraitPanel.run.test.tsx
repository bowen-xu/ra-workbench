// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { AiRun } from "@research-assistant/core/ai";
import type { AiAssistanceGateway } from "../../../../../../apps/web/src/core/services/aiAssistanceGateway";
import { PaperPortraitPanel } from "../../../../../../../plugins/ra-literature-library/src/features/ai/PaperPortraitPanel";
import { panelItem } from "../../../../../../../plugins/ra-literature-library/src/features/ai/paperOverviewTestFixtures";
import { selectAppStorage } from "../../../../../../apps/web/src/core/runtime/appStorage";
import { OPEN_AGENT_CONVERSATION, type OpenAgentConversation } from "../../../../../../apps/web/src/core/features/agent/openAgentConversation";

vi.mock("../../../../../../apps/web/src/core/services/notifications", () => ({ notify: { success: vi.fn(), info: vi.fn(), error: vi.fn() } }));

afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

it.each([true, false])("restores generation across rails and synchronizes cancellation (from summary=%s)", async fromSummary => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); vi.useFakeTimers();
  const storage = new Map<string, string>();
  selectAppStorage({ get length() { return storage.size; }, key: index => [...storage.keys()][index] ?? null,
    getItem: key => storage.get(key) ?? null, setItem: (key, value) => { storage.set(key, value); },
    removeItem: key => { storage.delete(key); }, clear: () => storage.clear() });
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  let terminal = false;
  const run: AiRun = { id: "portrait-run", taskKind: "paper_profile_classification", status: "generating",
    providerId: "fixture", modelId: "fixture", pipelineVersion: "v1", promptVersion: "v1", outputSchemaVersion: 1,
    userGoal: "portrait", scopeHash: "a".repeat(64), inputItemCount: 1, maxInputTokens: 100, maxOutputTokens: 100,
    timeoutMs: 120000, cancelRequested: false, createdAt: "now", updatedAt: "now", scopeItems: [], stages: [],
    inputTokens: 0, outputTokens: 0, sharedConversationId: "portrait-conversation", sharedRequestId: "portrait-request" };
  const gateway = { listCandidates: vi.fn(async () => []),
    previewTask: vi.fn(async () => ({ evidencePreview: [{ citationKey: "E1" }], scopeHash: "scope", requiresCloudConsent: false })),
    startRun: vi.fn(async () => run), getRun: vi.fn(async () => ({ ...run, status: terminal ? "canceled" : "generating" })),
    cancelRun: vi.fn(async () => { terminal = true; return true; }),
  } as unknown as AiAssistanceGateway;
  const paper = panelItem();
  const opened: string[] = [];
  const navigate = (event: Event) => {
    event.preventDefault();
    const request = (event as CustomEvent<OpenAgentConversation>).detail;
    opened.push(request.id); request.resolve();
  };
  window.addEventListener(OPEN_AGENT_CONVERSATION, navigate);
  const render = (library: boolean, reader: boolean) => act(async () => root.render(<>
    {library && <div key="library" data-view="library"><PaperPortraitPanel item={paper} gateway={gateway}/></div>}
    {reader && <div key="reader" data-view="reader"><PaperPortraitPanel item={paper} gateway={gateway}/></div>}
  </>));
  try {
    await render(true, false);
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="生成画像"]')!.click());
    expect(host.textContent).toContain("正在生成画像");
    expect(host.querySelector("h3")!.textContent).toBe("文献画像");
    await render(false, false);
    await render(false, true);
    expect(host.querySelector('[data-view="reader"]')!.textContent).toContain("正在生成画像");
    expect(host.querySelector<HTMLButtonElement>('[aria-label="取消生成"]')!.disabled).toBe(false);
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="打开画像内部对话"]')!.click());
    expect(opened).toEqual(["portrait-conversation"]);
    await render(true, true);
    expect(host.querySelector('[data-view="library"]')!.textContent).toContain("正在生成画像");
    await act(async () => host.querySelector<HTMLButtonElement>('[data-view="reader"] [aria-label="查看画像详情"]')!.click());
    const dialog = document.querySelector('[aria-label="文献画像详情"]')!;
    expect(dialog.querySelector<HTMLButtonElement>('[aria-label="移除来源 paper.pdf"]')!.disabled).toBe(true);
    expect(dialog.querySelector<HTMLButtonElement>('[aria-label="新增画像维度"]')!.disabled).toBe(true);
    await act(async () => (fromSummary
      ? host.querySelector<HTMLButtonElement>('[data-view="library"] [aria-label="取消生成"]')!
      : dialog.querySelector<HTMLButtonElement>('[aria-label="取消生成"]')!).click());
    expect(gateway.cancelRun).toHaveBeenCalledExactlyOnceWith("portrait-run");
    expect(gateway.startRun).toHaveBeenCalledOnce();
    expect(host.textContent).not.toContain("正在生成画像");
    expect(host.querySelectorAll("h3")[0]!.textContent).toBe("文献画像（待生成）");
    expect(host.querySelector<HTMLButtonElement>('[data-view="library"] [aria-label="生成画像"]')!.disabled).toBe(false);
    expect(dialog.textContent).not.toContain("正在生成画像");
    await act(async () => dialog.querySelector<HTMLButtonElement>('[aria-label="打开画像内部对话"]')!.click());
    expect(opened).toEqual(["portrait-conversation", "portrait-conversation"]);
    expect(document.querySelector('[aria-label="文献画像详情"]')).toBeNull();
  } finally { window.removeEventListener(OPEN_AGENT_CONVERSATION, navigate); await act(async () => root.unmount()); host.remove(); }
});

it("shows an unknown creation in both production rails and blocks another start", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const storage = new Map<string, string>();
  selectAppStorage({ get length() { return storage.size; }, key: index => [...storage.keys()][index] ?? null,
    getItem: key => storage.get(key) ?? null, setItem: (key, value) => { storage.set(key, value); },
    removeItem: key => { storage.delete(key); }, clear: () => storage.clear() });
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const id = "12345678-1234-1234-1234-123456789abc";
  const gateway = { listCandidates: vi.fn(async () => []),
    previewTask: vi.fn(async () => ({ evidencePreview: [], scopeHash: "scope", requiresCloudConsent: false })),
    startRun: vi.fn(async () => { throw { code: "data_operation_outcome_unknown", message: `请重启后核对原运行 ${id}` }; }),
  } as unknown as AiAssistanceGateway;
  const paper = panelItem();
  try {
    await act(async () => root.render(<><div data-view="library"><PaperPortraitPanel item={paper} gateway={gateway}/></div>
      <div data-view="reader"><PaperPortraitPanel item={paper} gateway={gateway}/></div></>));
    await act(async () => host.querySelector<HTMLButtonElement>('[data-view="library"] [aria-label="生成画像"]')!.click());
    for (const view of ["library", "reader"]) {
      const rail = host.querySelector(`[data-view="${view}"]`)!;
      expect(rail.querySelector('[role="alert"]')!.textContent).toContain(id);
      expect(rail.querySelector<HTMLButtonElement>('[aria-label="生成画像"]')!.disabled).toBe(true);
      expect(rail.textContent).not.toContain("重试读取画像生成状态");
    }
    expect(gateway.startRun).toHaveBeenCalledOnce();
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it.each(["data_operation_outcome_unknown", "ai_run_terminal_outcome_unknown"])("shows %s in both rails without cancel or duplicate generation", async (code) => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const id = "12345678-1234-1234-1234-123456789abc";
  const run: AiRun = {id, taskKind:"paper_profile_classification", status:"generating", providerId:"fixture", modelId:"fixture",
    pipelineVersion:"v1", promptVersion:"v1", outputSchemaVersion:1, userGoal:"portrait", scopeHash:"a".repeat(64),
    inputItemCount:1, maxInputTokens:100, maxOutputTokens:100, timeoutMs:120000, cancelRequested:false,
    createdAt:"now", updatedAt:"now", scopeItems:[], stages:[], inputTokens:0, outputTokens:0};
  const gateway = {listCandidates: vi.fn(async () => []),
    previewTask: vi.fn(async () => ({evidencePreview:[], scopeHash:"scope", requiresCloudConsent:false})),
    startRun: vi.fn(async () => run), getRun: vi.fn(async () => {throw {code, message:`请核对原运行 ${id}`};}),
    cancelRun: vi.fn(async () => true)} as unknown as AiAssistanceGateway;
  const paper = panelItem();
  try {
    await act(async () => root.render(<><div data-view="library"><PaperPortraitPanel item={paper} gateway={gateway}/></div>
      <div data-view="reader"><PaperPortraitPanel item={paper} gateway={gateway}/></div></>));
    await act(async () => host.querySelector<HTMLButtonElement>('[data-view="library"] [aria-label="生成画像"]')!.click());
    for (const view of ["library", "reader"]) {
      const rail = host.querySelector(`[data-view="${view}"]`)!;
      expect(rail.querySelector('[role="alert"]')!.textContent).toContain(id);
      expect(rail.querySelector<HTMLButtonElement>('[aria-label="生成画像"]')!.disabled).toBe(true);
      expect(rail.querySelector('[aria-label="取消生成"]')).toBeNull();
      expect(rail.textContent).not.toContain("重试读取画像生成状态");
    }
    expect(gateway.startRun).toHaveBeenCalledOnce(); expect(gateway.cancelRun).not.toHaveBeenCalled();
    await act(async () => host.querySelector<HTMLButtonElement>('[data-view="library"] [aria-label="查看画像详情"]')!.click());
    const dialog = document.querySelector('[aria-label="文献画像详情"]')!;
    expect(dialog.querySelector('[role="alert"]')!.textContent).toContain(id);
    expect(dialog.querySelector('[aria-label="取消生成"]')).toBeNull();
    expect(dialog.querySelector('[aria-label="生成画像"], [aria-label="重新生成画像"]')).toHaveProperty("disabled", true);
    expect(dialog.textContent).not.toContain("重试读取画像生成状态");
  } finally {await act(async () => root.unmount()); host.remove();}
});
