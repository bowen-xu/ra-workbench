// @vitest-environment happy-dom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getAppStorage, selectAppStorage } from "../../../../../../apps/web/src/core/runtime/appStorage";
import { DeveloperModeProvider } from "../../../../../../apps/web/src/core/features/settings/DeveloperMode";
import { PORTRAIT_DIMENSIONS } from "@research-assistant/core/ai";
import type {
  AiRun,
  AiTaskPreview,
  AiTaskSpecDraft,
  PaperAnalysisStartDraft,
} from "@research-assistant/core/ai";

const notifications = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn(), warning: vi.fn(), info: vi.fn() }));
vi.mock("../../../../../../apps/web/src/core/services/notifications", () => ({ notify: notifications }));

import type { AiAssistanceGateway } from "../../../../../../apps/web/src/core/services/aiAssistanceGateway";
import type { DoclingGateway } from "../../../../../../apps/web/src/core/services/doclingGateway";
import type { LiteratureGateway } from "../../../../../../../plugins/ra-literature-library/src/services/literatureGateway";
import type { ResearchKnowledgeGateway } from "../../../../../../../plugins/ra-literature-library/src/services/knowledgeGateway";
import { overviewCandidate, workflow, failedConceptWorkflow, panelItem, profile } from "../../../../../../../plugins/ra-literature-library/src/features/ai/paperOverviewTestFixtures";
import type { AiCandidate, PaperAnalysisWorkflow } from "@research-assistant/core/ai";
import { launchPaperOverviewRun, listOverviewCandidatesForPaper, PaperOverviewPanel } from "../../../../../../../plugins/ra-literature-library/src/features/ai/PaperOverviewPanel";

function pendingResult<T>() {
  let complete: ((value: T | PromiseLike<T>) => void) | undefined;
  let fail: ((reason?: unknown) => void) | undefined;
  const promise = new Promise<T>((resolve, reject) => {complete = resolve; fail = reject;});
  return {promise,
    resolve(value: T) {if (!complete) throw new Error("Promise executor did not run"); complete(value);},
    reject(reason: unknown) {if (!fail) throw new Error("Promise executor did not run"); fail(reason);},
  };
}

async function flushPromises(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe("paper overview one-click launch", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    const values = new Map<string, string>();
    selectAppStorage({ get length() { return values.size; }, key: index => [...values.keys()][index] ?? null,
      getItem: key => values.get(key) ?? null, setItem: (key, value) => { values.set(key, value); }, removeItem: key => { values.delete(key); }, clear: () => values.clear() });
    vi.clearAllMocks();
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.useRealTimers();
  });

  it("validates the frozen scope internally and starts without a second UI decision", async () => {
    const task: AiTaskSpecDraft = {
      taskKind: "paper_overview", providerId: "", userGoal: "提取结构化信息",
      pipelineVersion: "pipeline", promptVersion: "prompt", outputSchemaVersion: 1,
      maxInputTokens: 100, maxOutputTokens: 100, timeoutMs: 1_000,
      scopeItems: [
        { itemKind: "attachment", objectId: "main-pdf" },
        { itemKind: "attachment", objectId: "supplement-pdf" },
      ],
    };
    const previewTask = vi.fn(async () => ({
      taskKind: "paper_overview", providerId: "openai", modelId: "model", isCloud: true,
      scopeHash: "scope", inputItemCount: 1, scopeItems: [], requiresCloudConsent: true,
      evidencePreview: [],
    }) satisfies AiTaskPreview);
    const startRun = vi.fn(async (_draft: AiTaskSpecDraft) => ({ id: "run" }) as AiRun);

    await expect(launchPaperOverviewRun({ previewTask, startRun }, task))
      .resolves.toEqual({ id: "run" });
    expect(previewTask).toHaveBeenCalledOnce();
    expect(startRun).toHaveBeenCalledWith(expect.objectContaining({
      confirmedScopeHash: "scope",
      cloudConsentVersion: "ai-cloud-consent-v1",
      scopeItems: [
        { itemKind: "attachment", objectId: "main-pdf" },
        { itemKind: "attachment", objectId: "supplement-pdf" },
      ],
    }));
  });

  it("loads overview candidates by paper without applying project filtering in the detail panel", async () => {
    const paperCandidate = overviewCandidate("visible", "paper");
    const otherCandidate = overviewCandidate("hidden", "other-paper");
    const listCandidates = vi.fn(async () => [paperCandidate, otherCandidate]);

    await expect(listOverviewCandidatesForPaper({ listCandidates }, "paper"))
      .resolves.toEqual([paperCandidate]);
    expect(listCandidates).toHaveBeenCalledWith(undefined, { paperId: "paper" });
  });

  it.each([false, true])("reveals the complete rail after parallel reads settle, including failed reads (failure=%s)", async failure => {
    const units = pendingResult<readonly never[]>();
    const candidates = pendingResult<readonly AiCandidate[]>();
    const workflow = pendingResult<undefined>();
    const aiGateway = {
      listCandidates: vi.fn(() => candidates.promise),
      getLatestPaperAnalysisWorkflow: vi.fn(() => workflow.promise),
    } as unknown as AiAssistanceGateway;
    const knowledgeGateway = {
      listReadingUnits: vi.fn(() => units.promise),
      listReadingProfiles: vi.fn(async () => []),
      getPaperReadingProfileSelection: vi.fn(async () => ({ paperId: "paper", profiles: [], revision: 0, isPersisted: false })),
    } as unknown as ResearchKnowledgeGateway;
    const doclingGateway = { listParseRuns: vi.fn(async () => []) } as unknown as DoclingGateway;
    const render = (id: string) => act(async () => root.render(createElement(PaperOverviewPanel, {
      item: { ...panelItem(), id }, aiGateway, knowledgeGateway, doclingGateway,
      onOpenAttachment: vi.fn(), onPaperReviewChanged: async () => undefined,
    })));
    await render("paper");
    const hidden = () => container.querySelector<HTMLElement>(".paper-overview-panel")!.hidden;
    expect(hidden()).toBe(true);
    expect(container.querySelector('[role="status"]')?.textContent).toBe("加载中…");
    expect(aiGateway.listCandidates).toHaveBeenCalled();
    expect(knowledgeGateway.listReadingUnits).toHaveBeenCalled();
    expect(doclingGateway.listParseRuns).not.toHaveBeenCalled();
    await act(async () => { units.resolve([]); workflow.resolve(undefined); });
    await flushPromises();
    expect(hidden()).toBe(true);
    await act(async () => { if (failure) candidates.reject(new Error("candidate read failed")); else candidates.resolve([]); });
    await flushPromises();
    expect(hidden()).toBe(false);
    expect(container.querySelector(".paper-reading-details-loading")).toBeNull();
    const nextCandidates = pendingResult<readonly AiCandidate[]>();
    vi.mocked(aiGateway.listCandidates).mockImplementationOnce(() => nextCandidates.promise);
    await render("other-paper");
    await flushPromises();
    expect(hidden()).toBe(true);
    await act(async () => { nextCandidates.resolve([]); });
    await flushPromises();
    expect(container.querySelectorAll(".paper-reading-details")).toHaveLength(1);
    expect(container.querySelector(".paper-reading-details")?.getAttribute("aria-busy")).toBe("false");
    expect(doclingGateway.listParseRuns).not.toHaveBeenCalled();
  });

  it("separates source model settings from persistent reading display preferences", async () => {
    const onReadingHighlightsVisibleChanged = vi.fn();
    const aiGateway = { listCandidates: vi.fn(async () => [overviewCandidate("reading", "paper")]),
      getLatestPaperAnalysisWorkflow: vi.fn(async () => undefined), getLlmConfiguration: vi.fn() } as unknown as AiAssistanceGateway;
    const knowledgeGateway = { listReadingUnits: vi.fn(async () => []), listReadingProfiles: vi.fn(async () => [profile]),
      getPaperReadingProfileSelection: vi.fn(async () => ({ paperId: "paper", profiles: [], revision: 0, isPersisted: false })) } as unknown as ResearchKnowledgeGateway;
    const props = { aiGateway, knowledgeGateway, item: panelItem(), doclingGateway: {} as DoclingGateway,
      onOpenAttachment: vi.fn(), onPaperReviewChanged: vi.fn(async () => undefined),
      readingHighlightsVisible: true, onReadingHighlightsVisibleChanged };
    await act(async () => root.render(createElement(PaperOverviewPanel, props))); await flushPromises();
    expect(container.querySelector('.source-pdf-header [aria-label="辅助阅读模型设置"]')).not.toBeNull();
    expect(container.querySelector('.structured-reading-header [aria-label="辅助阅读模型设置"]')).toBeNull();
    expect(container.querySelector('[aria-label="隐藏结构标注"]')).toBeNull();
    const settings = container.querySelector<HTMLButtonElement>('[aria-label="结构化阅读显示设置"]')!;
    await act(async () => settings.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 30)); });
    const popup = document.querySelector<HTMLElement>('[role="dialog"][aria-label="结构化阅读显示设置"]')!;
    expect(popup.textContent).not.toContain("模型服务");
    const select = popup.querySelector<HTMLSelectElement>('[aria-label="结构化阅读正文字号"]')!;
    await act(async () => { select.value = "large"; select.dispatchEvent(new Event("change", { bubbles: true })); });
    expect(container.querySelector('.structured-reading-panel')?.getAttribute('data-reading-text-size')).toBe('large');
    expect(popup.querySelector<HTMLInputElement>('[role="switch"]')?.checked).toBe(true);
    await act(async () => popup.querySelector<HTMLInputElement>('[type="checkbox"]')!.click());
    expect(onReadingHighlightsVisibleChanged).toHaveBeenCalledOnce();
    expect(aiGateway.getLlmConfiguration).not.toHaveBeenCalled();
    await act(async () => root.unmount()); root = createRoot(container);
    const { onReadingHighlightsVisibleChanged: _toggle, ...libraryProps } = props;
    await act(async () => root.render(createElement(PaperOverviewPanel, libraryProps)));
    expect(container.querySelector('.structured-reading-panel')?.getAttribute('data-reading-text-size')).toBe('large');
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="结构化阅读显示设置"]')!.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 30)); });
    expect(document.querySelector('.reading-settings-display input[type="checkbox"]')).toBeNull();
  });

  it("places developer PDF actions before model settings only in the auxiliary reading header", async () => {
    const props = { item: panelItem(), onOpenAttachment: vi.fn(), onPaperReviewChanged: vi.fn(async () => undefined),
      aiGateway: { listCandidates: vi.fn(async () => []), getLatestPaperAnalysisWorkflow: vi.fn(async () => undefined) } as unknown as AiAssistanceGateway,
      knowledgeGateway: { listReadingUnits: vi.fn(async () => []), listReadingProfiles: vi.fn(async () => []), getPaperReadingProfileSelection: vi.fn(async () => ({ paperId: "paper", profiles: [], revision: 0, isPersisted: false })) } as unknown as ResearchKnowledgeGateway,
      doclingGateway: { listParseRuns: vi.fn(async () => []) } as unknown as DoclingGateway };
    await act(async () => root.render(createElement(PaperOverviewPanel, props)));
    expect(container.querySelector('[aria-label="解析 PDF"]')).toBeNull();
    expect(container.querySelector('[aria-label="查看最新解析数据"]')).toBeNull();
    getAppStorage().setItem("research-assistant.developer-mode", "true");
    await act(async () => root.render(createElement(DeveloperModeProvider, null, createElement(PaperOverviewPanel, props))));
    await flushPromises();
    const actions = [...container.querySelectorAll<HTMLButtonElement>(".source-pdf-header-actions button")].map(button => button.getAttribute("aria-label"));
    expect(actions).toEqual(["解析 PDF", "查看最新解析数据", "辅助阅读模型设置"]);
    expect(container.querySelector('.structured-reading-header [aria-label="解析 PDF"]')).toBeNull();
    expect(container.querySelector('.structured-reading-header [aria-label="查看最新解析数据"]')).toBeNull();
  });

  it("uses the shared notification for candidate-load failures and retains the retry action", async () => {
    const aiGateway = {
      listCandidates: vi.fn(async () => { throw new Error("AI 运行数据保存失败，请重试"); }),
      getLatestPaperAnalysisWorkflow: vi.fn(async () => undefined),
    } as unknown as AiAssistanceGateway;
    const knowledgeGateway = {
      listReadingProfiles: vi.fn(async () => [profile]),
      getPaperReadingProfileSelection: vi.fn(async () => ({
        paperId: "paper",
        profiles: [],
        revision: 0,
        isPersisted: false,
        createdAt: "now",
        updatedAt: "now",
      })),
      listReadingUnits: vi.fn(async () => []),
    } as unknown as ResearchKnowledgeGateway;

    await act(async () => root.render(createElement(PaperOverviewPanel, {
      aiGateway,
      doclingGateway: { parseAttachment: vi.fn() } as unknown as DoclingGateway,
      item: panelItem(),
      knowledgeGateway,
      literatureGateway: { readAttachment: vi.fn() } as unknown as LiteratureGateway,
      onOpenAttachment: vi.fn(),
      onPaperReviewChanged: vi.fn(async () => undefined),
    })));
    await flushPromises();

    expect(container.textContent).not.toContain("无法读取本论文 AI 候选");
    expect(container.textContent).toContain("重试读取");
    expect(notifications.error).toHaveBeenCalledWith(expect.stringContaining("无法读取本论文 AI 候选"));
  });

  it("hides parse and raw-data developer controls in ordinary reading", async () => {
    const aiGateway = {
      listCandidates: vi.fn(async () => []),
      getLatestPaperAnalysisWorkflow: vi.fn(async () => undefined),
    } as unknown as AiAssistanceGateway;
    const knowledgeGateway = {
      listReadingProfiles: vi.fn(async () => [profile]),
      getPaperReadingProfileSelection: vi.fn(async () => ({
        paperId: "paper",
        profiles: [],
        revision: 0,
        isPersisted: false,
        createdAt: "now",
        updatedAt: "now",
      })),
      listReadingUnits: vi.fn(async () => []),
    } as unknown as ResearchKnowledgeGateway;
    const listParseRuns = vi.fn(async () => []);
    const doclingGateway = {
      listParseRuns,
      parseAttachment: vi.fn(),
    } as unknown as DoclingGateway;

    await act(async () => root.render(createElement(PaperOverviewPanel, {
      aiGateway,
      doclingGateway,
      item: panelItem(),
      knowledgeGateway,
      literatureGateway: { readAttachment: vi.fn() } as unknown as LiteratureGateway,
      onOpenAttachment: vi.fn(),
      onPaperReviewChanged: vi.fn(async () => undefined),
    })));
    await flushPromises();

    const structuredReading = container.querySelector(".structured-reading-panel");
    expect(structuredReading?.querySelector(".docling-raw-panel")).toBeNull();
    expect(container.querySelector('[aria-label="解析 PDF"]')).toBeNull();
    expect(container.querySelector('[aria-label="查看最新解析数据"]')).toBeNull();
    expect(listParseRuns).not.toHaveBeenCalled();
  });

  it("runs only structured reading from the extraction action", async () => {
    let visibleCandidates: readonly AiCandidate[] = [];
    const aiGateway = {
      listCandidates: vi.fn(async () => visibleCandidates),
      preparePaperReadingContext: vi.fn(async () => ({
        attachmentStatuses: [{
          attachmentId: "attachment",
          displayName: "paper.pdf",
          contentSha256: "c".repeat(64),
          parseRunId: "parse-run",
          parseStatus: "available",
        }],
        contextItems: [],
        imageRequests: [],
        textItemCount: 1,
        tableItemCount: 0,
        pictureItemCount: 0,
        model: { providerId: "openai", modelId: "vision-model" },
      })),
      getLatestPaperAnalysisWorkflow: vi.fn(async () => undefined),
      startPaperAnalysis: vi.fn(async () => {
        visibleCandidates = [overviewCandidate("overview", "paper")];
        return workflow("needs_review");
      }),
    } as unknown as AiAssistanceGateway;
    const knowledgeGateway = {
      listReadingProfiles: vi.fn(async () => [profile]),
      getPaperReadingProfileSelection: vi.fn(async () => ({
        paperId: "paper",
        profiles: [{ profileId: profile.profileId, profileVersion: profile.profileVersion }],
        revision: 2,
        isPersisted: true,
        createdAt: "now",
        updatedAt: "now",
      })),
      listReadingUnits: vi.fn(async () => []),
    } as unknown as ResearchKnowledgeGateway;
    const doclingGateway = { parseAttachment: vi.fn() } as unknown as DoclingGateway;
    const literatureGateway = { readAttachment: vi.fn() } as unknown as LiteratureGateway;

    await act(async () => root.render(createElement(PaperOverviewPanel, {
      aiGateway,
      doclingGateway,
      item: panelItem(),
      knowledgeGateway,
      literatureGateway,
      onOpenAttachment: vi.fn(),
      onPaperReviewChanged: vi.fn(async () => undefined),
    })));
    await flushPromises();

    const launch = [...container.querySelectorAll("button")]
      .find((button) => button.getAttribute("aria-label") === "提取结构化阅读");
    await act(async () => launch?.click());
    await flushPromises();

    expect(aiGateway.startPaperAnalysis).toHaveBeenCalledWith(expect.objectContaining({
      clientRequestId: expect.any(String),
      paperId: "paper",
      attachmentIds: ["attachment"],
      overviewProfileSelectionRevision: 2,
      imageAssets: [],
    }));
    expect(vi.mocked(aiGateway.startPaperAnalysis).mock.calls[0]?.[0].resourceInputs).toBeUndefined();
    expect(vi.mocked(aiGateway.startPaperAnalysis).mock.calls[0]?.[0].skillSourceIds).toBeUndefined();
    expect(container.textContent).not.toContain("AI 候选");
    expect(container.textContent).toContain("question");
    expect(notifications.success).toHaveBeenCalledWith("结构化阅读已完成。");
  });

  it.each(["accepted_unchanged", "unreviewed"] as const)("extracts with a %s per-paper template even when legacy profiles are unspecified", async reviewState => {
    const base = overviewCandidate("portrait", "paper");
    const candidate: AiCandidate = { ...base, candidateKind: "paper_portrait_v1", reviewState,
      revisions: [{ ...base.revisions[0]!, payload: { kind: "paper_portrait_v1", schemaVersion: 1, coverage: "第1页",
        claims: PORTRAIT_DIMENSIONS.map((dimension, index) => ({ id: `claim_${index}`, dimension, label: "形式描述", description: "明确模型", certainty: "supported", evidence: [] })),
        template: [{ id: "model", title: "模型与假设", question: "有哪些前提？", claimIds: ["claim_1"] }] } }],
      decisions: reviewState === "unreviewed" ? [] : [{ id: "accepted", candidateId: "portrait", candidateRevision: 1, action: "accept_unchanged", rationale: "checked", idempotencyKey: "accept", createdAt: "now" }] };
    const aiGateway = { listCandidates: vi.fn(async () => [candidate]), getLatestPaperAnalysisWorkflow: vi.fn(async () => undefined),
      preparePaperReadingContext: vi.fn(async () => ({ attachmentStatuses: [{ attachmentId: "attachment", parseStatus: "available" }], imageRequests: [] })),
      startPaperAnalysis: vi.fn(async () => workflow("partial")) } as unknown as AiAssistanceGateway;
    const knowledgeGateway = { listReadingProfiles: vi.fn(async () => [profile]), listReadingUnits: vi.fn(async () => []),
      getPaperReadingProfileSelection: vi.fn(async () => ({ paperId: "paper", profiles: [], revision: 0, isPersisted: false, createdAt: "now", updatedAt: "now" })) } as unknown as ResearchKnowledgeGateway;
    await act(async () => root.render(createElement(PaperOverviewPanel, { aiGateway, knowledgeGateway, item: panelItem(),
      doclingGateway: {} as DoclingGateway, literatureGateway: {} as LiteratureGateway,
      onOpenAttachment: vi.fn(), onPaperReviewChanged: vi.fn(async () => undefined) })));
    await flushPromises();
    expect(container.querySelector('.structured-reading-template-summary')).toBeNull();
    expect(container.textContent).not.toContain("尚未提取");
    const launch = [...container.querySelectorAll("button")].find(button => button.getAttribute("aria-label") === "提取结构化阅读");
    await act(async () => launch?.click()); await flushPromises();
    expect(aiGateway.startPaperAnalysis).toHaveBeenCalledWith(expect.objectContaining({ paperTemplateRef: { portraitCandidateId: "portrait", portraitRevision: 1 }, overviewProfileSelectionRevision: 0 }));
  });

  it.each([
    ["rejected", "当前文献画像已丢弃，历史模板不会用于提取。"],
  ] as const)("explains a %s portrait when no reading profile is selected", async (reviewState, message) => {
    const base = overviewCandidate("portrait", "paper");
    const candidate: AiCandidate = { ...base, candidateKind: "paper_portrait_v1", reviewState,
      revisions: [{ ...base.revisions[0]!, payload: { kind: "paper_portrait_v1", schemaVersion: 1, coverage: "第1页",
        claims: PORTRAIT_DIMENSIONS.map((dimension, index) => ({ id: `claim_${index}`, dimension, label: "待判断", description: "材料不足", certainty: "uncertain", evidence: [] })),
        template: [{ id: "model", title: "模型", question: "模型是什么？", claimIds: [] }] } }], decisions: [] };
    const aiGateway = { listCandidates: vi.fn(async () => [candidate]), getLatestPaperAnalysisWorkflow: vi.fn(async () => undefined), startPaperAnalysis: vi.fn() } as unknown as AiAssistanceGateway;
    const knowledgeGateway = { listReadingProfiles: vi.fn(async () => [profile]), listReadingUnits: vi.fn(async () => []),
      getPaperReadingProfileSelection: vi.fn(async () => ({ paperId: "paper", profiles: [], revision: 0, isPersisted: false })) } as unknown as ResearchKnowledgeGateway;
    await act(async () => root.render(createElement(PaperOverviewPanel, { aiGateway, knowledgeGateway, item: panelItem(),
      doclingGateway: {} as DoclingGateway, literatureGateway: {} as LiteratureGateway,
      onOpenAttachment: vi.fn(), onPaperReviewChanged: vi.fn(async () => undefined) })));
    await flushPromises();
    await act(async () => [...container.querySelectorAll("button")].find(button => button.getAttribute("aria-label") === "提取结构化阅读")?.click());
    expect(notifications.info).toHaveBeenCalledWith(expect.stringContaining(message));
    expect(aiGateway.startPaperAnalysis).not.toHaveBeenCalled();
  });

  it.each(["partial", "overview_ready", "concept_failed"] as const)("keeps legacy %s reading usable without concept or resource controls", async status => {
    const aiGateway = { listCandidates: vi.fn(async () => [overviewCandidate("overview", "paper")]),
      getLatestPaperAnalysisWorkflow: vi.fn(async () => status === "concept_failed" ? failedConceptWorkflow() : workflow(status)) } as unknown as AiAssistanceGateway;
    const knowledgeGateway = { listReadingProfiles: vi.fn(async () => [profile]),
      getPaperReadingProfileSelection: vi.fn(async () => undefined), listReadingUnits: vi.fn(async () => []) } as unknown as ResearchKnowledgeGateway;
    await act(async () => root.render(createElement(PaperOverviewPanel, { aiGateway, knowledgeGateway, item: panelItem(),
      doclingGateway: {} as DoclingGateway, literatureGateway: {} as LiteratureGateway,
      onOpenAttachment: vi.fn(), onPaperReviewChanged: vi.fn(async () => undefined) })));
    await flushPromises();
    expect(container.textContent).toContain("question");
    expect(container.textContent).not.toContain("概念抽取");
    expect(container.textContent).not.toContain("关键概念尚未完成");
    expect(container.querySelector('[aria-label="添加上下文和工具"]')).toBeNull();
    expect(container.querySelector('.reading-review-legend')).toBeNull();
    expect(container.querySelector('.structured-reading-template-summary')).toBeNull();
    expect(container.querySelector<HTMLButtonElement>('[aria-label="重新生成结构化阅读"]')?.disabled).toBe(false);
    expect(container.querySelector('.structured-reading-workflow-error')).toBeNull();
  });

  it("shows the safe turn error instead of claiming that no candidate exists", async () => {
    const aiGateway = {
      listCandidates: vi.fn(async () => []),
      getLatestPaperAnalysisWorkflow: vi.fn(async () => workflow("failed")),
    } as unknown as AiAssistanceGateway;
    const knowledgeGateway = {
      listReadingProfiles: vi.fn(async () => []),
      getPaperReadingProfileSelection: vi.fn(async () => undefined),
      listReadingUnits: vi.fn(async () => []),
    } as unknown as ResearchKnowledgeGateway;
    await act(async () => root.render(createElement(PaperOverviewPanel, {
      aiGateway, doclingGateway: {} as DoclingGateway, item: panelItem(), knowledgeGateway,
      literatureGateway: {} as LiteratureGateway, onOpenAttachment: vi.fn(),
      onPaperReviewChanged: vi.fn(async () => undefined),
    })));
    await flushPromises();

    expect(container.textContent).toContain("候选已保存，但本地状态读取失败。");
    expect(container.textContent).toContain("storage_error");
    expect(container.textContent).not.toContain("未生成可审查的概要");
  });

  it("shares the running task across rails and reopening, with an icon canceling its original request", async () => {
    let finishWorkflow: ((value: PaperAnalysisWorkflow) => void) | undefined;
    const pendingWorkflow = new Promise<PaperAnalysisWorkflow>((resolve) => {
      finishWorkflow = resolve;
    });
    const cancelPaperAnalysis = vi.fn(async () => true);
    const startPaperAnalysis = vi.fn((_draft: PaperAnalysisStartDraft) => pendingWorkflow);
    const aiGateway = {
      listCandidates: vi.fn(async () => []),
      preparePaperReadingContext: vi.fn(async () => ({
        attachmentStatuses: [{ attachmentId: "attachment", displayName: "paper.pdf",
          contentSha256: "c".repeat(64), parseRunId: "parse-run", parseStatus: "available" }],
        contextItems: [], imageRequests: [], textItemCount: 1, tableItemCount: 0,
        pictureItemCount: 0, model: { providerId: "openai", modelId: "vision-model" },
      })),
      getLatestPaperAnalysisWorkflow: vi.fn(async () => undefined),
      startPaperAnalysis,
      cancelPaperAnalysis,
    } as unknown as AiAssistanceGateway;
    const knowledgeGateway = {
      listReadingProfiles: vi.fn(async () => [profile]),
      getPaperReadingProfileSelection: vi.fn(async () => ({
        paperId: "paper", profiles: [{ profileId: profile.profileId,
          profileVersion: profile.profileVersion }], revision: 2, isPersisted: true,
        createdAt: "now", updatedAt: "now",
      })),
      listReadingUnits: vi.fn(async () => []),
    } as unknown as ResearchKnowledgeGateway;

    const props = {
      aiGateway,
      doclingGateway: { parseAttachment: vi.fn() } as unknown as DoclingGateway,
      item: panelItem(),
      knowledgeGateway,
      literatureGateway: { readAttachment: vi.fn() } as unknown as LiteratureGateway,
      onOpenAttachment: vi.fn(),
      onPaperReviewChanged: vi.fn(async () => undefined),
    };
    await act(async () => root.render(createElement("div", null,
      createElement(PaperOverviewPanel, { ...props, key: "library" }),
      createElement(PaperOverviewPanel, { ...props, key: "reader" }),
    )));
    await flushPromises();
    await act(async () => {
      [...container.querySelectorAll("button")]
        .find((button) => button.getAttribute("aria-label") === "提取结构化阅读")?.click();
      await Promise.resolve();
    });
    const requestId = startPaperAnalysis.mock.calls[0]?.[0].clientRequestId;
    expect(requestId).toEqual(expect.any(String));
    const cancelButtons = container.querySelectorAll<HTMLButtonElement>('[aria-label="取消提取"]');
    expect(cancelButtons).toHaveLength(2);
    for (const button of cancelButtons) { expect(button.textContent).toBe(""); expect(button.querySelector("svg")).not.toBeNull(); }
    expect(container.querySelector('[aria-label="提取结构化阅读"]')).toBeNull();
    // The native request continues when both original panels are replaced.
    await act(async () => root.render(createElement(PaperOverviewPanel, { ...props, key: "reopened" })));
    await flushPromises();
    expect(container.querySelector('[aria-label="取消提取"]')).not.toBeNull();
    expect(startPaperAnalysis).toHaveBeenCalledOnce();

    await act(async () => {
      [...container.querySelectorAll("button")]
        .find((button) => button.getAttribute("aria-label") === "取消提取")?.click();
      await Promise.resolve();
    });
    expect(cancelPaperAnalysis).toHaveBeenCalledWith(requestId);
    await act(async () => {
      finishWorkflow?.(workflow("canceled"));
      await pendingWorkflow;
    });

    expect(notifications.info).toHaveBeenCalledWith(
      "论文分析已取消；已完成的结构化阅读候选会保留。",
    );
  });
  it.each(["data_operation_outcome_unknown", "ai_run_terminal_outcome_unknown"])("preserves %s even when cancellation was requested", async (code) => {
    const pending = pendingResult<PaperAnalysisWorkflow>();
    const cancellation = pendingResult<boolean>();
    const startPaperAnalysis = vi.fn(() => pending.promise);
    const aiGateway = {
      listCandidates: vi.fn(async () => []), getLatestPaperAnalysisWorkflow: vi.fn(async () => undefined),
      preparePaperReadingContext: vi.fn(async () => ({ attachmentStatuses: [{ attachmentId: "attachment", parseStatus: "available" }], imageRequests: [] })),
      startPaperAnalysis, cancelPaperAnalysis: vi.fn(() => cancellation.promise),
    } as unknown as AiAssistanceGateway;
    const knowledgeGateway = {
      listReadingProfiles: vi.fn(async () => [profile]),
      getPaperReadingProfileSelection: vi.fn(async () => ({ paperId: "paper", profiles: [{ profileId: profile.profileId,
        profileVersion: profile.profileVersion }], revision: 2, isPersisted: true, createdAt: "now", updatedAt: "now" })),
      listReadingUnits: vi.fn(async () => []),
    } as unknown as ResearchKnowledgeGateway;
    const props = { aiGateway, knowledgeGateway, item: panelItem(), doclingGateway: {} as DoclingGateway,
      literatureGateway: {} as LiteratureGateway, onOpenAttachment: vi.fn(), onPaperReviewChanged: vi.fn(async () => undefined) };
    await act(async () => root.render(createElement("div", null,
      createElement(PaperOverviewPanel, { ...props, key: "library" }),
      createElement(PaperOverviewPanel, { ...props, key: "reader" }))));
    await flushPromises();
    await act(async () => { container.querySelector<HTMLButtonElement>('[aria-label="提取结构化阅读"]')!.click(); await Promise.resolve(); });
    expect(startPaperAnalysis).toHaveBeenCalledOnce();
    await act(async () => { container.querySelector<HTMLButtonElement>('[aria-label="取消提取"]')!.click(); await Promise.resolve(); });
    const id = "12345678-1234-1234-1234-123456789abc";
    await act(async () => { pending.reject({ code, message: `请重启后核对原运行 ${id}` }); await Promise.resolve(); });
    await act(async () => { cancellation.resolve(true); await Promise.resolve(); });
    expect(notifications.info).not.toHaveBeenCalledWith("论文分析已取消。");
    expect(notifications.info).not.toHaveBeenCalledWith("正在取消当前论文分析阶段；已完成结果会保留。");
    expect(notifications.error).toHaveBeenCalledWith(expect.stringContaining(id));
    const alerts = [...container.querySelectorAll('[role="alert"]')].filter(alert => alert.textContent?.includes(id));
    expect(alerts).toHaveLength(2);
    for (const button of container.querySelectorAll<HTMLButtonElement>('[aria-label="提取结果待核对"]')) expect(button.disabled).toBe(true);
    expect(container.querySelectorAll('[aria-label="提取结果待核对"]')).toHaveLength(2);
    await act(async () => root.render(createElement(PaperOverviewPanel, { ...props, key: "reopened" })));
    await flushPromises();
    expect(container.querySelector('[role="alert"]')?.textContent).toContain(id);
    expect(container.querySelector('[aria-label="取消提取"]')).toBeNull();
    expect(startPaperAnalysis).toHaveBeenCalledOnce();
  });
  it("persists the cleared view without exposing the obsolete version manager", async () => {
    const candidate = overviewCandidate("old", "paper");
    const aiGateway = { listCandidates: vi.fn(async () => [candidate]), getLatestPaperAnalysisWorkflow: vi.fn(async () => undefined) } as unknown as AiAssistanceGateway;
    const knowledgeGateway = { listReadingProfiles: vi.fn(async () => [profile]), listReadingUnits: vi.fn(async () => []), getPaperReadingProfileSelection: vi.fn(async () => ({ paperId: "paper", profiles: [], revision: 0, isPersisted: false, createdAt: "now", updatedAt: "now" })) } as unknown as ResearchKnowledgeGateway;
    const props = { aiGateway, knowledgeGateway, item: panelItem(), doclingGateway: { listParseRuns: vi.fn(async () => []) } as unknown as DoclingGateway, literatureGateway: {} as LiteratureGateway, onOpenAttachment: vi.fn(), onPaperReviewChanged: vi.fn(async () => undefined) };
    await act(async () => root.render(createElement(PaperOverviewPanel, props))); await flushPromises();
    expect(container.querySelectorAll(".reading-category-card")).toHaveLength(1);
    expect(container.querySelector(".structured-reading-header h3")?.textContent).toBe("结构化阅读记录");
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="清空结构化阅读记录"]')!.click());
    expect(container.querySelectorAll(".reading-category-card")).toHaveLength(1);
    await act(async () => [...document.querySelectorAll("button")].find(button => button.textContent === "确认清空")!.click());
    expect(container.querySelectorAll(".reading-category-card")).toHaveLength(0);
    expect(container.querySelector(".structured-reading-header h3")?.textContent).toBe("结构化阅读记录（待生成）");
    await act(async () => root.render(null));
    await act(async () => root.render(createElement(PaperOverviewPanel, props))); await flushPromises();
    expect(container.querySelectorAll(".reading-category-card")).toHaveLength(0);
    expect(container.querySelector('[aria-label="阅读版本管理"]')).toBeNull();
    expect(container.querySelector('[aria-label="历史阅读版本"]')).toBeNull();
    expect(candidate.reviewState).toBe("unreviewed");
  });

  it("preserves a formal reading edit made while regeneration is running", async () => {
    const old = overviewCandidate("old", "paper");
    const candidate: AiCandidate = { ...old, overviewFieldConfirmations: [{ id: "confirmation", batchId: "batch", candidateId: old.id, candidateRevision: 1, fieldKind: "research_question", fieldPayloadHash: "f".repeat(64), decisionKind: "ai_unchanged", targetReadingUnitId: "formal", targetAppliedRevision: 1, active: true, createdAt: "now" }] };
    let unit = { id: "formal", paperId: "paper", unitKind: "research_question" as const, title: "Question", body: "Reviewed original", provenanceKind: "ai_accepted_unchanged" as const, epistemicRole: "source_paraphrase" as const, revision: 1, evidenceCount: 1, createdAt: "now", updatedAt: "now" };
    let candidates = [candidate];
    const aiGateway = { listCandidates: vi.fn(async () => candidates), getLatestPaperAnalysisWorkflow: vi.fn(async () => undefined), preparePaperReadingContext: vi.fn(async () => ({ attachmentStatuses: [{ attachmentId: "attachment", parseStatus: "available" }], imageRequests: [] })), startPaperAnalysis: vi.fn(async () => { unit = { ...unit, body: "Human edit during generation", revision: 2 }; candidates = [overviewCandidate("overview", "paper"), candidate]; return workflow("needs_review"); }) } as unknown as AiAssistanceGateway;
    const knowledgeGateway = { listReadingProfiles: vi.fn(async () => [profile]), listReadingUnits: vi.fn(async () => [unit]), getPaperReadingProfileSelection: vi.fn(async () => ({ paperId: "paper", profiles: [{ profileId: profile.profileId, profileVersion: profile.profileVersion }], revision: 1, isPersisted: true, createdAt: "now", updatedAt: "now" })) } as unknown as ResearchKnowledgeGateway;
    await act(async () => root.render(createElement(PaperOverviewPanel, { aiGateway, knowledgeGateway, item: panelItem(), doclingGateway: { listParseRuns: vi.fn(async () => []) } as unknown as DoclingGateway, literatureGateway: {} as LiteratureGateway, onOpenAttachment: vi.fn(), onPaperReviewChanged: vi.fn(async () => undefined) }))); await flushPromises();
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="重新生成结构化阅读"]')!.click());
    expect(aiGateway.startPaperAnalysis).not.toHaveBeenCalled();
    const all = [...document.querySelectorAll<HTMLInputElement>('input[type="radio"]')].at(-1)!;
    await act(async () => all.click());
    await act(async () => [...document.querySelectorAll("button")].find(button => button.textContent === "确认重新生成")!.click()); await flushPromises();
    expect(aiGateway.startPaperAnalysis).toHaveBeenCalledWith(expect.objectContaining({ regenerate: true }));
    expect(container.textContent).toContain("Human edit during generation");
    expect(candidate.overviewFieldConfirmations[0]?.active).toBe(true);
  });

});
