/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { AgentCanvasDemo } from "../../../app/previews/agent/AgentCanvasDemo";
import { enqueueCanvasMessage } from "../../../app/previews/agent/AgentCanvasDemo.testSupport";
import { notify } from "../../services/notifications";
import { compactRunningMessage } from "./dispatchQueuedAgentCommand";
import { selectAppStorage } from "../../runtime/appStorage";
import { resetWritingCompactionOwnersForTests } from "../../../../../../plugins/writing_projects/src/features/writing/agent/writingCompactionController";

const created = vi.hoisted(() => ({runtimes: [] as unknown[]}));
type Runtime = ReturnType<typeof import("../../../app/previews/agent/writingDiffDemoRuntime")["createWritingDiffDemoRuntime"]>;
vi.mock("../../../app/previews/agent/writingDiffDemoRuntime", async importOriginal => {
  const actual = await importOriginal<typeof import("../../../app/previews/agent/writingDiffDemoRuntime")>();
  return {...actual, createWritingDiffDemoRuntime: (...args: Parameters<typeof actual.createWritingDiffDemoRuntime>) => {
    const runtime = actual.createWritingDiffDemoRuntime(...args); created.runtimes.push(runtime); return runtime;
  }};
});
vi.mock("../../../app/previews/agent/WritingDemoTextStream", async importOriginal => ({
  ...await importOriginal<typeof import("../../../app/previews/agent/WritingDemoTextStream")>(),
  DEFAULT_DEMO_TEXT_PACING: {charactersPerChunk: 1000, intervalMs: 10},
}));
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
async function mount(options: {withFollowups?: boolean} = {}) {
  const values = new Map<string, string>(); created.runtimes.length = 0;
  selectAppStorage({get length() {return values.size;}, key: index => [...values.keys()][index] ?? null, getItem: key => values.get(key) ?? null,
    setItem: (key, value) => {values.set(key, value);}, removeItem: key => {values.delete(key);}, clear: () => values.clear()});
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  await act(async () => root.render(<AgentCanvasDemo/>));
  await vi.waitFor(async () => {await act(async () => Promise.resolve()); expect(host.querySelector('[data-action="stop"]')).not.toBeNull();}, {timeout: 4000});
  if (options.withFollowups) {
    await enqueueCanvasMessage(host, "修改 keywords 部分");
    await enqueueCanvasMessage(host, "按百分比报告进度");
  }
  const runtime = () => created.runtimes.at(-1) as Runtime;
  return {host, runtime, close: async () => {await act(async () => root.unmount()); host.remove(); resetWritingCompactionOwnersForTests(); vi.useRealTimers(); vi.restoreAllMocks();}};
}
async function type(host: HTMLElement, value: string) {
  const input = host.querySelector("textarea")!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(input, value);
    input.setSelectionRange(value.length, value.length); input.dispatchEvent(new Event("input", {bubbles: true}));
  });
  return input;
}
const texts = (host: HTMLElement) => [...host.querySelectorAll(".agent-input-queue-body li p")].map(row => row.textContent);
const status = (host: HTMLElement) => [...host.querySelectorAll('.agent-compaction-activity [role="status"]')].at(-1)?.textContent;
async function click(host: HTMLElement, selector: string) {await act(async () => host.querySelector<HTMLButtonElement>(selector)!.click());}
async function advance(ms: number) {await act(async () => vi.advanceTimersByTimeAsync(ms));}
async function waitStatus(host: HTMLElement, expected: string) {
  await vi.waitFor(async () => {await act(async () => Promise.resolve()); expect(status(host)).toBe(expected);}, {timeout: 5000});
}
async function completeTurn(host: HTMLElement) {
  const duration = host.querySelector<HTMLSelectElement>('[aria-label="模拟运行时长"]')!;
  await act(async () => {duration.value = String(1 / 60); duration.dispatchEvent(new Event("change", {bubbles: true}));});
  await waitStatus(host, "正在压缩上下文");
  // A history event may precede the original start receipt. Wait for admission,
  // then ordinary input can queue while the command continues running.
  await vi.waitFor(async () => {
    await act(async () => Promise.resolve());
    const input = host.querySelector<HTMLTextAreaElement>("textarea")!;
    expect(input.disabled).toBe(false);
    expect(host.querySelector(input.value.trim() ? '[data-action="send"]' : '[data-action="stop"]')).not.toBeNull();
  });
}
async function completedScenario(host: HTMLElement) {
  const scenario = host.querySelector<HTMLSelectElement>('[aria-label="模拟消息场景"]')!;
  await act(async () => {scenario.value = "completed"; scenario.dispatchEvent(new Event("change", {bubbles: true}));});
  await vi.waitFor(async () => {await act(async () => Promise.resolve()); expect(host.textContent).toContain("已完成"); expect(host.querySelector("textarea")!.disabled).toBe(false);});
}

it("keeps the production compaction status pending for the configured preview duration", async () => {
  const view = await mount();
  try {
    await completedScenario(view.host);
    const duration = view.host.querySelector<HTMLInputElement>('[aria-label="模拟压缩时长"]')!;
    await act(async () => {Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(duration, "5"); duration.dispatchEvent(new Event("input", {bubbles: true}));});
    vi.useFakeTimers(); const input = await type(view.host, "/co");
    await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", {key: "Tab", bubbles: true, cancelable: true})));
    await waitStatus(view.host, "正在压缩上下文"); expect(input.value).toBe("");
    await advance(4500); expect(status(view.host)).toBe("正在压缩上下文"); await advance(500);
    await waitStatus(view.host, "上下文已压缩");
  } finally {await view.close();}
});

it("blocks ordinary direct and queued steers during manual compaction while preserving FIFO", async () => {
  const view = await mount();
  try {
    view.runtime().setCompactionDuration(20_000); const start = vi.spyOn(view.runtime().compaction.gateway, "start");
    await type(view.host, "/compact"); await click(view.host, '[data-action="send"]');
    vi.useFakeTimers(); await completeTurn(view.host);
    const before = view.host.querySelectorAll(".writing-agent-message.user").length;
    await type(view.host, "排队的普通消息"); expect(view.host.querySelector<HTMLButtonElement>('[data-action="send"]')!.disabled).toBe(false);
    await click(view.host, '[data-action="send"]'); const input = await type(view.host, "压缩期间的引导");
    await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", ctrlKey: true, bubbles: true, cancelable: true})));
    expect(texts(view.host)).toEqual(["排队的普通消息"]); expect(input.value).toBe("压缩期间的引导");
    const steers = view.host.querySelectorAll<HTMLButtonElement>('[aria-label="将第 1 条消息插入当前轮次"], [aria-label="用第 1 条消息启动新轮次"]');
    expect(steers).toHaveLength(1); const steer = steers[0]!;
    expect(steer.disabled).toBe(true); await act(async () => steer.click()); expect(texts(view.host)).toEqual(["排队的普通消息"]);
    await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", bubbles: true, cancelable: true})));
    expect(texts(view.host)).toEqual(["排队的普通消息", "压缩期间的引导"]); await advance(2000);
    expect(view.host.querySelectorAll(".writing-agent-message.user")).toHaveLength(before); expect(start).toHaveBeenCalledOnce();
    await advance(20_000); await waitStatus(view.host, "上下文已压缩");
    await vi.waitFor(() => expect([...view.host.querySelectorAll(".writing-agent-message.user")].at(-1)?.textContent).toContain("排队的普通消息"));
    expect(texts(view.host)).not.toContain("/compact"); expect(view.host.textContent).not.toContain("压缩完成后发送");
  } finally {await view.close();}
});

it.each([1, 2])("does not resume paused input %i when steering is disabled during manual compaction", async position => {
  const view = await mount({withFollowups: true});
  try {
    await click(view.host, '[data-action="stop"]');
    await vi.waitFor(() => expect(view.host.querySelector('[data-action="stop"]')).toBeNull());
    view.runtime().setCompactionDuration(20_000);
    const start = vi.spyOn(view.runtime().compaction.gateway, "start"), original = texts(view.host);
    vi.useFakeTimers(); await type(view.host, "/compact"); await click(view.host, '[data-action="send"]'); await waitStatus(view.host, "正在压缩上下文");
    const before = view.host.querySelectorAll(".writing-agent-message.user").length;
    const steer = view.host.querySelector<HTMLButtonElement>(`[aria-label="将第 ${position} 条消息插入当前轮次"]`)!;
    expect(steer.disabled).toBe(true); await act(async () => steer.click()); expect(texts(view.host)).toEqual(original);
    await advance(2000); expect(view.host.querySelectorAll(".writing-agent-message.user")).toHaveLength(before); expect(start).toHaveBeenCalledOnce();
    await advance(20_000); await waitStatus(view.host, "上下文已压缩"); expect(texts(view.host)).toEqual(original);
    await click(view.host, `[aria-label="用第 ${position} 条消息启动新轮次"]`);
    await vi.waitFor(() => expect([...view.host.querySelectorAll(".writing-agent-message.user")].at(-1)?.textContent).toContain(original[position - 1]));
    expect(texts(view.host)).toEqual(original.filter((_, index) => index !== position - 1));
  } finally {await view.close();}
});

it("hides an executing queued command, disables steering, and restores it after failure", async () => {
  const view = await mount({withFollowups: true});
  try {
    view.runtime().setCompactionDuration(20_000); view.runtime().compaction.nextOutcome = "failed";
    const prepare = vi.spyOn(view.runtime().compaction.gateway, "prepareWritingQueued");
    await type(view.host, "/compact"); await click(view.host, '[data-action="send"]'); await click(view.host, '[data-action="stop"]');
    await vi.waitFor(() => expect(view.host.querySelector('[data-action="stop"]')).toBeNull());
    expect(texts(view.host)).toEqual(["修改 keywords 部分", "按百分比报告进度", "/compact"]);
    vi.useFakeTimers(); await click(view.host, '[aria-label="用第 3 条消息启动新轮次"]'); await waitStatus(view.host, "正在压缩上下文");
    expect(texts(view.host)).toEqual(["修改 keywords 部分", "按百分比报告进度"]);
    const intent = prepare.mock.calls[0]![0]; expect(intent.submission.requestId).not.toBe(intent.inputId);
    const before = view.host.querySelectorAll(".writing-agent-message.user").length;
    const steers = view.host.querySelectorAll<HTMLButtonElement>('[aria-label="将第 1 条消息插入当前轮次"], [aria-label="用第 1 条消息启动新轮次"]');
    expect(steers).toHaveLength(1); const steer = steers[0]!;
    expect(steer.disabled).toBe(true); await act(async () => steer.click()); expect(view.host.querySelectorAll(".writing-agent-message.user")).toHaveLength(before);
    await advance(20_000); await waitStatus(view.host, "上下文压缩失败");
    await vi.waitFor(() => expect(texts(view.host)).toEqual(["/compact", "修改 keywords 部分", "按百分比报告进度"]));
    expect(view.host.querySelector(".agent-input-queue-deferred")).toBeNull(); expect(view.host.textContent).not.toContain("压缩完成后发送");
  } finally {await view.close();}
});

it("retains a blocked steer draft and pauses queued ordinary input if compaction fails", async () => {
  const view = await mount();
  try {
    view.runtime().setCompactionDuration(20_000); view.runtime().compaction.nextOutcome = "failed";
    await type(view.host, "/compact"); await click(view.host, '[data-action="send"]'); vi.useFakeTimers(); await completeTurn(view.host);
    const before = view.host.querySelectorAll(".writing-agent-message.user").length, input = await type(view.host, "压缩失败也不能丢失");
    await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", ctrlKey: true, bubbles: true, cancelable: true})));
    expect(input.value).toBe("压缩失败也不能丢失"); await click(view.host, '[data-action="send"]'); await type(view.host, "保留草稿");
    await advance(20_000); await waitStatus(view.host, "上下文压缩失败");
    await vi.waitFor(() => expect(texts(view.host)).toEqual(["/compact", "压缩失败也不能丢失"]));
    expect(view.host.querySelectorAll(".writing-agent-message.user")).toHaveLength(before); expect(input.value).toBe("保留草稿");
  } finally {await view.close();}
});

it("queues /compact behind ordinary inputs and refuses running command steering without removing it", async () => {
  const view = await mount({withFollowups: true}), error = vi.spyOn(notify, "error").mockReturnValue(1);
  try {
    vi.useFakeTimers(); const before = view.host.querySelector(".writing-agent-transcript")!.textContent;
    const input = await type(view.host, "/compact"); await click(view.host, '[data-action="send"]');
    expect(texts(view.host)).toEqual(["修改 keywords 部分", "按百分比报告进度", "/compact"]); expect(input.value).toBe(""); expect(status(view.host)).toBeUndefined();
    await click(view.host, '[aria-label="将第 3 条消息插入当前轮次"]'); expect(error).toHaveBeenLastCalledWith(compactRunningMessage);
    expect(texts(view.host)).toHaveLength(3); expect(view.host.querySelector(".writing-agent-transcript")!.textContent).toBe(before); expect(view.host.querySelector('[data-action="stop"]')).not.toBeNull();
  } finally {await view.close();}
});

it("Tab queues a partial command, then FIFO executes it after the active turn without sending a user message", async () => {
  const view = await mount();
  try {
    view.runtime().setCompactionDuration(20_000); vi.useFakeTimers();
    const input = await type(view.host, "/co"); await advance(20);
    await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", {key: "Tab", bubbles: true, cancelable: true})));
    expect(texts(view.host)).toEqual(["/compact"]); expect(input.value).toBe(""); expect(status(view.host)).toBeUndefined();
    await type(view.host, "保留的新草稿"); await completeTurn(view.host);
    // The shared composer shows send while a draft exists; the admitted
    // history still identifies this as a compact task, without a user turn.
    const history = await view.runtime().history.gateway.readHistory({conversationId: "00000000-0000-4000-8000-000000000001", afterSeq: 0, limit: 1000});
    expect(history.events.some(event => event.payload.type === "turnStarted" && event.payload.taskKind === "compact")).toBe(true);
    await advance(20_000); await waitStatus(view.host, "上下文已压缩"); await vi.waitFor(() => expect(texts(view.host)).toEqual([]));
    expect(input.value).toBe("保留的新草稿"); expect([...view.host.querySelectorAll(".writing-agent-message.user")].some(row => row.textContent?.includes("/compact"))).toBe(false);
    expect(view.host.querySelector<HTMLElement>(".writing-agent-context-ring")!.style.getPropertyValue("--writing-agent-context-progress")).toBe("50.4deg");
  } finally {await view.close();}
});

it("keeps a failed queued command and the newer draft, and permits an explicit retry while idle", async () => {
  const view = await mount();
  try {
    view.runtime().setCompactionDuration(20_000); view.runtime().compaction.nextOutcome = "failed";
    const prepare = vi.spyOn(view.runtime().compaction.gateway, "prepareWritingQueued");
    await type(view.host, "/compact"); await click(view.host, '[data-action="send"]'); await type(view.host, "不要覆盖这个草稿");
    vi.useFakeTimers(); await completeTurn(view.host); await advance(20_000); await waitStatus(view.host, "上下文压缩失败");
    await vi.waitFor(() => expect(texts(view.host)).toEqual(["/compact"])); expect(view.host.querySelector("textarea")!.value).toBe("不要覆盖这个草稿");
    const firstIntent = prepare.mock.calls[0]![0];
    await vi.waitFor(async () => expect(await view.runtime().compaction.gateway.readWritingQueued({conversationId: firstIntent.submission.conversationId,
      manuscriptId: firstIntent.submission.context.manuscriptId, inputId: firstIntent.inputId, requestId: null})).toMatchObject({kind: "compact", current: false, outcome: "failed"}));
    await advance(1500); expect(prepare).toHaveBeenCalledOnce(); await click(view.host, '[aria-label="用第 1 条消息启动新轮次"]');
    await waitStatus(view.host, "正在压缩上下文"); const first = prepare.mock.calls[0]![0], second = prepare.mock.calls[1]![0];
    expect(second.inputId).toBe(first.inputId); expect(second.previousRequestId).toBe(first.submission.requestId); expect(second.submission.requestId).not.toBe(first.submission.requestId);
    await advance(20_000); await waitStatus(view.host, "上下文已压缩");
    expect([...view.host.querySelectorAll('.agent-compaction-activity [role="status"]')].map(row => row.textContent)).toEqual(["上下文压缩失败", "上下文已压缩"]);
    await vi.waitFor(() => expect(texts(view.host)).toEqual([])); expect(view.host.querySelector("textarea")!.value).toBe("不要覆盖这个草稿");
  } finally {await view.close();}
});
