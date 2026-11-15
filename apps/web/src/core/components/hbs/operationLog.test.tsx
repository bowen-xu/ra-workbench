// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createTextDocumentAdapter, HbsSession, type DocumentSnapshot } from "@research-assistant/core/hbs";
import { directTextDiff } from "./codeMirrorHbsPolicy";
import { HbsDeliveryPort } from "./deliveryPort";
import { HbsOperationLog } from "./operationLog";
import { HbsAdapterWorkspace } from "./HbsAdapterWorkspace";
import { HbsOperationLogPanel } from "./HbsOperationLogPanel";

const feedback = vi.hoisted(() => ({success: vi.fn(), error: vi.fn(), info: vi.fn(), save: vi.fn().mockResolvedValue("saved")}));
vi.mock("../../services/notifications", () => ({notify: feedback}));
vi.mock("../../features/notes/transfer/saveNoteRecoveryFile", () => ({saveNoteRecoveryFile: feedback.save}));

let root: ReturnType<typeof createRoot> | undefined, host: HTMLDivElement;
afterEach(async () => {if (root) await act(async () => root!.unmount()); root = undefined; host?.remove(); vi.unstubAllGlobals(); vi.clearAllMocks();});
async function fixture() {
  let current: DocumentSnapshot<string> = {version: 1, saved: "initial", baseline: "initial"};
  const actual = {read: async () => current, commit: vi.fn(async (request: {saved: string; baseline: string}) => {
    current = {version: current.version + 1, saved: request.saved, baseline: request.baseline}; return current;
  })};
  const session = await HbsSession.open({adapter: createTextDocumentAdapter(directTextDiff), port: actual,
    preservation: {load: async () => undefined, save: async () => {}}, requestId: n => `test:${n}`});
  return {session, actual};
}
it("exports an initial context and exact H/B′ deltas without causing saves or accepting candidates", async () => {
  const f = await fixture(), log = new HbsOperationLog(), stop = log.observe(f.session, "text");
  f.session.edit("typed text");
  const entries = JSON.parse(log.exportJson()).entries;
  expect(entries[0].detail.initial.observed).toEqual({version: 1, saved: "initial", baseline: "initial"});
  const delta = entries.find((entry: {label: string}) => entry.label === "模型状态变化").detail;
  expect(delta.contentChanges[0].after).toBe("typed text"); expect(delta.baselineChanges[0].after).toBe("typed text");
  expect(f.actual.commit).not.toHaveBeenCalled(); expect(f.session.getSnapshot().changes).toHaveLength(0);
  stop(); f.session.dispose(); log.record("开始重置");
  expect(log.getSnapshot().map(entry => entry.sequence)).toEqual(log.getSnapshot().map((_, index) => index + 1));
});
it("does not invent another session or duplicate communications when React reattaches observers", async () => {
  const f = await fixture(), log = new HbsOperationLog(), delivery = new HbsDeliveryPort(f.actual);
  const first = log.observe(f.session, "text"); first(); const second = log.observe(f.session, "text");
  delivery.setAutomatic(false);
  const before = log.observeDelivery(delivery, "text"); before(); const after = log.observeDelivery(delivery, "text");
  expect(log.getSnapshot().map(entry => entry.label)).toEqual(["会话开始", "通信消息"]);
  second(); after(); f.session.dispose(); delivery.dispose();
});
it("clears both channels and restarts sequence numbers without reimporting old delivery messages", async () => {
  const f = await fixture(), log = new HbsOperationLog(), delivery = new HbsDeliveryPort(f.actual);
  delivery.setAutomatic(false);
  const stop = log.observeDelivery(delivery, "text");
  log.record("旧输入", {text: "old"});
  expect(log.getSnapshot()).toHaveLength(2);
  const changed = vi.fn(), unsubscribe = log.subscribe(changed);
  log.clear();
  expect(changed).toHaveBeenCalledOnce();
  expect(JSON.parse(log.exportJson()).entries).toEqual([]);
  stop(); const reattached = log.observeDelivery(delivery, "text");
  expect(log.getSnapshot()).toEqual([]);
  const observe = log.observe(f.session, "text");
  expect(log.getSnapshot()).toMatchObject([{sequence: 1, label: "会话开始", detail: {initial: {content: "initial"}}}]);
  observe(); reattached(); unsubscribe(); f.session.dispose(); delivery.dispose();
});
it("copies the complete JSON with both channels, reports failure and permits retry", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const log = new HbsOperationLog();
  log.record("输入", {text: "完整输入内容", changes: [{before: "old", after: "new"}]});
  log.record("通信消息", {message: "ACK", requestId: "request-1"}, "communication");
  const writeText = vi.fn().mockRejectedValueOnce(new Error("clipboard denied")).mockResolvedValue(undefined);
  const prior = Object.getOwnPropertyDescriptor(navigator, "clipboard");
  Object.defineProperty(navigator, "clipboard", {configurable: true, value: {writeText}});
  try {
    host = document.createElement("div"); document.body.append(host); root = createRoot(host);
    await act(async () => root!.render(<HbsOperationLogPanel log={log}/>));
    const copy = host.querySelector<HTMLButtonElement>('button')!;
    await act(async () => copy.click());
    expect(feedback.error).toHaveBeenCalledWith("MODEL_OPERATION_LOG_COPY_FAILED：clipboard denied");
    expect(log.getSnapshot()).toHaveLength(2);
    await act(async () => copy.click());
    expect(writeText).toHaveBeenLastCalledWith(log.exportJson());
    expect(JSON.parse(writeText.mock.calls[1]![0]).entries).toEqual(log.getSnapshot());
    expect(feedback.success).toHaveBeenCalledWith("已复制完整操作记录");
    await act(async () => log.clear());
    expect(host.querySelector('[role="status"]')).toBeNull();
    expect(host.textContent).toContain("操作记录（0）");
  } finally {
    if (prior) Object.defineProperty(navigator, "clipboard", prior);
    else Reflect.deleteProperty(navigator, "clipboard");
  }
});

it("bounds retention by count and bytes with stable sequence and explicit incomplete exports", () => {
  const log = new HbsOperationLog({scope: "note-editor", maxEntries: 3, maxBytes: 1_024, maxDetailBytes: 256});
  for (let index = 0; index < 7; index++) log.record("input", {text: "small", index});
  expect(log.getSnapshot().map(entry => entry.sequence)).toEqual([5, 6, 7]);
  expect(log.getStats()).toMatchObject({recorded: 7, retained: 3, dropped: 4, complete: false});
  log.record("large", {text: "a".repeat(1_000)});
  expect(log.getSnapshot().at(-1)?.detail).toMatchObject({truncated: true});
  expect(log.getStats().truncated).toBe(1);
  expect(log.getStats().bytes).toBeLessThanOrEqual(1_024);
  const exported = JSON.parse(log.exportJson());
  expect(exported).toMatchObject({format: 1, scope: "note-editor", capture: {complete: false, dropped: 5, truncated: 1}});
  const small = new HbsOperationLog({maxEntries: 100, maxBytes: 300});
  for (let index = 0; index < 10; index++) small.record("input", {text: "汉字".repeat(10)});
  expect(small.getStats().bytes).toBeLessThanOrEqual(300);
  expect(small.getStats().dropped).toBeGreaterThan(0);
  const detail = new HbsOperationLog({maxDetailBytes: 64}); detail.record("large", {resourceId: "x".repeat(200), text: "y".repeat(200)});
  expect(new TextEncoder().encode(JSON.stringify(detail.getSnapshot()[0]!.detail)).byteLength).toBeLessThanOrEqual(64);
});

it("does not let a cyclic diagnostic graph break editing, and starts a fresh context after disabling", async () => {
  const log = new HbsOperationLog(), cycle: {self?: unknown} = {};
  cycle.self = cycle; expect(() => log.record("cycle", cycle)).not.toThrow();
  expect(log.getStats()).toMatchObject({truncated: 1, complete: false});
  const f = await fixture(), first = log.observe(f.session, "text"); first();
  log.clear({forgetSessions: true}); const second = log.observe(f.session, "text");
  expect(log.getSnapshot()).toHaveLength(1);
  expect(log.getSnapshot()[0]).toMatchObject({sequence: 1, label: "会话开始"});
  expect(log.getStats()).toMatchObject({truncated: 0, complete: true});
  second(); f.session.dispose();
});

it("exports the same scoped capture through the existing file adapter and reports failures", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const log = new HbsOperationLog({scope: "note-editor"}); log.record("paste", {text: "## 1. 给定规则"});
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => root!.render(<HbsOperationLogPanel log={log}/>));
  const button = [...host.querySelectorAll("button")].find(item => item.textContent === "导出操作记录 JSON")!;
  await act(async () => button.click());
  expect(feedback.save).toHaveBeenCalledWith(expect.stringMatching(/^hbs-operations-.*\.json$/u), JSON.parse(log.exportJson()), {title: "导出笔记操作记录"});
  expect(feedback.success).toHaveBeenCalledWith("已导出操作记录。");
  feedback.save.mockRejectedValueOnce(new Error("disk denied"));
  await act(async () => button.click());
  expect(feedback.error).toHaveBeenCalledWith("MODEL_OPERATION_LOG_EXPORT_FAILED：disk denied");
  expect(log.getSnapshot()).toHaveLength(1);
});
it("records native input, delete and undo/redo triggers separately from model outcomes", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const f = await fixture(), log = new HbsOperationLog();
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => root!.render(<HbsAdapterWorkspace session={f.session} delivery={new HbsDeliveryPort(f.actual)} authoritative={f.session.getSnapshot().observed}
    operations={log} resourceId="text" renderSnapshot={() => null} renderEditor={() => <textarea aria-label="test input"/>}
    externalActions={[]} onExternal={() => {}} onReset={() => {}} onRestart={() => {}}/>));
  const input = host.querySelector('textarea')!;
  await act(async () => {
    input.dispatchEvent(new InputEvent("input", {bubbles: true, inputType: "deleteContentBackward", data: null}));
    for (const [key, shiftKey] of [["z", false], ["z", true]] as const) input.dispatchEvent(new KeyboardEvent("keydown", {bubbles: true, key, ctrlKey: true, shiftKey}));
  });
  const operations = log.getSnapshot().filter(entry => entry.channel === "operation");
  expect(operations.map(entry => entry.label)).toEqual(["会话开始", "文档输入／删除事件", "触发撤销", "触发重做"]);
  expect(operations[1]!.detail).toMatchObject({inputType: "deleteContentBackward", target: {field: "test input"}});
  expect(f.actual.commit).not.toHaveBeenCalled(); f.session.dispose();
});
