// @vitest-environment happy-dom
import { notify } from "@research-assistant/workbench-sdk/feedback";
vi.mock("../../../../../../apps/web/src/core/services/notifications", () => ({ notify: { error: vi.fn(), info: vi.fn(), success: vi.fn(), warning: vi.fn() } }));

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { AiCandidate, AiRun } from "@research-assistant/core/ai";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AiAssistanceGateway } from "../../../../../../apps/web/src/core/services/aiAssistanceGateway";
import { AiRunHistoryPanel } from "./AiRunHistoryPanel";

function run(overrides: Partial<AiRun> = {}): AiRun {
  return {
    id: "run-12345678",
    taskKind: "paper_overview",
    status: "generating",
    providerId: "qwen",
    modelId: "qwen-vl",
    pipelineVersion: "pipeline",
    promptVersion: "prompt",
    outputSchemaVersion: 1,
    userGoal: "overview",
    scopeHash: "a".repeat(64),
    inputItemCount: 1,
    maxInputTokens: 64_000,
    maxOutputTokens: 4_000,
    timeoutMs: 120_000,
    cloudConsentVersion: "cloud-consent-v1",
    cloudConsentedAt: "2026-08-09T00:00:00Z",
    cancelRequested: false,
    createdAt: "2026-08-09T00:00:00Z",
    updatedAt: "2026-08-09T00:00:01Z",
    scopeItems: [{
      id: "scope",
      ordinal: 0,
      itemKind: "attachment",
      objectId: "attachment-abcdef123456",
      parentObjectId: "paper-abcdef123456",
      contentSha256: "c".repeat(64),
      objectRevision: 3,
      snapshotHash: "b".repeat(64),
      createdAt: "now",
    }],
    stages: [{
      id: "stage",
      runId: "run-12345678",
      stageKind: "generating",
      attempt: 1,
      status: "running",
      inputTokens: 12,
      outputTokens: 5,
      inputHash: "input-hash-value",
      outputHash: "output-hash-value",
      durationMs: 320,
      createdAt: "now",
    }],
    ...overrides,
  };
}

function candidate(runId: string, reviewState: AiCandidate["reviewState"]): AiCandidate {
  return { id: `${runId}-${reviewState}`, runId, reviewState } as AiCandidate;
}

async function flush(): Promise<void> {
  await act(async () => { await new Promise((resolve) => window.setTimeout(resolve, 0)); });
}

describe("AiRunHistoryPanel", () => {
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
  });

  it("lists recent AI runs and requests cancellation for active runs", async () => {
    const listRuns = vi.fn(async () => [run({
      sharedConversationId: "conversation-12345678",
      sharedRequestId: "request-12345678",
      sharedTurnId: "turn-12345678",
      sharedTurnOutcome: "completed",
      sharedCandidateId: "candidate-12345678",
      sharedTerminalStatus: "needs_review",
    })]);
    const listCandidates = vi.fn(async () => [
      candidate("run-12345678", "unreviewed"),
      candidate("run-12345678", "accepted_after_edit"),
      candidate("other-run", "unreviewed"),
    ]);
    const cancelRun = vi.fn(async () => true);
    const gateway = { listRuns, listCandidates, cancelRun } as unknown as AiAssistanceGateway;

    await act(async () => root.render(<AiRunHistoryPanel gateway={gateway} projectId="project" />));
    await flush();

    expect(listCandidates).toHaveBeenCalledWith("project");
    expect(container.textContent).toContain("论文概要");
    expect(container.textContent).toContain("生成中");
    expect(container.textContent).toContain("qwen / qwen-vl");
    expect(container.textContent).toContain("shared turn completed");
    expect(container.textContent).toContain("2 个候选 · 1 待审查 · 1 已终态");
    expect(container.textContent).toContain("pipeline · prompt · schema 1");
    expect(container.textContent).toContain("输入 64000 · 输出 4000 · 超时 120000 ms");
    expect(container.textContent).toContain("已确认 cloud-consent-v1");
    expect(container.textContent).toContain("不保存 provider 原始请求/响应");
    expect(container.textContent).toContain("conversation conversation-12345678 · request request-12345678 · turn turn-12345678 · completed · candidate candidate-12345678 · legacy needs_review");
    expect(container.textContent).toContain("Scope Snapshot");
    expect(container.textContent).toContain("1. attachment");
    expect(container.textContent).toContain("attachment-a…");
    expect(container.textContent).toContain("parent paper-abcdef… · revision 3 · content cccccccccccc… · snapshot bbbbbbbbbbbb…");
    expect(container.textContent).toContain("阶段记录");
    expect(container.textContent).toContain("generating #1");
    expect(container.textContent).toContain("输入 12 · 输出 5 · 320 ms");
    expect(container.textContent).toContain("input input-hash-v…");
    expect(container.textContent).not.toContain("查看候选");

    const cancel = [...container.querySelectorAll("button")].find((button) => button.textContent === "取消");
    await act(async () => cancel?.click());
    await flush();

    expect(cancelRun).toHaveBeenCalledWith("run-12345678");
    expect(notify.info).toHaveBeenCalledWith(expect.stringContaining("已请求取消 run-1234…"));
  });

  it("does not offer cancellation for terminal runs", async () => {
    const gateway = {
      listRuns: vi.fn(async () => [run({
        status: "failed",
        errorMessage: "模型超时",
        stages: [
          ...run().stages,
          {
            id: "stage-failed",
            runId: "run-12345678",
            stageKind: "validating",
            attempt: 1,
            status: "failed",
            errorCode: "schema_invalid",
            errorMessage: "字段缺失",
            createdAt: "later",
          },
        ],
      })]),
      listCandidates: vi.fn(async () => []),
      cancelRun: vi.fn(),
    } as unknown as AiAssistanceGateway;

    await act(async () => root.render(<AiRunHistoryPanel gateway={gateway} />));
    await flush();

    expect(container.textContent).toContain("失败");
    expect(container.textContent).toContain("模型超时");
    expect(container.textContent).toContain("schema_invalid：字段缺失");
    expect(container.textContent).toContain("尚无候选");
    expect([...container.querySelectorAll("button")].some((button) => button.textContent === "取消")).toBe(false);
  });

  it("filters recent runs by task kind and grouped status", async () => {
    const gateway = {
      listRuns: vi.fn(async () => [
        run({ id: "run-overview", taskKind: "paper_overview", status: "generating" }),
        run({ id: "run-reference", taskKind: "reference_extraction", status: "failed", errorMessage: "解析失败" }),
      ]),
      listCandidates: vi.fn(async () => []),
      cancelRun: vi.fn(),
    } as unknown as AiAssistanceGateway;

    await act(async () => root.render(<AiRunHistoryPanel gateway={gateway} />));
    await flush();

    expect(container.textContent).toContain("论文概要");
    expect(container.textContent).toContain("参考文献提取");

    const taskFilter = container.querySelector<HTMLSelectElement>("select[aria-label='运行任务筛选']");
    await act(async () => {
      taskFilter!.value = "reference_extraction";
      taskFilter!.dispatchEvent(new Event("change", { bubbles: true }));
    });
    const filteredCards = [...container.querySelectorAll("article.ai-run-card")].map((card) => card.textContent ?? "");
    expect(filteredCards).toHaveLength(1);
    expect(filteredCards[0]).not.toContain("论文概要");
    expect(filteredCards[0]).toContain("参考文献提取");

    const statusFilter = container.querySelector<HTMLSelectElement>("select[aria-label='运行状态筛选']");
    await act(async () => {
      statusFilter!.value = "running";
      statusFilter!.dispatchEvent(new Event("change", { bubbles: true }));
    });
    expect(container.textContent).toContain("当前筛选下没有 AI 运行记录。");
  });
});
