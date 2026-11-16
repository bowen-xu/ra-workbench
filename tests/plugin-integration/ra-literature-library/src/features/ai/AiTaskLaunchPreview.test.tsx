/** @vitest-environment happy-dom */
import { notify } from "@research-assistant/workbench-sdk/feedback";
vi.mock("../../../../../../apps/web/src/core/services/notifications", () => ({ notify: { error: vi.fn(), info: vi.fn(), success: vi.fn(), warning: vi.fn() } }));

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { AiRun, AiRunStatus, AiTaskPreview, AiTaskSpecDraft } from "@research-assistant/core/ai";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AiAssistanceGateway } from "../../../../../../apps/web/src/core/services/aiAssistanceGateway";
import { AiTaskLaunchPreview } from "../../../../../../../plugins/ra-literature-library/src/features/ai/AiTaskLaunchPreview";

const draft: AiTaskSpecDraft = {
  taskKind: "reference_extraction",
  providerId: "deepseek",
  userGoal: "Extract one bounded synthetic reference.",
  pipelineVersion: "test-pipeline-v1",
  promptVersion: "test-prompt-v1",
  outputSchemaVersion: 1,
  maxInputTokens: 512,
  maxOutputTokens: 128,
  timeoutMs: 10_000,
  scopeItems: [{ itemKind: "anchor", objectId: "anchor-1" }],
};

const preview = {
  providerId: "deepseek",
  modelId: "deepseek-v4-flash",
  requiresCloudConsent: true,
  scopeHash: "a".repeat(64),
  evidencePreview: [{
    citationKey: "ev-1", relationKind: "direct_quote", quoteExact: "Visible synthetic evidence.",
    contextText: "Visible synthetic evidence.", paperTitle: "Synthetic paper",
    pageIndex: 0, pageLabel: "1", contentSha256: "b".repeat(64),
  }],
} as unknown as AiTaskPreview;

function button(container: HTMLElement, label: string): HTMLButtonElement {
  const match = [...container.querySelectorAll("button")].find((item) => item.textContent === label);
  if (match === undefined) throw new Error(`missing button ${label}`);
  return match;
}

function run(status: AiRunStatus, id = "run-1"): AiRun {
  return {
    id,
    taskKind: "reference_extraction",
    userGoal: "Extract one bounded synthetic reference.",
    status,
    providerId: "deepseek",
    modelId: "deepseek-v4-flash",
    pipelineVersion: "test-pipeline-v1",
    promptVersion: "test-prompt-v1",
    outputSchemaVersion: 1,
    maxInputTokens: 512,
    maxOutputTokens: 128,
    timeoutMs: 10_000,
    scopeHash: "a".repeat(64),
    inputItemCount: 1,
    scopeItems: [],
    stages: [],
    cancelRequested: false,
    createdAt: "2026-08-09T00:00:00.000Z",
    updatedAt: "2026-08-09T00:00:00.000Z",
  };
}

describe("AI cloud launch privacy gate", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean })
      .IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    vi.useRealTimers();
    await act(async () => root.unmount());
    container.remove();
  });

  it("shows the exact outbound evidence and requires fresh consent before starting", async () => {
    const startRun = vi.fn(async () => run("needs_review"));
    const gateway = {
      listProviders: vi.fn(async () => [{
        settings: {
          providerId: "deepseek", providerKind: "generation", modelId: "deepseek-v4-flash",
          enabled: true,
        },
      }]),
      previewTask: vi.fn(async () => preview),
      startRun,
    } as unknown as AiAssistanceGateway;
    const onStarted = vi.fn();
    await act(async () => root.render(<AiTaskLaunchPreview
      description="No research content beyond the visible scope."
      gateway={gateway}
      heading="Bounded extraction"
      onCancel={() => undefined}
      onStarted={onStarted}
      prepareDraft={async () => draft}
      startLabel="启动候选"
      taskKind="reference_extraction"
    />));
    await act(async () => undefined);
    await act(async () => button(container, "生成外发预览").click());

    expect(container.textContent).toContain("Visible synthetic evidence.");
    expect(container.textContent).toContain("我确认仅将以上可见证据发送到该云端 provider");
    expect(button(container, "启动候选").disabled).toBe(true);
    expect(startRun).not.toHaveBeenCalled();

    const consent = container.querySelector<HTMLInputElement>('.ai-cloud-consent input[type="checkbox"]');
    await act(async () => {
      consent?.click();
    });
    expect(button(container, "启动候选").disabled).toBe(false);
    await act(async () => button(container, "启动候选").click());

    expect(startRun).toHaveBeenCalledWith(expect.objectContaining({
      confirmedScopeHash: preview.scopeHash,
      cloudConsentVersion: "ai-cloud-consent-v1",
    }));
    expect(onStarted).toHaveBeenCalledWith(expect.objectContaining({ id: "run-1" }));
  });

  it("tracks an active run in place until candidates are ready", async () => {
    vi.useFakeTimers();
    const startRun = vi.fn(async () => run("queued"));
    const getRun = vi.fn(async () => run("needs_review"));
    const gateway = {
      listProviders: vi.fn(async () => [{
        settings: {
          providerId: "deepseek", providerKind: "generation", modelId: "deepseek-v4-flash",
          enabled: true,
        },
      }]),
      previewTask: vi.fn(async () => preview),
      startRun,
      getRun,
    } as unknown as AiAssistanceGateway;
    const onStarted = vi.fn();
    await act(async () => root.render(<AiTaskLaunchPreview
      description="No research content beyond the visible scope."
      gateway={gateway}
      heading="Bounded extraction"
      onCancel={() => undefined}
      onStarted={onStarted}
      prepareDraft={async () => draft}
      startLabel="启动候选"
      taskKind="reference_extraction"
      trackRunInPlace
    />));
    await act(async () => undefined);
    await act(async () => button(container, "生成外发预览").click());
    const consent = container.querySelector<HTMLInputElement>('.ai-cloud-consent input[type="checkbox"]');
    await act(async () => {
      consent?.click();
    });
    await act(async () => button(container, "启动候选").click());

    expect(onStarted).not.toHaveBeenCalled();
    expect(container.textContent).toContain("AI 正在运行；完成后会保存为 AI 候选记录。");
    expect(button(container, "生成外发预览").disabled).toBe(true);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(700);
    });

    expect(getRun).toHaveBeenCalledWith("run-1");
    expect(onStarted).toHaveBeenCalledWith(expect.objectContaining({ status: "needs_review" }));
  });

  it("can request cancellation for an in-place active run", async () => {
    const startRun = vi.fn(async () => run("queued"));
    const cancelRun = vi.fn(async () => true);
    const gateway = {
      listProviders: vi.fn(async () => [{
        settings: {
          providerId: "deepseek", providerKind: "generation", modelId: "deepseek-v4-flash",
          enabled: true,
        },
      }]),
      previewTask: vi.fn(async () => preview),
      startRun,
      cancelRun,
    } as unknown as AiAssistanceGateway;
    const onStarted = vi.fn();
    await act(async () => root.render(<AiTaskLaunchPreview
      description="No research content beyond the visible scope."
      gateway={gateway}
      heading="Bounded extraction"
      onCancel={() => undefined}
      onStarted={onStarted}
      prepareDraft={async () => draft}
      startLabel="启动候选"
      taskKind="reference_extraction"
      trackRunInPlace
    />));
    await act(async () => undefined);
    await act(async () => button(container, "生成外发预览").click());
    const consent = container.querySelector<HTMLInputElement>('.ai-cloud-consent input[type="checkbox"]');
    await act(async () => {
      consent?.click();
    });
    await act(async () => button(container, "启动候选").click());
    await act(async () => button(container, "取消运行").click());

    expect(cancelRun).toHaveBeenCalledWith("run-1");
    expect(onStarted).not.toHaveBeenCalled();
    expect(notify.info).toHaveBeenCalledWith(expect.stringContaining("已请求取消 AI 运行（run-1…）"));
  });
  it.each([
    ["start", "data_operation_outcome_unknown"],
    ["start", "ai_run_terminal_outcome_unknown"],
    ["poll", "data_operation_outcome_unknown"],
    ["poll", "ai_run_terminal_outcome_unknown"],
  ] as const)("retains %s uncertainty for %s and blocks repeat actions", async (phase, code) => {
    vi.useFakeTimers();
    const reason = { code, message: "Result unconfirmed for original run-1, evidence_bundle, commit" };
    const startRun = vi.fn(async () => {
      if (phase === "start") throw reason;
      return run("queued");
    });
    const getRun = vi.fn(async () => { throw reason; });
    const cancelRun = vi.fn(async () => true);
    const prepareDraft = vi.fn(async () => draft);
    const gateway = {
      listProviders: vi.fn(async () => [{ settings: {
        providerId: "deepseek", providerKind: "generation", modelId: "deepseek-v4-flash", enabled: true,
      } }]),
      previewTask: vi.fn(async () => preview), startRun, getRun, cancelRun,
    } as unknown as AiAssistanceGateway;
    const onStarted = vi.fn();
    await act(async () => root.render(<AiTaskLaunchPreview
      description="Bounded selected text."
      gateway={gateway}
      heading="Reference extraction"
      onCancel={() => undefined}
      onStarted={onStarted}
      prepareDraft={prepareDraft}
      startLabel="启动候选"
      taskKind="reference_extraction"
      trackRunInPlace
    />));
    await act(async () => button(container, "生成外发预览").click());
    await act(async () => container.querySelector<HTMLInputElement>('.ai-cloud-consent input')!.click());
    await act(async () => button(container, "启动候选").click());
    if (phase === "poll") await act(async () => { await vi.advanceTimersByTimeAsync(700); });
    expect(container.textContent).toContain(code);
    expect(container.textContent).toContain("run-1");
    expect(container.textContent).not.toContain("取消运行");
    expect(button(container, "返回人工操作").disabled).toBe(true);
    expect(button(container, phase === "start" ? "启动候选" : "生成外发预览").disabled).toBe(true);
    await act(async () => { await vi.advanceTimersByTimeAsync(10_000); });
    expect(startRun).toHaveBeenCalledTimes(1);
    expect(getRun).toHaveBeenCalledTimes(phase === "poll" ? 1 : 0);
    expect(cancelRun).not.toHaveBeenCalled();
    expect(onStarted).not.toHaveBeenCalled();
  });

});
