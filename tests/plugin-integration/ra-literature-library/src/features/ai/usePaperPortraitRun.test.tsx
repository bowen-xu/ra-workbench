// @vitest-environment happy-dom
import { act, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { AiRun } from "@research-assistant/core/ai";
import type { AiAssistanceGateway } from "../../../../../../apps/web/src/core/services/aiAssistanceGateway";
import { notify } from "@research-assistant/workbench-sdk/feedback";
import { subscribePaperReadingChanged } from "../../../../../../../plugins/ra-literature-library/src/features/ai/readingPanelEvents";
import { usePaperPortraitRun } from "../../../../../../../plugins/ra-literature-library/src/features/ai/usePaperPortraitRun";

vi.mock("../../../../../../apps/web/src/core/services/notifications", () => ({ notify: { success: vi.fn(), info: vi.fn(), error: vi.fn() } }));

function run(status: AiRun["status"] = "generating"): AiRun {
  return { id: "run", taskKind: "paper_profile_classification", status, providerId: "fixture", modelId: "fixture",
    pipelineVersion: "v1", promptVersion: "v1", outputSchemaVersion: 1, userGoal: "portrait", scopeHash: "a".repeat(64),
    inputItemCount: 1, maxInputTokens: 100, maxOutputTokens: 100, timeoutMs: 120000, cancelRequested: false,
    createdAt: "now", updatedAt: "now", scopeItems: [], stages: [], inputTokens: 0, outputTokens: 0 };
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}
let roots: Root[], hosts: HTMLDivElement[];
let gateway: AiAssistanceGateway;
let prepare: ReturnType<typeof vi.fn<() => Promise<AiRun>>>;
function View({ paper = "paper", adapter = gateway }: { paper?: string; adapter?: AiAssistanceGateway }) {
  const task = usePaperPortraitRun(adapter, paper);
  return <div>
    <span data-conversation>{task.conversationId}</span>
    <span data-error>{task.error}</span>
    <output>{task.preparing ? "preparing" : task.runId ? `${task.canceling ? "canceling" : "running"}:${task.runId}` : "idle"}{task.error ? ":unavailable" : ""}</output>
    <button onClick={() => void task.start(prepare)}>start</button>
    <button onClick={() => void task.cancel()}>cancel</button>
    <button onClick={task.retry}>retry</button>
  </div>;
}
async function mount(paper = "paper", adapter = gateway) {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host); roots.push(root); hosts.push(host);
  await act(async () => root.render(<StrictMode><View paper={paper} adapter={adapter}/></StrictMode>));
  return { host, root };
}
async function click(host: HTMLElement, label: string) {
  await act(async () => [...host.querySelectorAll("button")].find(button => button.textContent === label)!.click());
}
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); vi.useFakeTimers(); vi.clearAllMocks();
  roots = []; hosts = [];
  prepare = vi.fn(async () => run());
  gateway = { getRun: vi.fn(async () => run()), cancelRun: vi.fn(async () => true) } as unknown as AiAssistanceGateway;
});
afterEach(async () => {
  for (const root of roots) await act(async () => root.unmount());
  hosts.forEach(host => host.remove()); vi.useRealTimers(); vi.unstubAllGlobals();
});

it("shares preparation, generation and cancellation across both rails without duplicate requests", async () => {
  const waiting = deferred<AiRun>(); prepare.mockReturnValue(waiting.promise);
  const library = await mount(), reader = await mount();
  await click(library.host, "start");
  expect(reader.host.querySelector("output")!.textContent).toBe("preparing");
  await click(reader.host, "start"); expect(prepare).toHaveBeenCalledOnce();
  await act(async () => waiting.resolve(run()));
  expect(library.host.querySelector("output")!.textContent).toBe("running:run");
  expect(reader.host.querySelector("output")!.textContent).toBe("running:run");
  expect(gateway.getRun).toHaveBeenCalledOnce();
  await click(reader.host, "cancel");
  expect(library.host.querySelector("output")!.textContent).toBe("canceling:run");
  await click(library.host, "cancel");
  expect(gateway.cancelRun).toHaveBeenCalledExactlyOnceWith("run");
  vi.mocked(gateway.getRun).mockResolvedValue(run("canceled"));
  await act(async () => vi.advanceTimersByTimeAsync(800));
  expect(reader.host.querySelector("output")!.textContent).toBe("idle");
  expect(library.host.querySelector("output")!.textContent).toBe("idle");
  expect(notify.info).toHaveBeenCalledOnce();
});

it("retains a pending start and active run across complete navigation, polling only while mounted", async () => {
  const waiting = deferred<AiRun>(); prepare.mockReturnValue(waiting.promise);
  const library = await mount(); await click(library.host, "start");
  await act(async () => library.root.render(null));
  await act(async () => waiting.resolve(run()));
  expect(gateway.getRun).not.toHaveBeenCalled();
  const reader = await mount();
  expect(reader.host.querySelector("output")!.textContent).toBe("running:run");
  expect(prepare).toHaveBeenCalledOnce();
  await act(async () => reader.root.render(null));
  const count = vi.mocked(gateway.getRun).mock.calls.length;
  await act(async () => vi.advanceTimersByTimeAsync(5000));
  expect(gateway.getRun).toHaveBeenCalledTimes(count);
  const returning = await mount();
  expect(returning.host.querySelector("output")!.textContent).toBe("running:run");
  // StrictMode deliberately unsubscribes and resubscribes once on remount.
  expect(gateway.getRun).toHaveBeenCalledTimes(count + 2);
  await act(async () => vi.advanceTimersByTimeAsync(800));
  expect(gateway.getRun).toHaveBeenCalledTimes(count + 3);
});

it.each(["needs_review", "partial", "failed", "canceled"] as const)("refreshes both rails once on %s", async status => {
  const changed = vi.fn(), unlisten = subscribePaperReadingChanged("paper", changed, "test");
  try {
    const library = await mount(), reader = await mount();
    await click(library.host, "start");
    vi.mocked(gateway.getRun).mockResolvedValue(run(status));
    await act(async () => vi.advanceTimersByTimeAsync(800));
    expect(changed).toHaveBeenCalledOnce();
    expect(library.host.querySelector("output")!.textContent).toBe("idle");
    expect(reader.host.querySelector("output")!.textContent).toBe("idle");
    expect(vi.mocked(notify.success).mock.calls.length + vi.mocked(notify.info).mock.calls.length).toBe(1);
  } finally { unlisten(); }
});

it("retains task identity after a status read failure and allows explicit retry", async () => {
  vi.mocked(gateway.getRun).mockRejectedValueOnce(new Error("offline"));
  const library = await mount(); await click(library.host, "start");
  const reader = await mount();
  expect(reader.host.querySelector("output")!.textContent).toBe("running:run:unavailable");
  await click(reader.host, "start"); expect(prepare).toHaveBeenCalledOnce();
  await click(reader.host, "retry");
  expect(library.host.querySelector("output")!.textContent).toBe("running:run");
  expect(notify.error).toHaveBeenCalledOnce();
});

it("ignores stale terminal responses after navigation and isolates papers and runtime adapters", async () => {
  const old = deferred<AiRun>(); vi.mocked(gateway.getRun).mockReturnValueOnce(old.promise);
  const library = await mount(); await click(library.host, "start");
  await act(async () => library.root.render(null));
  const reader = await mount(), otherPaper = await mount("other");
  const otherRuntime = await mount("paper", { ...gateway });
  await act(async () => old.resolve(run("needs_review")));
  expect(reader.host.querySelector("output")!.textContent).toBe("running:run");
  expect(otherPaper.host.querySelector("output")!.textContent).toBe("idle");
  expect(otherRuntime.host.querySelector("output")!.textContent).toBe("idle");
  expect(notify.success).not.toHaveBeenCalled();
});

it("unlocks both rails when preparation fails", async () => {
  prepare.mockRejectedValueOnce(new Error("no source"));
  const library = await mount(), reader = await mount();
  await click(library.host, "start");
  expect(library.host.querySelector("output")!.textContent).toBe("idle");
  expect(reader.host.querySelector("output")!.textContent).toBe("idle");
  await click(reader.host, "start");
  expect(library.host.querySelector("output")!.textContent).toBe("running:run");
});

it.each(["data_operation_outcome_unknown", "ai_run_terminal_outcome_unknown"])("retains %s at start across rails and navigation without retrying", async (code) => {
  const message = "AI 运行创建结果尚未确认，请重启应用后核对原运行 12345678-1234-1234-1234-123456789abc";
  prepare.mockRejectedValueOnce({ code, message });
  const library = await mount(), reader = await mount();
  await click(library.host, "start");
  expect(library.host.querySelector("output")!.textContent).toBe("idle:unavailable");
  expect(reader.host.querySelector("output")!.textContent).toBe("idle:unavailable");
  expect(reader.host.querySelector("[data-error]")!.textContent).toContain(message);
  expect(notify.error).toHaveBeenCalledWith(expect.stringContaining(message));
  await click(reader.host, "retry");
  await click(reader.host, "start");
  expect(prepare).toHaveBeenCalledOnce();
  await act(async () => { library.root.render(null); reader.root.render(null); });
  const returning = await mount();
  expect(returning.host.querySelector("output")!.textContent).toBe("idle:unavailable");
  expect(returning.host.querySelector("[data-error]")!.textContent).toContain(message);
  await click(returning.host, "start");
  expect(prepare).toHaveBeenCalledOnce();
});

it.each(["data_operation_outcome_unknown", "ai_run_terminal_outcome_unknown"])("retains %s across rails without retrying or canceling", async (code) => {
  const message = "AI 运行 run 的终态结果尚未确认，请重启后核对原运行";
  vi.mocked(gateway.getRun).mockRejectedValueOnce({code, message});
  const library = await mount(), reader = await mount();
  await click(library.host, "start");
  expect(reader.host.querySelector("[data-error]")!.textContent).toContain(message);
  expect(notify.error).toHaveBeenCalledWith(expect.stringContaining(message));
  const reads = vi.mocked(gateway.getRun).mock.calls.length;
  await click(reader.host, "retry"); await click(reader.host, "cancel"); await click(reader.host, "start");
  expect(gateway.getRun).toHaveBeenCalledTimes(reads);
  expect(gateway.cancelRun).not.toHaveBeenCalled(); expect(prepare).toHaveBeenCalledOnce();
  await act(async () => {library.root.render(null); reader.root.render(null);});
  const returning = await mount();
  expect(returning.host.querySelector("[data-error]")!.textContent).toContain(message);
  expect(returning.host.querySelector("output")!.textContent).toBe("running:run:unavailable");
});

it.each(["data_operation_outcome_unknown", "ai_run_terminal_outcome_unknown"])("does not let a late cancellation acknowledgment clear %s", async (code) => {
  const cancellation = deferred<boolean>();
  vi.mocked(gateway.cancelRun).mockReturnValueOnce(cancellation.promise);
  const view = await mount();
  await click(view.host, "start"); await click(view.host, "cancel");
  vi.mocked(gateway.getRun).mockRejectedValueOnce({code, message:"原运行终态尚未确认"});
  await act(async () => vi.advanceTimersByTimeAsync(800));
  expect(view.host.querySelector("[data-error]")!.textContent).toContain("原运行终态尚未确认");
  await act(async () => cancellation.resolve(true));
  expect(view.host.querySelector("[data-error]")!.textContent).toContain("原运行终态尚未确认");
  expect(view.host.querySelector("output")!.textContent).toBe("running:run:unavailable");
});

it("recovers the paper's saved conversation without announcing an old result", async () => {
  gateway.getLatestPaperPortraitRun = vi.fn(async paperId => paperId === "paper"
    ? {...run("canceled"), sharedConversationId: "saved-conversation"} : undefined);
  const library = await mount(), other = await mount("other");
  expect(library.host.querySelector("[data-conversation]")!.textContent).toBe("saved-conversation");
  expect(other.host.querySelector("[data-conversation]")!.textContent).toBe("");
  expect(gateway.getRun).not.toHaveBeenCalled();
  expect(notify.info).not.toHaveBeenCalled();
  prepare.mockResolvedValue({...run(), sharedConversationId: "saved-conversation"});
  await click(library.host, "start");
  vi.mocked(gateway.getRun).mockResolvedValue(run("needs_review"));
  await act(async () => vi.advanceTimersByTimeAsync(800));
  expect(library.host.querySelector("[data-conversation]")!.textContent).toBe("saved-conversation");
  await click(library.host, "start");
  expect(prepare).toHaveBeenCalledTimes(2);
  expect(library.host.querySelector("[data-conversation]")!.textContent).toBe("saved-conversation");
});

it("recovers an active run for both rails and blocks duplicate generation until lookup succeeds", async () => {
  const lookup = deferred<AiRun>();
  gateway.getLatestPaperPortraitRun = vi.fn(() => lookup.promise);
  const library = await mount(), reader = await mount();
  await click(library.host, "start"); expect(prepare).not.toHaveBeenCalled();
  await act(async () => lookup.resolve({...run(), sharedConversationId: "active-conversation"}));
  for (const view of [library, reader]) {
    expect(view.host.querySelector("output")!.textContent).toBe("running:run");
    expect(view.host.querySelector("[data-conversation]")!.textContent).toBe("active-conversation");
  }
  expect(gateway.getLatestPaperPortraitRun).toHaveBeenCalledOnce();
  await click(reader.host, "start"); expect(prepare).not.toHaveBeenCalled();
  await click(reader.host, "cancel");
  expect(gateway.cancelRun).toHaveBeenCalledExactlyOnceWith("run");
});

it("does not start an unknown duplicate after recovery fails, and supports retry", async () => {
  gateway.getLatestPaperPortraitRun = vi.fn().mockRejectedValueOnce(new Error("offline"))
    .mockResolvedValue({...run("failed"), sharedConversationId: "restored"});
  const view = await mount();
  await click(view.host, "start"); expect(prepare).not.toHaveBeenCalled();
  await click(view.host, "retry");
  expect(view.host.querySelector("[data-conversation]")!.textContent).toBe("restored");
  await click(view.host, "start"); expect(prepare).toHaveBeenCalledOnce();
});
