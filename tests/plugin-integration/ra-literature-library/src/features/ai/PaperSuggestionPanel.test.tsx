// @vitest-environment happy-dom
import { notify } from "@research-assistant/workbench-sdk/feedback";
vi.mock("../../../../../../apps/web/src/core/services/notifications", () => ({ notify: { error: vi.fn(), info: vi.fn(), success: vi.fn(), warning: vi.fn() } }));

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { AiCandidate, AiRun, AiTaskPreview } from "@research-assistant/core/ai";
import type { LiteratureItem } from "@research-assistant/core/literature";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../../../../../apps/web/src/core/features/models/LlmRoutePicker", () => ({
  LlmRoutePicker: () => <div data-testid="llm-route-picker" />,
}));

import type { AiAssistanceGateway } from "../../../../../../apps/web/src/core/services/aiAssistanceGateway";
import { PaperSuggestionPanel } from "../../../../../../../plugins/ra-literature-library/src/features/ai/PaperSuggestionPanel";

function item(): LiteratureItem {
  return {
    id: "paper",
    itemType: "journalArticle",
    title: "Paper",
    abstractText: "",
    containerTitle: "",
    volume: "",
    issue: "",
    pages: "",
    doi: "",
    url: "",
    language: "",
    bibtexKey: "",
    creators: [],
    tags: [],
    readingStatus: "unread",
    attachments: [{
      id: "attachment",
      displayName: "paper.pdf",
      mimeType: "application/pdf",
      byteSize: 1024,
      contentSha256: "c".repeat(64),
    }],
    projectIds: [],
    metadataSources: [],
    createdAt: "now",
    updatedAt: "now",
  };
}

function preview(): AiTaskPreview {
  return {
    taskKind: "paper_suggestions",
    suggestionCandidateKind: "attention_suggestion_v1",
    providerId: "openai",
    modelId: "gpt-vision",
    isCloud: true,
    scopeHash: "s".repeat(64),
    inputItemCount: 1,
    scopeItems: [],
    requiresCloudConsent: false,
    evidencePreview: [{
      attachmentId: "attachment",
      anchorId: "anchor",
      contentSha256: "c".repeat(64),
      paperTitle: "Paper",
      pageIndex: 0,
      citationKey: "E1",
      quoteExact: "quote",
      contextText: "context",
      relationKind: "direct_quote",
    }],
  };
}

function run(status: AiRun["status"], id = "run-suggestion"): AiRun {
  return {
    id,
    taskKind: "paper_suggestions",
    suggestionCandidateKind: "attention_suggestion_v1",
    status,
    providerId: "openai",
    modelId: "gpt-vision",
    pipelineVersion: "paper-suggestions-runtime-v1",
    promptVersion: "paper-suggestions-grounded-v1",
    outputSchemaVersion: 1,
    userGoal: "生成阅读建议",
    scopeHash: "s".repeat(64),
    inputItemCount: 1,
    maxInputTokens: 16_000,
    maxOutputTokens: 3_000,
    timeoutMs: 90_000,
    cancelRequested: false,
    createdAt: "now",
    updatedAt: "now",
    scopeItems: [],
    stages: [],
  };
}

function attentionCandidate({
  id = "candidate-concept",
  paperId = "paper",
  updatedAt = "2026-08-09T10:00:00.000Z",
}: {
  readonly id?: string;
  readonly paperId?: string;
  readonly updatedAt?: string;
} = {}): AiCandidate {
  return {
    id,
    runId: "run-suggestion",
    providerId: "openai",
    modelId: "gpt-vision",
    candidateKind: "attention_suggestion_v1",
    schemaVersion: 1,
    currentRevision: 1,
    reviewState: "unreviewed",
    validationStatus: "valid",
    originalPayloadHash: "h".repeat(64),
    createdAt: "2026-08-09T09:59:00.000Z",
    updatedAt,
    evidenceBundle: {
      id: `bundle-${id}`,
      runId: "run-suggestion",
      scopeHash: "s".repeat(64),
      retrievalConfigVersion: "test",
      validationStatus: "valid",
      createdAt: "2026-08-09T09:59:00.000Z",
      items: [{
        id: `evidence-${id}`,
        citationKey: "E1",
        ordinal: 1,
        anchorId: "anchor",
        paperId,
        paperTitle: "Paper",
        attachmentId: "attachment",
        contentSha256: "c".repeat(64),
        pageIndex: 0,
        quoteExact: "concept quote",
        quotePrefix: "",
        quoteSuffix: "",
        contextText: "concept context",
        relationKind: "direct_quote",
        validationStatus: "valid",
        currentAnchorStatus: "exact",
        createdAt: "2026-08-09T09:59:00.000Z",
      }],
    },
    revisions: [{
      id: `revision-${id}`,
      revision: 1,
      payloadHash: "p".repeat(64),
      editorKind: "ai",
      sourceRunId: "run-suggestion",
      changeReason: "generated",
      createdAt: "2026-08-09T10:00:00.000Z",
      payload: {
        kind: "attention_suggestion_v1",
        schemaVersion: 1,
        suggestions: [{
          id: "attention_1",
          title: "Inspect prototype evidence",
          reason: "The learned prototype hierarchy is central to the paper's argument.",
          priority: "high",
          evidenceItemIds: ["E1"],
        }],
      },
    }],
    decisions: [],
    attentionFeedback: [],
    suggestionItemDecisions: [],
    citationAssessments: [],
    overviewFieldConfirmations: [],
    summaryVersions: [],
  };
}

async function flush(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

function clickButton(container: HTMLElement, text: string): Promise<void> {
  const button = [...container.querySelectorAll("button")]
    .find((candidate) => candidate.textContent === text);
  return act(async () => button?.click());
}

describe("PaperSuggestionPanel", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.useRealTimers();
  });

  it.each(["data_operation_outcome_unknown", "ai_run_terminal_outcome_unknown"])(
    "retains %s during admission or observation without issuing another operation",
    async (code) => {
      vi.useFakeTimers();
      for (const admission of [true, false]) {
        const message = `请重启后核对原运行 run-suggestion，evidence_bundle，${code}`;
        const gateway = {
          listCandidates: vi.fn(async () => []),
          previewTask: vi.fn(async () => preview()),
          startRun: vi.fn(async () => {
            if (admission) throw {code, message};
            return run("queued");
          }),
          getRun: vi.fn(async () => { throw {code, message}; }),
          cancelRun: vi.fn(async () => true),
        } as unknown as AiAssistanceGateway;
        await act(async () => root.render(
          <PaperSuggestionPanel key={String(admission)} gateway={gateway} item={item()} onOpenAttachment={() => undefined} />,
        ));
        await flush();
        await clickButton(container, "预览并抽取");
        await flush();
        await clickButton(container, "生成候选");
        await flush();
        await act(async () => vi.advanceTimersByTimeAsync(700));
        expect(container.textContent).toContain(message);
        expect(container.textContent).not.toContain("取消建议");
        const prepare = [...container.querySelectorAll("button")].find(button => button.textContent === "预览并抽取");
        expect(prepare).toHaveProperty("disabled", true);
        await clickButton(container, "预览并抽取");
        await clickButton(container, "生成候选");
        await clickButton(container, "取消建议");
        const reads = vi.mocked(gateway.getRun).mock.calls.length;
        await act(async () => vi.advanceTimersByTimeAsync(10_000));
        expect(gateway.startRun).toHaveBeenCalledOnce();
        expect(gateway.previewTask).toHaveBeenCalledOnce();
        expect(gateway.cancelRun).not.toHaveBeenCalled();
        expect(gateway.getRun).toHaveBeenCalledTimes(reads);
      }
    },
  );

  it("tracks a started suggestion run until it reaches review", async () => {
    vi.useFakeTimers();
    const candidate = attentionCandidate();
    const gateway = {
      listCandidates: vi.fn()
        .mockResolvedValueOnce([])
        .mockResolvedValue([candidate, attentionCandidate({ id: "candidate-other", paperId: "other-paper" })]),
      previewTask: vi.fn(async () => preview()),
      startRun: vi.fn(async () => run("queued")),
      getRun: vi.fn(async () => run("needs_review")),
    } as unknown as AiAssistanceGateway;

    await act(async () => root.render(
      <PaperSuggestionPanel gateway={gateway} item={item()} onOpenAttachment={() => undefined} />,
    ));
    await flush();

    await clickButton(container, "预览并抽取");
    await flush();
    await clickButton(container, "生成候选");
    await flush();

    expect(gateway.startRun).toHaveBeenCalledWith(expect.objectContaining({
      taskKind: "paper_suggestions",
      confirmedScopeHash: "s".repeat(64),
      suggestionCandidateKind: "attention_suggestion_v1",
    }));
    expect(container.textContent).toContain("AI 正在生成阅读建议");
    expect(container.textContent).toContain("取消建议");
    const previewAgain = [...container.querySelectorAll("button")]
      .find((button) => button.textContent === "预览并抽取");
    expect(previewAgain).toHaveProperty("disabled", true);

    await act(async () => {
      vi.advanceTimersByTime(700);
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(gateway.getRun).toHaveBeenCalledWith("run-suggestion");
    expect(gateway.listCandidates).toHaveBeenCalledTimes(2);
    expect(notify.info).toHaveBeenCalledWith(expect.stringContaining("建议运行已完成（run-sugg…）；候选已显示在下方。"));
    expect(container.textContent).toContain("Inspect prototype evidence");
    expect(container.textContent).toContain("The learned prototype hierarchy is central");
    expect(container.textContent).toContain("concept quote");
    expect(container.textContent).not.toContain("other-paper");
    expect(container.textContent).not.toContain("取消建议");
  });

  it("starts collapsed with independent reading suggestions", async () => {
    const gateway = {
      listCandidates: vi.fn(async () => []),
    } as unknown as AiAssistanceGateway;

    await act(async () => root.render(
      <PaperSuggestionPanel gateway={gateway} item={item()} onOpenAttachment={() => undefined} />,
    ));
    await flush();

    const panel = container.querySelector<HTMLDetailsElement>(".paper-suggestion-panel");
    expect(panel?.open).toBe(false);
    expect(panel?.querySelector(":scope > summary")?.textContent).toBe("阅读注意力建议");
    expect(panel?.textContent).not.toContain("概念抽取");
  });

  it("can request cancellation for an active suggestion run", async () => {
    const gateway = {
      listCandidates: vi.fn(async () => []),
      previewTask: vi.fn(async () => preview()),
      startRun: vi.fn(async () => run("queued")),
      cancelRun: vi.fn(async () => true),
    } as unknown as AiAssistanceGateway;

    await act(async () => root.render(
      <PaperSuggestionPanel gateway={gateway} item={item()} onOpenAttachment={() => undefined} />,
    ));
    await flush();

    await clickButton(container, "预览并抽取");
    await flush();
    await clickButton(container, "生成候选");
    await flush();
    await clickButton(container, "取消建议");
    await flush();

    expect(gateway.cancelRun).toHaveBeenCalledWith("run-suggestion");
    expect(notify.info).toHaveBeenCalledWith(expect.stringContaining("已请求取消建议运行（run-sugg…）"));
    expect([...container.querySelectorAll("button")]
      .some((button) => button.textContent === "取消建议")).toBe(false);
  });
});
