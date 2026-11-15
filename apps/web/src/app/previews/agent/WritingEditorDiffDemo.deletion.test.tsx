/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { EditorView } from "@codemirror/view";
import { undo, redo } from "@codemirror/commands";
import { getChunks, getOriginalDoc } from "@codemirror/merge";
import { webcrypto } from "node:crypto";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { WritingEditorDiffDemo } from "./WritingEditorDiffDemo";
import { createWritingDiffDemoRuntime } from "./writingDiffDemoRuntime";
import { mockWebLocks } from "../../../core/runtime/testing/mockWebLocks";

vi.mock("./writingDiffDemoRuntime", async importOriginal => {
  const original = await importOriginal<typeof import("./writingDiffDemoRuntime")>();
  return { ...original, createWritingDiffDemoRuntime: vi.fn(original.createWritingDiffDemoRuntime) };
});
const native = vi.hoisted(() => ({ invoke: vi.fn(async () => { throw new Error("Preview must not invoke desktop I/O"); }) }));
vi.mock("@tauri-apps/api/core", async importOriginal => ({ ...await importOriginal<typeof import("@tauri-apps/api/core")>(), ...native, isTauri: () => true }));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root | undefined, host: HTMLDivElement;
const preferences = new Map<string, string>();
Object.defineProperty(window, "localStorage", { configurable: true, value: {
  getItem: (key: string) => preferences.get(key) ?? null,
  setItem: (key: string, value: string) => preferences.set(key, value),
  removeItem: (key: string) => preferences.delete(key),
} });
const digest = webcrypto.subtle.digest.bind(webcrypto.subtle);
beforeEach(() => {
  mockWebLocks();
  native.invoke.mockClear(); vi.mocked(createWritingDiffDemoRuntime).mockClear(); preferences.clear();
  vi.spyOn(crypto.subtle, "digest").mockImplementation((algorithm, data) => digest(algorithm, new Uint8Array(data as ArrayBuffer)));
  vi.spyOn(HTMLDialogElement.prototype, "showModal").mockImplementation(function (this: HTMLDialogElement) { this.setAttribute("open", ""); });
  vi.spyOn(HTMLDialogElement.prototype, "close").mockImplementation(function (this: HTMLDialogElement) { this.removeAttribute("open"); });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root?.unmount()); host?.remove(); document.querySelectorAll("dialog").forEach(dialog => dialog.remove()); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
async function settled(check: () => void) {
  await vi.waitFor(async () => { await act(async () => { await Promise.resolve(); }); check(); }, { timeout: 5000 });
}
function view(): EditorView {
  const element = host!.querySelector<HTMLElement>('[aria-label="编辑器 Diff 预览"] .cm-editor');
  expect(element).not.toBeNull(); return EditorView.findFromDOM(element!)!;
}
function clickLabel(container: ParentNode, label: string): void {
  const button = container.querySelector<HTMLButtonElement>(`[aria-label="${label}"]`);
  expect(button).not.toBeNull(); button!.click();
}
async function confirm(): Promise<void> {
  await act(async () => { const button = [...document.querySelectorAll<HTMLButtonElement>("dialog button")].find(item => item.textContent === "确认"); expect(button).toBeDefined(); button!.click(); });
}
async function mountAndDelete() {
  await act(async () => root!.render(<WritingEditorDiffDemo />));
  await settled(() => { expect(getChunks(view().state)?.chunks).toHaveLength(2); expect(host.textContent).toContain("Edited 3 files");
    expect(host.querySelector(".recovery-banner")).toBeNull(); expect(host.textContent).not.toContain("请先等待或核对文件修改接收，再继续发送。"); });
  const call = vi.mocked(createWritingDiffDemoRuntime).mock.calls[0]!, runtime = vi.mocked(createWritingDiffDemoRuntime).mock.results[0]!.value;
  const editor = view(), before = editor.state.doc.toString(), baseline = getOriginalDoc(editor.state).toString();
  await act(async () => editor.dispatch({ selection: { anchor: before.indexOf("This paper") + 4 } }));
  const cursor = editor.state.selection.main.head;
  await act(async () => clickLabel(host, "对话列表"));
  const history = await call[1].reviews.list({ manuscriptId: call[2], conversationId: call[3], cursor: null, limit: 20 });
  const proposal = await call[1].gateways.delivery.proposals.latest(history.entries[0]!.scope);
  const applicationScope = { manuscriptId: call[2], conversationId: call[3], proposalId: proposal!.proposalId };
  let application = await call[1].gateways.application.applications.find(applicationScope);
  await vi.waitFor(async () => {
    await act(async () => { await Promise.resolve(); });
    application = await call[1].gateways.application.applications.find(applicationScope);
    expect(application?.status).toBe("applied");
  }, { timeout: 5000 });
  const row = host.querySelector(`[data-conversation-id="${call[3]}"]`)!;
  await act(async () => clickLabel(row, "归档对话"));
  await settled(() => expect(host.querySelector(`[data-conversation-id="${call[3]}"]`)).toBeNull());
  await act(async () => clickLabel(host, "查看归档对话"));
  await settled(() => expect(host.querySelector(`[data-conversation-id="${call[3]}"]`)).not.toBeNull());
  await act(async () => clickLabel(host.querySelector(`[data-conversation-id="${call[3]}"]`)!, "永久删除对话"));
  await settled(() => expect(document.querySelector('dialog[aria-label="永久删除对话"]')).not.toBeNull());
  await confirm();
  await settled(() => expect(document.querySelector('dialog[aria-label="强制删除对话"]')).not.toBeNull());
  await confirm();
  await settled(() => { expect(document.querySelector("dialog")).toBeNull(); expect(host.querySelector(`[data-conversation-id="${call[3]}"]`)).toBeNull(); });
  return { storage: call[0], agent: call[1], conversationId: call[3], manuscriptId: call[2], runtime, editor, before, baseline, cursor, application: application! };
}
it("deletes through production dialogs while retaining the editor, diff, cursor and unified history", async () => {
  const f = await mountAndDelete();
  expect(view()).toBe(f.editor); expect(f.editor.state.doc.toString()).toBe(f.before);
  expect(getOriginalDoc(f.editor.state).toString()).toBe(f.baseline);
  expect(f.editor.state.selection.main.head).toBe(f.cursor);
  await act(async () => { expect(undo(f.editor)).toBe(true); });
  expect(getChunks(f.editor.state)?.chunks).toHaveLength(0);
  await act(async () => { expect(redo(f.editor)).toBe(true); });
  expect(f.editor.state.doc.toString()).toBe(f.before); expect(getChunks(f.editor.state)?.chunks).toHaveLength(2);
  await act(async () => clickLabel(host!, "Keep all"));
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === "模拟退出并重开")!.click());
  await settled(() => { expect(view()).not.toBe(f.editor); expect(view().state.doc.toString()).toBe(f.before); expect(getChunks(view().state)?.chunks).toHaveLength(0); });
  await expect(f.agent.gateways.delivery.sessions.open(f.conversationId)).rejects.toBe("notFound");
  expect((await f.storage.files.readFile(f.manuscriptId, "main.tex")).content).toBe(f.before);
  expect(await f.agent.gateways.application.applications.find(f.application.scope)).toEqual(f.application);
  expect(native.invoke).not.toHaveBeenCalled();
});

it("rediscovers a retained file Undo after deleting its chat and rebuilding the preview workspace", async () => {
  const f = await mountAndDelete(), undoId = crypto.randomUUID();
  const buffers = await Promise.all(f.application.files.map(async file => {
    const saved = await f.storage.files.readFile(f.manuscriptId, file.path);
    const checkpoint = (await f.storage.checkpoints.list(f.manuscriptId)).find(record => record.checkpoint.relativePath === file.path)?.checkpoint;
    expect(checkpoint).not.toBeNull();
    return { path: file.path, expectedDiskSha256: saved.sha256, text: saved.content,
      sessionId: checkpoint!.editorReceipt!.sessionId, revision: checkpoint!.editorReceipt!.revision };
  }));
  await act(async () => { await f.agent.gateways.application.undos.execute({ scope: f.application.scope, undoId, command: { action: "prepare", buffers } }); });
  await act(async () => [...host!.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === "模拟退出并重开")!.click());
  await settled(() => { expect(host!.querySelector('[aria-label="文件撤销恢复"]')).not.toBeNull(); expect(view().state.doc.toString()).toBe(f.before); });
  expect(view().state.doc.toString()).toBe(f.before); expect(getChunks(view().state)?.chunks).toHaveLength(2);
  await act(async () => [...host!.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === "取消未执行撤销")!.click());
  await settled(() => expect(host!.querySelector('[aria-label="文件撤销恢复"]')).toBeNull());
  expect(view().state.doc.toString()).toBe(f.before);
  const retained = (await f.agent.gateways.application.undos.find(f.application.scope))?.result;
  expect(retained?.kind).toBe("state");
  expect(retained?.kind === "state" && retained.status).toBe("rolledBack");
  expect(native.invoke).not.toHaveBeenCalled();
});
