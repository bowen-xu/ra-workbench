// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AiAssistanceGateway } from "../../../../../../apps/web/src/core/services/aiAssistanceGateway";
import { getPaperAnalysisActivity } from "../../../../../../../plugins/ra-literature-library/src/features/ai/paperAnalysisActivity";
import { workflow } from "../../../../../../../plugins/ra-literature-library/src/features/ai/paperOverviewTestFixtures";

afterEach(() => vi.useRealTimers());

describe("paper analysis activity ownership", () => {
  it("keeps one request through panel lifetimes and shares cancellation across both rails", async () => {
    const cancelPaperAnalysis = vi.fn(async () => true);
    const gateway = { cancelPaperAnalysis } as unknown as AiAssistanceGateway;
    const outer = getPaperAnalysisActivity(gateway, "paper");
    const listener = vi.fn();
    const unsubscribe = outer.subscribe(listener);
    const request = outer.begin()!;
    outer.phase(request, "parsing");
    unsubscribe();
    const inner = getPaperAnalysisActivity(gateway, "paper");
    expect(inner.getSnapshot()).toMatchObject({ phase: "parsing", requestId: request });
    expect(inner.begin()).toBeUndefined();
    expect(getPaperAnalysisActivity(gateway, "other").getSnapshot().phase).toBe("idle");
    await expect(inner.cancel()).resolves.toBe(true);
    expect(outer.isCanceled(request)).toBe(true);
    expect(cancelPaperAnalysis).toHaveBeenCalledExactlyOnceWith(request);
    await inner.cancel();
    expect(cancelPaperAnalysis).toHaveBeenCalledOnce();
    inner.finish(request);
    const next = inner.begin()!;
    outer.finish(request); // A stale completion cannot clear the next task.
    expect(inner.getSnapshot().requestId).toBe(next);
    inner.finish(next);
  });

  it("recovers the native cancel identity and refreshes terminal state without starting work", async () => {
    vi.useFakeTimers();
    const latest = vi.fn(async () => workflow("needs_review"));
    const gateway = { getLatestPaperAnalysisWorkflow: latest,
      cancelPaperAnalysis: vi.fn(async () => true) } as unknown as AiAssistanceGateway;
    const owner = getPaperAnalysisActivity(gateway, "paper");
    const stop = owner.subscribe(vi.fn());
    owner.adopt({ ...workflow("overview_running"), activeClientRequestId: "native-request" });
    expect(owner.getSnapshot().requestId).toBe("native-request");
    await owner.cancel();
    expect(gateway.cancelPaperAnalysis).toHaveBeenCalledWith("native-request");
    await vi.advanceTimersByTimeAsync(1000);
    expect(owner.getSnapshot().phase).toBe("idle");
    expect(latest).toHaveBeenCalledOnce();
    stop();
  });

  it("does not treat an unavailable native observation as task completion", async () => {
    vi.useFakeTimers();
    const latest = vi.fn().mockRejectedValueOnce(new Error("IPC unavailable"))
      .mockResolvedValueOnce(workflow("needs_review"));
    const gateway = { getLatestPaperAnalysisWorkflow: latest } as unknown as AiAssistanceGateway;
    const owner = getPaperAnalysisActivity(gateway, "paper");
    const stop = owner.subscribe(vi.fn());
    owner.adopt({ ...workflow("overview_running"), activeClientRequestId: "native-request" });
    await vi.advanceTimersByTimeAsync(1000);
    expect(owner.getSnapshot().phase).toBe("extracting");
    await vi.advanceTimersByTimeAsync(1000);
    expect(owner.getSnapshot().phase).toBe("idle");
    stop();
  });

  it("retains an unknown outcome after cancellation and stale completion", async () => {
    const gateway = { cancelPaperAnalysis: vi.fn(async () => true) } as unknown as AiAssistanceGateway;
    const owner = getPaperAnalysisActivity(gateway, "paper");
    const request = owner.begin()!;
    await owner.cancel();
    owner.unknown(request, "原运行 run-1 待核对");
    owner.phase(request, "extracting");
    owner.finish(request);
    expect(owner.getSnapshot()).toMatchObject({ phase: "unknown", requestId: request,
      cancelRequested: false, error: "原运行 run-1 待核对" });
    expect(getPaperAnalysisActivity(gateway, "paper").begin()).toBeUndefined();
    await expect(owner.cancel()).resolves.toBeUndefined();
    expect(gateway.cancelPaperAnalysis).toHaveBeenCalledOnce();
  });

  it("does not replace an unknown outcome when a pending cancellation fails", async () => {
    let reject!: (reason: Error) => void;
    const pending = new Promise<boolean>((_, fail) => { reject = fail; });
    const gateway = { cancelPaperAnalysis: vi.fn(() => pending) } as unknown as AiAssistanceGateway;
    const owner = getPaperAnalysisActivity(gateway, "paper");
    const request = owner.begin()!;
    const cancellation = owner.cancel();
    owner.unknown(request, "原运行待核对");
    reject(new Error("cancel unavailable"));
    await expect(cancellation).rejects.toThrow("cancel unavailable");
    expect(owner.getSnapshot()).toMatchObject({ phase: "unknown", error: "原运行待核对" });
    owner.finish(request);
    expect(owner.begin()).toBeUndefined();
  });
});
