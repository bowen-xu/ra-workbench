// @vitest-environment happy-dom
import { notify } from "@research-assistant/workbench-sdk/feedback";
vi.mock("../../../../../../apps/web/src/core/services/notifications", () => ({ notify: { error: vi.fn(), info: vi.fn(), success: vi.fn(), warning: vi.fn() } }));

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type {
  AiCandidate,
  AiEvidenceItem,
  AiRun,
  AiTaskPreview,
  PaperOverviewUnitCandidateV1,
} from "@research-assistant/core/ai";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../../../../../apps/web/src/core/features/models/LlmRoutePicker", () => ({
  LlmRoutePicker: () => <div data-testid="llm-route-picker" />,
}));

import type { AiAssistanceGateway } from "../../../../../../apps/web/src/core/services/aiAssistanceGateway";
import { AiOverviewRetryPanel } from "../../../../../../../plugins/ra-literature-library/src/features/ai/AiOverviewRetryPanel";

const payload: PaperOverviewUnitCandidateV1 = {
  kind: "paper_overview_unit_v1",
  schemaVersion: 1,
  profileId: "generic_overview_v1",
  fields: [
    {
      field: "research_question",
      status: "filled",
      statements: [{
        id: "statement-question",
        text: "question",
        epistemicRole: "source_paraphrase",
        evidenceItemIds: ["E1"],
      }],
    },
    {
      field: "limitation",
      status: "not_found",
      statements: [],
    },
  ],
};

function evidence(): AiEvidenceItem {
  return {
    id: "evidence",
    citationKey: "E1",
    ordinal: 0,
    anchorId: "anchor",
    paperId: "paper",
    paperTitle: "Paper",
    attachmentId: "attachment",
    contentSha256: "c".repeat(64),
    pageIndex: 0,
    quoteExact: "quote",
    quotePrefix: "",
    quoteSuffix: "",
    contextText: "context",
    relationKind: "direct_quote",
    validationStatus: "valid",
    currentAnchorStatus: "exact",
    createdAt: "now",
  };
}

function candidate(): AiCandidate {
  return {
    id: "candidate",
    runId: "run-original",
    providerId: "openai",
    modelId: "model",
    candidateKind: "paper_overview_unit_v1",
    schemaVersion: 1,
    currentRevision: 2,
    reviewState: "edited",
    validationStatus: "valid",
    originalPayloadHash: "a".repeat(64),
    createdAt: "now",
    updatedAt: "now",
    evidenceBundle: {
      id: "bundle",
      runId: "run-original",
      scopeHash: "b".repeat(64),
      retrievalConfigVersion: "docling-context-v1",
      validationStatus: "valid",
      createdAt: "now",
      items: [evidence()],
    },
    revisions: [],
    decisions: [],
    attentionFeedback: [],
    suggestionItemDecisions: [],
    citationAssessments: [],
    overviewFieldConfirmations: [],
  };
}

function preview(): AiTaskPreview {
  return {
    taskKind: "paper_overview",
    providerId: "openai",
    modelId: "vision-model",
    isCloud: true,
    scopeHash: "s".repeat(64),
    inputItemCount: 1,
    scopeItems: [],
    requiresCloudConsent: false,
    evidencePreview: [{
      citationKey: "E1",
      anchorId: "anchor",
      paperTitle: "Paper",
      attachmentId: "attachment",
      contentSha256: "c".repeat(64),
      pageIndex: 0,
      quoteExact: "quote",
      contextText: "context",
      relationKind: "direct_quote",
    }],
  };
}

function run(status: AiRun["status"], id = "run-retry"): AiRun {
  return {
    id,
    taskKind: "paper_overview",
    status,
    providerId: "openai",
    modelId: "vision-model",
    pipelineVersion: "paper-overview-runtime-v1",
    promptVersion: "paper-overview-field-retry-v1",
    outputSchemaVersion: 1,
    userGoal: "重试字段",
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

async function flush(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

function button(container: HTMLElement, text: string): HTMLButtonElement {
  const match = [...container.querySelectorAll("button")]
    .find((candidate) => candidate.textContent === text);
  if (match === undefined) throw new Error(`Missing button ${text}`);
  return match;
}

describe("AiOverviewRetryPanel", () => {
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

  it("tracks a field retry run until it creates a new AI revision", async () => {
    vi.useFakeTimers();
    const gateway = {
      previewTask: vi.fn(async () => preview()),
      startRun: vi.fn(async () => run("queued")),
      getRun: vi.fn(async () => run("needs_review")),
    } as unknown as AiAssistanceGateway;

    await act(async () => root.render(
      <AiOverviewRetryPanel candidate={candidate()} disabled={false} gateway={gateway} payload={payload} />,
    ));

    await act(async () => button(container, "选择全部未完成字段").click());
    await act(async () => button(container, "预览重试外发范围").click());
    await flush();
    await act(async () => button(container, "启动所选字段重试").click());
    await flush();

    expect(gateway.startRun).toHaveBeenCalledWith(expect.objectContaining({
      taskKind: "paper_overview",
      confirmedScopeHash: "s".repeat(64),
      overviewFields: ["limitation"],
      baseCandidateId: "candidate",
      baseCandidateRevision: 2,
    }));
    expect(container.textContent).toContain("AI 正在重试所选字段");
    expect(container.textContent).toContain("取消重试");
    expect(button(container, "预览重试外发范围")).toHaveProperty("disabled", true);

    await act(async () => {
      vi.advanceTimersByTime(700);
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(gateway.getRun).toHaveBeenCalledWith("run-retry");
    expect(notify.info).toHaveBeenCalledWith(expect.stringContaining("字段重试已完成（run-retr…）；已保存新的 AI revision。"));
    expect([...container.querySelectorAll("button")]
      .some((candidate) => candidate.textContent === "取消重试")).toBe(false);
  });

  it("can request cancellation for an active field retry", async () => {
    const gateway = {
      previewTask: vi.fn(async () => preview()),
      startRun: vi.fn(async () => run("queued")),
      cancelRun: vi.fn(async () => true),
    } as unknown as AiAssistanceGateway;

    await act(async () => root.render(
      <AiOverviewRetryPanel candidate={candidate()} disabled={false} gateway={gateway} payload={payload} />,
    ));

    await act(async () => button(container, "选择全部未完成字段").click());
    await act(async () => button(container, "预览重试外发范围").click());
    await flush();
    await act(async () => button(container, "启动所选字段重试").click());
    await flush();
    await act(async () => button(container, "取消重试").click());
    await flush();

    expect(gateway.cancelRun).toHaveBeenCalledWith("run-retry");
    expect(notify.info).toHaveBeenCalledWith(expect.stringContaining("已请求取消字段重试（run-retr…）"));
    expect([...container.querySelectorAll("button")]
      .some((candidate) => candidate.textContent === "取消重试")).toBe(false);
  });
});
