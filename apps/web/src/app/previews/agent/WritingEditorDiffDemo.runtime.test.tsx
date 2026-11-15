/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { EditorView } from "@codemirror/view";
import { redo, undo } from "@codemirror/commands";
import { getChunks, getOriginalDoc } from "@codemirror/merge";
import { webcrypto } from "node:crypto";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { WritingEditorDiffDemo } from "./WritingEditorDiffDemo";
import { mixedFileChanges, binaryFileChanges } from "../../../core/features/agent/WritingDiffFixtures";

const nativeDigest = webcrypto.subtle.digest.bind(webcrypto.subtle);
const native = vi.hoisted(() => ({ invoke: vi.fn(async () => { throw new Error("Preview must not invoke desktop I/O"); }) }));
vi.mock("@tauri-apps/api/core", async importOriginal => ({ ...await importOriginal<typeof import("@tauri-apps/api/core")>(), ...native, isTauri: () => true }));
// Lifecycle tests keep mock emission fast; dedicated pacing tests cover defaults.
vi.mock("./WritingDemoTextStream", async importOriginal => ({
  ...await importOriginal<typeof import("./WritingDemoTextStream")>(),
  DEFAULT_DEMO_TEXT_PACING: { charactersPerChunk: 1000, intervalMs: 10 },
}));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root | null = null;
let host: HTMLDivElement;
beforeEach(() => {
  native.invoke.mockClear();
  vi.spyOn(crypto.subtle, "digest").mockImplementation((algorithm, data) =>
    nativeDigest(algorithm, new Uint8Array(data as ArrayBuffer)));
});
afterEach(async () => { await act(async () => root?.unmount()); root = null; host?.remove(); vi.restoreAllMocks(); });
async function eventually(check: () => void) {
  await vi.waitFor(async () => { await act(async () => { await Promise.resolve(); }); check(); }, { timeout: 4000 });
}
function view(): EditorView {
  const element = host.querySelector<HTMLElement>('[aria-label="编辑器 Diff 预览"] .cm-editor');
  expect(element).not.toBeNull(); return EditorView.findFromDOM(element!)!;
}
function button(label: string): HTMLButtonElement {
  const value = host.querySelector<HTMLButtonElement>(`[aria-label="${label}"]`);
  expect(value).not.toBeNull(); return value!;
}
async function mount() {
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => root?.render(<WritingEditorDiffDemo />));
  await eventually(() => { expect(getChunks(view().state)?.chunks).toHaveLength(2); expect(host.textContent).toContain("Edited 3 files"); });
  // The recovery banner is also absent while application is still in progress.
  // Wait for the production submission gate, not an intermediate rendered card,
  // before injecting the next turn's fault into this storage instance.
  await eventually(() => {
    expect(host.querySelector(".recovery-banner")).toBeNull();
    expect(host.querySelector('[aria-label="停止生成"]')).toBeNull();
    expect(host.textContent).not.toContain("请先等待或核对文件修改接收，再继续发送。");
  });
  await act(async () => {
    const duration = host.querySelector<HTMLInputElement>('[aria-label="模拟运行时长（秒）"]')!;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(duration, "0.1");
    duration.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
async function select(title: string) {
  if (!host.querySelector('[aria-label="对话列表面板"]')) await act(async () => button("对话列表").click());
  await act(async () => {
    const row = [...host.querySelectorAll<HTMLButtonElement>('[aria-label="对话列表面板"] button')].find(item => item.querySelector("strong")?.textContent === title);
    expect(row).toBeDefined(); row!.click();
  });
  await eventually(() => { expect(host.querySelector<HTMLTextAreaElement>("textarea")?.disabled).toBe(false); expect(host.textContent).not.toContain("正在核对对话状态"); });
  if (!host.querySelector<HTMLElement>('[aria-label="对话列表面板"]')?.hidden) await act(async () => button("隐藏对话列表").click());
}
async function send(text: string) {
  const textarea = host.querySelector<HTMLTextAreaElement>("textarea")!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(textarea, text);
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await eventually(() => expect(button("发送消息").disabled).toBe(false));
  await act(async () => textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })));
}

it("uses latest buffers across conversations and keeps background file delivery and undo independent of chat", async () => {
  await mount();
  const editor = view();
  await act(async () => editor.dispatch({ changes: { from: editor.state.doc.length, insert: "\n% manual latest text" } }));
  const before = editor.state.doc.toString(), baseline = getOriginalDoc(editor.state).toString();
  await select("第二个对话");
  expect(view()).toBe(editor);
  expect(editor.state.doc.toString()).toBe(before);
  await send("continue latest text");
  await eventually(() => expect(host.textContent).toContain("continue latest text"));
  await act(async () => button("隐藏 AI 写作助手").click());
  await eventually(() => expect(editor.state.doc.toString()).toBe(`${before}\n% Mock Agent: continue latest text`));
  expect(getOriginalDoc(editor.state).toString()).toBe(baseline);
  await select("完善摘要");
  expect(view()).toBe(editor);
  await act(async () => { expect(undo(editor)).toBe(true); });
  expect(editor.state.doc.toString()).toBe(before);
  await act(async () => { expect(redo(editor)).toBe(true); });
  const after = editor.state.doc.toString();
  await act(async () => button("Keep all").click());
  await select("第二个对话");
  expect(getChunks(editor.state)?.chunks).toHaveLength(0);
  await act(async () => { expect(undo(editor)).toBe(true); });
  expect(getChunks(editor.state)?.chunks.length).toBeGreaterThan(0);
  await act(async () => { expect(redo(editor)).toBe(true); });
  const restart = [...host.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === "模拟退出并重开")!;
  await act(async () => restart.click());
  await eventually(() => { expect(view()).not.toBe(editor); expect(view().state.doc.toString()).toBe(after); expect(getChunks(view().state)?.chunks).toHaveLength(0); });
  await select("完善摘要");
  expect(view().state.doc.toString()).toBe(after);
  expect(getChunks(view().state)?.chunks).toHaveLength(0);
  expect(native.invoke).not.toHaveBeenCalled();
  // This spans two submissions, chat switches, undo/redo and a workspace restart.
  // Each state wait remains bounded to 4 seconds; allow the complete sequence
  // more than the single-test default when the whole suite runs concurrently.
}, 15_000);

it("stops a simulated execution through the production cancellation path without applying files", async () => {
  await mount(); const editor = view(), before = editor.state.doc.toString();
  // Keep cancellation observable under suite-wide contention. Stopping aborts
  // this timer, so the test does not wait for the configured duration.
  await act(async () => {
    const duration = host.querySelector<HTMLInputElement>('[aria-label="模拟运行时长（秒）"]')!;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(duration, "60");
    duration.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await select("第二个对话"); await send("cancel this modification");
  await eventually(() => expect(host.querySelector('[aria-label="停止生成"]')).not.toBeNull());
  await act(async () => button("停止生成").click());
  await eventually(() => expect(host.querySelector('[aria-label="停止生成"]')).toBeNull());
  expect(editor.state.doc.toString()).toBe(before);
  expect(host.textContent).not.toContain("重新核对文件修改");
  expect(native.invoke).not.toHaveBeenCalled();
});
it("exercises pre-dispatch failure and restart recovery from the real preview controls", async () => {
  await mount(); const editor = view(), before = editor.state.doc.toString(), baseline = getOriginalDoc(editor.state).toString();
  await select("第二个对话");
  const fault = host.querySelector<HTMLSelectElement>('[aria-label="模拟故障"]')!;
  await act(async () => { fault.value = "dispatch"; fault.dispatchEvent(new Event("change", { bubbles: true })); });
  await send("resume the interrupted files");
  await eventually(() => expect(host.querySelector('[aria-label="处理文件操作"], [aria-label="处理 Agent 文件修改"]')).not.toBeNull());
  expect(editor.state.doc.toString()).toBe(before);
  await act(async () => { fault.value = "none"; fault.dispatchEvent(new Event("change", { bubbles: true })); });
  const restart = [...host.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === "模拟退出并重开")!;
  await act(async () => restart.click());
  await eventually(() => {
    expect(view()).not.toBe(editor);
    expect(view().state.doc.toString()).toBe(before);
    expect(host.textContent).toContain("继续原文件修改");
  });
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")]
    .find(item => item.textContent === "继续原文件修改")!.click());
  await eventually(() => {
    expect(view()).not.toBe(editor);
    expect(view().state.doc.toString()).toBe(`${before}\n% Mock Agent: resume the interrupted files`);
    expect(host.querySelector('[aria-label="处理文件操作"], [aria-label="处理 Agent 文件修改"]')).toBeNull();
  });
  expect(getOriginalDoc(view().state).toString()).toBe(baseline);
  expect(native.invoke).not.toHaveBeenCalled();
});

it("dispatches a queued follow-up from the file content delivered by the preceding turn", async () => {
  await mount(); const editor = view();
  await select("第二个对话"); await send("first queued modification");
  const textarea = host.querySelector<HTMLTextAreaElement>("textarea")!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(textarea, "second queued modification");
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await eventually(() => expect(button("发送消息").disabled).toBe(false));
  await act(async () => textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })));
  await eventually(() => {
    expect(editor.state.doc.toString()).toContain("% Mock Agent: first queued modification\n% Mock Agent: second queued modification");
  });
  expect(native.invoke).not.toHaveBeenCalled();
});
it("reconciles the injected editor journal fault using the production workspace recovery control", async () => {
  await mount();
  const editor = view(), before = editor.state.doc.toString();
  await select("第二个对话");
  const fault = host.querySelector<HTMLSelectElement>('[aria-label="模拟故障"]')!;
  await act(async () => { fault.value = "editorConfirm"; fault.dispatchEvent(new Event("change", { bubbles: true })); });
  await send("confirm original delivery receipt");
  await eventually(() => expect(host.querySelector('[aria-label="处理文件操作"], [aria-label="处理 Agent 文件修改"]')).not.toBeNull());
  expect(editor.state.doc.toString()).toBe(`${before}\n% Mock Agent: confirm original delivery receipt`);
  const applied = editor.state.doc.toString();
  await act(async () => { fault.value = "none"; fault.dispatchEvent(new Event("change", { bubbles: true })); });
  expect(host.querySelector(".writing-agent-inline-diff-card details")).toBeNull();
  await eventually(() => expect([...host.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === "核对文件结果")?.disabled).toBe(false));
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === "核对文件结果")!.click());
  await eventually(() => expect(host.textContent).toContain("重试保存文件结果"));
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === "重试保存文件结果")!.click());
  await eventually(() => expect(host.querySelector('[aria-label="处理文件操作"], [aria-label="处理 Agent 文件修改"]')).toBeNull());
  expect(view()).toBe(editor); expect(editor.state.doc.toString()).toBe(applied);
  expect(native.invoke).not.toHaveBeenCalled();
});

it("opens added source files and reconciles a removed file through the real preview recovery controls", async () => {
  await mount();
  const fault = host.querySelector<HTMLSelectElement>('[aria-label="模拟故障"]')!;
  await act(async () => { fault.value = "diskConfirm"; fault.dispatchEvent(new Event("change", { bubbles: true })); });
  const scenario = host.querySelector<HTMLSelectElement>('[aria-label="模拟文件场景"]')!;
  await act(async () => { scenario.value = "mixed"; scenario.dispatchEvent(new Event("change", { bubbles: true })); });
  await eventually(() => {
    expect(host.querySelector('[aria-label="处理文件操作"], [aria-label="处理 Agent 文件修改"]')).not.toBeNull();
    expect(view().state.doc.toString()).toBe(mixedFileChanges[0]!.after);
  });
  const original = view();
  const nextFault = host.querySelector<HTMLSelectElement>('[aria-label="模拟故障"]')!;
  await act(async () => { nextFault.value = "none"; nextFault.dispatchEvent(new Event("change", { bubbles: true })); });
  expect(host.querySelector(".writing-agent-inline-diff-card details")).toBeNull();
  await eventually(() => expect([...host.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === "核对文件结果")?.disabled).toBe(false));
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === "核对文件结果")!.click());
  await eventually(() => expect(host.textContent).toContain("重试保存文件结果"));
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === "重试保存文件结果")!.click());
  await eventually(() => expect(host.querySelector('[aria-label="处理文件操作"], [aria-label="处理 Agent 文件修改"]')).toBeNull());
  expect(view()).toBe(original);
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent?.includes("sections/new.tex"))!.click());
  expect(host.querySelector("dialog")).toBeNull();
  await eventually(() => expect(view().state.doc.toString()).toBe(mixedFileChanges[1]!.after));
  expect(view()).toBe(original);
  await act(async () => original.dispatch({ changes: { from: original.state.doc.length, insert: "% later input" } }));
  const latest = original.state.doc.toString();
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === "模拟退出并重开")!.click());
  await eventually(() => expect(view().state.doc.toString()).toBe(latest));
  expect(native.invoke).not.toHaveBeenCalled();
});

it("uses production non-text history and recovery while retaining the source editor through restart", async () => {
  await mount();
  const fault = host.querySelector<HTMLSelectElement>('[aria-label="模拟故障"]')!;
  await act(async () => { fault.value = "diskConfirm"; fault.dispatchEvent(new Event("change", { bubbles: true })); });
  const scenario = host.querySelector<HTMLSelectElement>('[aria-label="模拟文件场景"]')!;
  await act(async () => { scenario.value = "binary"; scenario.dispatchEvent(new Event("change", { bubbles: true })); });
  await eventually(() => { expect(host.querySelector('[aria-label="处理文件操作"], [aria-label="处理 Agent 文件修改"]')).not.toBeNull(); expect(view().state.doc.toString()).toBe(binaryFileChanges[0]!.after); });
  const editor = view();
  const nextFault = host.querySelector<HTMLSelectElement>('[aria-label="模拟故障"]')!;
  await act(async () => { nextFault.value = "none"; nextFault.dispatchEvent(new Event("change", { bubbles: true })); });
  expect(host.querySelector(".writing-agent-inline-diff-card details")).toBeNull();
  await eventually(() => expect([...host.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === "核对文件结果")?.disabled).toBe(false));
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === "核对文件结果")!.click());
  await eventually(() => expect(host.textContent).toContain("重试保存文件结果"));
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === "重试保存文件结果")!.click());
  await eventually(() => expect(host.querySelector('[aria-label="处理文件操作"], [aria-label="处理 Agent 文件修改"]')).toBeNull());
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent?.startsWith("assets/figure.bin"))!.click());
  expect(host.querySelector('[aria-label="副本变更审阅"]')).toBeNull();
  expect(view()).toBe(editor);
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === "模拟退出并重开")!.click());
  await eventually(() => expect(view().state.doc.toString()).toBe(binaryFileChanges[0]!.after));
  expect(host.querySelectorAll('[aria-label="编辑器 Diff 预览"] .cm-editor')).toHaveLength(1);
  expect(native.invoke).not.toHaveBeenCalled();
});
