// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { noteBlockId, notePageId, type NoteDocumentV1 } from "@research-assistant/core/notes";
import type { TextPatchReview, TextPatchResolution, TextPatchResolutionReceipt } from "../../agent/textPatchGateway";
import { notifyTextPatchesChanged } from "../../agent/textPatchGateway";
import { getAppStorage, selectAppStorage } from "../../../runtime/appStorage";
import { NoteEditor } from "./NoteEditor";
import type { NoteEditorInstance } from "./noteEditorSchema";
import type { NoteEditorInstallPort } from "./noteEditorInstallPort";
import { flushNoteReview, holdTextEditor, retryNoteReview, useNoteReviewSaveState, synchronizeNoteReview, noteReviewRecovery } from "./noteTextEditorPorts";
import { readNoteReviewHistory } from "./noteReviewHistory";
import { NoteSaveState } from "../workspace/NoteSaveState";

const calls = vi.hoisted(() => ({ forPage: vi.fn(), review: vi.fn(), resolve: vi.fn(), editor: undefined as unknown }));
vi.hoisted(() => {
  if (document.doctype === null) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", { configurable: true, value: "CSS1Compat" });
});
vi.mock("../../../runtime/appRuntime", async () => {
  const actual = await vi.importActual<typeof import("../../../runtime/appRuntime")>("../../../runtime/appRuntime");
  return { ...actual, getAppRuntime: () => ({ mode: "preview", gateways: {} }) };
});
vi.mock("../../agent/textPatchGateway", async () => ({
  ...await vi.importActual<typeof import("../../agent/textPatchGateway")>("../../agent/textPatchGateway"),
  textPatchGateway: { forPage: calls.forPage, review: calls.review, resolve: calls.resolve, decide: vi.fn() },
}));
vi.mock("../NotesClient", async () => {
  const { NoteCommandHistory } = await import("./NoteCommandHistory");
  const history = new NoteCommandHistory();
  return { useNotesClient: () => ({ gateway: {}, history, mutate: vi.fn() }), useNotesVersion: () => 0 };
});
vi.mock("@blocknote/react", async () => {
  const actual = await vi.importActual<typeof import("@blocknote/react")>("@blocknote/react");
  return { ...actual, useCreateBlockNote: (...args: Parameters<typeof actual.useCreateBlockNote>) => {
    const editor = actual.useCreateBlockNote(...args); calls.editor = editor; return editor;
  } };
});
const cleanup: (() => Promise<void>)[] = [];
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); calls.editor = undefined; vi.clearAllMocks();
  const values = new Map<string, string>();
  selectAppStorage({ get length() { return values.size; }, key: index => [...values.keys()][index] ?? null,
    getItem: key => values.get(key) ?? null, setItem: (key, value) => { values.set(key, value); }, removeItem: key => { values.delete(key); }, clear: () => values.clear() });
});
afterEach(async () => { for (const dispose of cleanup.splice(0).reverse()) await dispose(); getAppStorage().clear(); vi.unstubAllGlobals(); });

function documentValue(argument: string): NoteDocumentV1 {
  return { schemaVersion: 1, pageId: notePageId("review-test-page"), blocks: [
    { id: noteBlockId("argument"), type: "paragraph", props: {}, children: [], content: { kind: "inline", items: [{ type: "text", text: argument, styles: [] }] } },
    { id: noteBlockId("source"), type: "paragraph", props: {}, children: [], content: { kind: "inline", items: [{ type: "text", text: "Source unchanged", styles: [] }] } },
  ] };
}
const original = documentValue("Original argument"), candidate = documentValue("Agent argument");
function review(after = candidate, baseline = original, generation = 1, state: TextPatchReview["state"] = "pending"): TextPatchReview {
  return { proposalId: "proposal", generation, operationGeneration: generation, baselineMatches: true, wholeUndoAvailable: true, state, pages: [{
    pageId: original.pageId, path: `source://notes/${original.pageId}`, title: "Synthetic note", kind: "update", expectedRevision: 1,
    before: "Original argument", after: "Agent argument", beforeDocument: original, afterDocument: after, reviewDocument: baseline,
    chunks: state === "pending" ? [{ id: `change-${generation}`, beforeBlockIds: ["argument"], afterBlockIds: ["argument"], beforeIndex: 0, afterIndex: 0 }] : [],
  }] };
}
function receipt(request: TextPatchResolution, view: TextPatchReview): TextPatchResolutionReceipt {
  return { requestId: request.requestId, proposalId: request.proposalId, action: request.action, generation: view.generation, review: view, pages: [] };
}
function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function button(host: HTMLElement, label: string): HTMLButtonElement {
  const found = host.querySelector<HTMLButtonElement>(`button[aria-label^="${label}"]`)
    ?? Array.from(host.querySelectorAll("button")).find(button => button.textContent === label);
  if (!found) throw new Error(`Missing ${label}`); return found;
}
function ReviewSaveIndicator() {
  const state = useNoteReviewSaveState(original.pageId);
  return <NoteSaveState pageId={original.pageId} status={state.status} message={state.message} />;
}
async function mounted(initial: TextPatchReview | null = review(), initialDocument = original) {
  let current = initial;
  calls.forPage.mockResolvedValue(["conversation"]);
  calls.review.mockImplementation(async () => current);
  calls.resolve.mockImplementation(async (request: TextPatchResolution) => {
    current = review(request.document ?? candidate, original, request.generation + 1);
    return receipt(request, current);
  });
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const onChange = vi.fn(); let port: NoteEditorInstallPort | undefined;
  const install = (next: NoteEditorInstallPort | undefined) => { port = next; };
  await act(async () => root.render(<><NoteEditor document={initialDocument} onChange={onChange} onInstallPort={install} /><ReviewSaveIndicator /></>));
  await vi.waitFor(() => expect(port).toBeDefined());
  cleanup.push(async () => { await act(async () => root.unmount()); host.remove(); });
  const editor = calls.editor as NoteEditorInstance;
  return { host, editor, onChange, port: () => port!, view: (value: TextPatchReview | null) => { current = value; } };
}
async function visibleDiff(host: HTMLElement): Promise<void> {
  await vi.waitFor(() => expect(host.querySelector(".note-diff-chunk-actions")).not.toBeNull());
}

it("adopts Agent D/B before unlocking without a chat-card notification, then saves manual edits through review", async () => {
  const state = await mounted(null);
  await act(async () => {
    holdTextEditor(original.pageId, "agent-write", true);
    expect(state.editor.isEditable).toBe(false);
  });
  expect(state.host.querySelector('[role="status"]')?.getAttribute("aria-label")).toBe("agent正在写入");
  expect(state.host.querySelector(".note-save-state.saving .lucide-loader-circle")).not.toBeNull();
  await act(async () => state.port().prepareStructure(state.port().read(), candidate, { baseline: original, origin: "agent", addToHistory: true })());
  state.view(review());
  await act(async () => { await synchronizeNoteReview(original.pageId); });
  await visibleDiff(state.host);
  await act(async () => holdTextEditor(original.pageId, "agent-write", false));
  expect(state.editor.isEditable).toBe(true);
  expect(state.host.querySelector('[role="status"]')?.getAttribute("aria-label")).toBe("已保存");
  state.onChange.mockClear();
  await act(async () => state.editor.updateBlock("argument", { content: "Agent plus manual input" }));
  expect(state.onChange).not.toHaveBeenCalled();
  await act(async () => { await flushNoteReview(original.pageId); });
  expect(calls.resolve.mock.lastCall?.[0]).toMatchObject({ action: "edit" });
  expect(body(state.port())).toBe("Agent plus manual input");
  expect(state.host.querySelector(".note-diff-chunk-actions")).not.toBeNull();
});

it("keeps newer formal input on reopen when an old proposal no longer matches S", async () => {
  const latest = documentValue("Saved manual input");
  const state = await mounted({ ...review(), baselineMatches: false }, latest);
  expect(body(state.port())).toBe("Saved manual input");
  expect(state.host.querySelector(".note-diff-chunk-actions")).toBeNull();
  expect(state.host.querySelector(".note-save-state")?.getAttribute("aria-label")).toMatch(/^版本冲突/);
  expect(state.onChange).not.toHaveBeenCalled();
});

it("retries a failed projection instead of treating its generation as installed", async () => {
  const state = await mounted(); await visibleDiff(state.host);
  const install = vi.spyOn(state.port(), "prepareStructure").mockImplementationOnce(() => { throw new Error("暂不能安装"); });
  state.view(review(documentValue("New Agent input"), original, 2));
  await act(async () => notifyTextPatchesChanged());
  await vi.waitFor(() => expect(state.host.querySelector(".note-review-recovery")?.getAttribute("aria-label")).toContain("暂不能安装"));
  expect(body(state.port())).toBe("Agent argument");
  const attempts = install.mock.calls.length;
  await act(async () => notifyTextPatchesChanged());
  await vi.waitFor(() => expect(body(state.port())).toBe("New Agent input"));
  expect(install.mock.calls.length).toBeGreaterThan(attempts);
});

it("releases definitively stale edit requests and retains recoverable input instead of replaying them forever", async () => {
  const state = await mounted(); await visibleDiff(state.host);
  calls.resolve.mockRejectedValue({ status: "failed", message: "saved note changed; preserve current input and compare the saved version" });
  await act(async () => state.editor.updateBlock("argument", { content: "Preserved manual input" }));
  await act(async () => { await expect(flushNoteReview(original.pageId)).rejects.toBeDefined(); });
  expect(getAppStorage().getItem(`note-inline-review:${original.pageId}`)).toBeNull();
  expect(state.editor.isEditable).toBe(false);
  await act(async () => { await expect(retryNoteReview(original.pageId)).rejects.toThrow(/比较版本/); });
  expect(calls.resolve).toHaveBeenCalledOnce();
  expect(noteReviewRecovery(original.pageId)?.document.blocks[0]?.content).toEqual({ kind: "inline", items: [{ type: "text", text: "Preserved manual input", styles: [] }] });
});

it("reports queued and in-flight D/B until the latest receipt while retaining enabled review buttons", async () => {
  const state = await mounted(); await visibleDiff(state.host);
  const status = () => state.host.querySelector(".note-save-state")?.getAttribute("aria-label");
  const keep = button(state.host, "保留第 1 处变更");
  const first = deferred<TextPatchResolutionReceipt>(), second = deferred<TextPatchResolutionReceipt>();
  calls.resolve.mockImplementationOnce(() => first.promise).mockImplementationOnce(() => second.promise);
  expect(status()).toBe("已保存");
  await act(async () => state.editor.updateBlock("argument", { content: "First input" }));
  expect(status()).toBe("等待保存");
  let flushing!: Promise<void>;
  await act(async () => { flushing = flushNoteReview(original.pageId); });
  await vi.waitFor(() => expect(calls.resolve).toHaveBeenCalledOnce());
  expect(status()).toBe("正在保存…"); expect(keep.disabled).toBe(false);
  await act(async () => state.editor.updateBlock("argument", { content: "First input + latest input" }));
  const request = calls.resolve.mock.calls[0]![0] as TextPatchResolution;
  await act(async () => first.resolve(receipt(request, review(request.document!, original, 2))));
  await vi.waitFor(() => expect(calls.resolve).toHaveBeenCalledTimes(2));
  expect(status()).toBe("正在保存…");
  expect(button(state.host, "保留第 1 处变更")).toBe(keep); expect(keep.disabled).toBe(false);
  const latest = calls.resolve.mock.calls[1]![0] as TextPatchResolution;
  const final = review(latest.document!, original, 3); state.view(final);
  await act(async () => { second.resolve(receipt(latest, final)); await flushing; });
  expect(status()).toBe("已保存"); expect(body(state.port())).toBe("First input + latest input");
  expect(button(state.host, "保留第 1 处变更")).toBe(keep); expect(keep.disabled).toBe(false);
});

it("exposes exact-request recovery to the shared save indicator after an unknown write", async () => {
  const state = await mounted(); await visibleDiff(state.host);
  calls.resolve.mockRejectedValueOnce({ status: "unknown", message: "Missing receipt" });
  await act(async () => state.editor.updateBlock("argument", { content: "Preserved input" }));
  await act(async () => { await expect(flushNoteReview(original.pageId)).rejects.toBeDefined(); });
  expect(state.host.querySelector(".note-save-state")?.getAttribute("aria-label")).toMatch(/^保存失败/);
  const originalRequest = calls.resolve.mock.calls[0]![0];
  await act(async () => { await retryNoteReview(original.pageId); });
  expect(calls.resolve.mock.calls[1]![0]).toEqual(originalRequest);
  expect(state.host.querySelector(".note-save-state")?.getAttribute("aria-label")).toBe("已保存");
});

it("queues a Keep clicked during typing save and resolves the latest generation after it drains", async () => {
  const state = await mounted(); await visibleDiff(state.host);
  const saving = deferred<TextPatchResolutionReceipt>();
  calls.resolve.mockImplementationOnce(() => saving.promise).mockImplementation(async (request: TextPatchResolution) => {
    const kept = review(documentValue("Latest input"), documentValue("Latest input"), 3, "accept");
    state.view(kept); return receipt(request, kept);
  });
  await act(async () => state.editor.updateBlock("argument", { content: "Latest input" }));
  let flushing!: Promise<void>;
  await act(async () => { flushing = flushNoteReview(original.pageId); });
  await vi.waitFor(() => expect(calls.resolve).toHaveBeenCalledOnce());
  const edit = calls.resolve.mock.calls[0]![0] as TextPatchResolution;
  await act(async () => button(state.host, "保留第 1 处变更").click());
  expect(calls.resolve).toHaveBeenCalledOnce();
  await act(async () => { saving.resolve(receipt(edit, review(edit.document!, original, 2))); await flushing; });
  await vi.waitFor(() => expect(state.host.querySelector(".note-diff-chunk-actions")).toBeNull());
  expect(calls.resolve.mock.calls[1]![0]).toMatchObject({ action: "keep", generation: 2, chunkId: "change-2" });
  expect(body(state.port())).toBe("Latest input");
  expect(state.host.querySelector(".note-save-state")?.getAttribute("aria-label")).toBe("已保存");
});
it.each(["keep", "undo"] as const)("returns keyboard ownership after %s resolves", async action => {
  const state = await mounted(); await visibleDiff(state.host);
  calls.resolve.mockImplementation(async (request: TextPatchResolution) => {
    const settled = action === "keep" ? review(candidate, candidate, 2, "accept") : review(original, original, 2, "reject");
    state.view(settled); return receipt(request, settled);
  });
  await act(async () => button(state.host, action === "keep" ? "Keep all" : "Undo all").click());
  await vi.waitFor(() => expect(state.host.querySelector(".note-diff-chunk-actions")).toBeNull());
  expect(globalThis.document.activeElement).toBe(state.editor.domElement);
});
it("does not steal another input's focus while a review request is pending", async () => {
  const state = await mounted(); await visibleDiff(state.host);
  const operation = deferred<TextPatchResolutionReceipt>();
  calls.resolve.mockImplementation(() => operation.promise);
  await act(async () => state.editor.focus());
  await act(async () => button(state.host, "Keep all").click());
  await vi.waitFor(() => expect(calls.resolve).toHaveBeenCalledOnce());
  const input = globalThis.document.createElement("input"); state.host.append(input); input.focus();
  const request = calls.resolve.mock.calls[0]![0] as TextPatchResolution;
  await act(async () => operation.resolve(receipt(request, review(candidate, candidate, 2, "accept"))));
  expect(globalThis.document.activeElement).toBe(input);
});
function body(port: NoteEditorInstallPort): string {
  const content = port.read().blocks[0]?.content;
  return content?.kind === "inline" ? content.items.filter(item => item.type === "text").map(item => item.text).join("") : "";
}

it("ignores a late proposal read after ordinary candidate input instead of replacing D", async () => {
  const state = await mounted(); await visibleDiff(state.host); state.onChange.mockClear();
  const reading = deferred<TextPatchReview | null>();
  calls.review.mockImplementationOnce(() => reading.promise);
  const beforeReads = calls.review.mock.calls.length;
  await act(async () => notifyTextPatchesChanged());
  await vi.waitFor(() => expect(calls.review.mock.calls.length).toBe(beforeReads + 1));
  await act(async () => state.editor.updateBlock("argument", { content: "Later ordinary input" }));
  await act(async () => reading.resolve(review(documentValue("Later agent response"), original, 2)));
  expect(body(state.port())).toBe("Later ordinary input");
  expect(state.onChange).not.toHaveBeenCalled();
  // Flush explicitly so cleanup cannot leave a scheduled private edit.
  await act(async () => { await flushNoteReview(original.pageId); });
  expect(calls.resolve.mock.calls.some(([request]) => request.action === "edit" && request.document.blocks[0].content.items[0].text === "Later ordinary input")).toBe(true);
});

it("keeps ordinary text undo available while its prior typing checkpoint is in flight", async () => {
  const state = await mounted(); await visibleDiff(state.host);
  const saving = deferred<TextPatchResolutionReceipt>();
  calls.resolve.mockImplementationOnce(() => saving.promise);
  await act(async () => state.editor.updateBlock("argument", { content: "Typed after the Agent" }));
  let flush!: Promise<void>;
  await act(async () => { flush = flushNoteReview(original.pageId); });
  await vi.waitFor(() => expect(calls.resolve).toHaveBeenCalledOnce());
  const recoveryWhileSaving = [...state.host.querySelectorAll("button")].some(button => button.textContent === "恢复原操作");
  const reviewEnabledWhileSaving = !button(state.host, "保留第 1 处变更").disabled;
  const first = calls.resolve.mock.calls[0]![0] as TextPatchResolution;
  await act(async () => { state.editor.undo(); });
  const undone = body(state.port());
  await act(async () => { saving.resolve(receipt(first, review(first.document!, original, 2))); await flush; });
  expect(undone).toBe("Agent argument");
  expect(recoveryWhileSaving).toBe(false);
  expect(reviewEnabledWhileSaving).toBe(true);
  expect(body(state.port())).toBe("Agent argument");
  expect(calls.resolve.mock.calls.at(-1)?.[0].document).toMatchObject(candidate);
  expect(readNoteReviewHistory(state.editor.prosemirrorState)?.baseline).toEqual(original);
});

it("still fences text history while an external Agent installation owns the editor", async () => {
  const state = await mounted(); await visibleDiff(state.host);
  await act(async () => state.editor.updateBlock("argument", { content: "Manual input" }));
  await act(async () => { await flushNoteReview(original.pageId); });
  await act(async () => holdTextEditor(original.pageId, "agent-install", true));
  await act(async () => { state.editor.undo(); });
  const fenced = body(state.port());
  await act(async () => holdTextEditor(original.pageId, "agent-install", false));
  await act(async () => { state.editor.undo(); await flushNoteReview(original.pageId); });
  expect(fenced).toBe("Manual input");
  expect(body(state.port())).toBe("Agent argument");
});

it("publishes only committed IME input to the review writer", async () => {
  const state = await mounted(); await visibleDiff(state.host); calls.resolve.mockClear();
  const surface = state.editor.domElement;
  if (!surface) throw new Error("Missing mounted editor");
  await act(async () => {
    surface.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    state.editor.updateBlock("argument", { content: "未完成的拼音" });
  });
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 400)); });
  const duringComposition = calls.resolve.mock.calls.length;
  await act(async () => {
    state.editor.updateBlock("argument", { content: "已完成的中文输入" });
    surface.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true }));
    await flushNoteReview(original.pageId);
  });
  expect(duringComposition).toBe(0);
  expect(calls.resolve).toHaveBeenCalledOnce();
  expect(body(state.port())).toBe("已完成的中文输入");
  expect(calls.resolve.mock.calls[0]![0].document.blocks[0].content.items[0].text).toBe("已完成的中文输入");
});

it("debounces manual review edits until three idle seconds while explicit flush remains immediate", async () => {
  const state = await mounted(); await visibleDiff(state.host); calls.resolve.mockClear();
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
  try {
    for (let index = 0; index < 5; index++) {
      await act(async () => state.editor.updateBlock("argument", { content: `Manual input ${index}` }));
      await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    }
    expect(calls.resolve).not.toHaveBeenCalled();
    await act(async () => { await vi.advanceTimersByTimeAsync(1999); });
    expect(calls.resolve).not.toHaveBeenCalled();
    await act(async () => { await vi.advanceTimersByTimeAsync(1); });
    expect(calls.resolve).toHaveBeenCalledOnce();
    await act(async () => state.editor.updateBlock("argument", { content: "Flush before switching" }));
    await act(async () => { await flushNoteReview(original.pageId); });
    expect(calls.resolve).toHaveBeenCalledTimes(2);
    await act(async () => { await vi.advanceTimersByTimeAsync(3000); });
    expect(calls.resolve).toHaveBeenCalledTimes(2);
  } finally { vi.useRealTimers(); }
});

it.each(["accept", "reject"] as const)("reopens the pending Agent Diff after undo reaches %s, without formal autosave on redo", async terminal => {
  const state = await mounted(null);
  await act(async () => { state.view(review()); notifyTextPatchesChanged(); }); await visibleDiff(state.host);
  const installationId = readNoteReviewHistory(state.editor.prosemirrorState)?.installationId;
  expect(installationId).toBeTypeOf("string"); state.onChange.mockClear();
  calls.resolve.mockImplementation(async (request: TextPatchResolution) => {
    const undone = request.action === "edit" || request.direction === "redo";
    const next = undone ? review(original, original, request.generation + 1, terminal) : review(candidate, original, request.generation + 1);
    state.view(next); return receipt(request, next);
  });
  for (let cycle = 0; cycle < 2; cycle++) {
    await act(async () => { expect(state.editor.undo()).toBe(true); });
    await vi.waitFor(() => expect(state.host.querySelector(".note-diff-chunk-actions")).toBeNull());
    expect(body(state.port())).toBe("Original argument");
    await act(async () => { expect(state.editor.redo()).toBe(true); }); await visibleDiff(state.host);
    expect(body(state.port())).toBe("Agent argument");
    expect(readNoteReviewHistory(state.editor.prosemirrorState)?.baseline).toEqual(original);
  }
  const requests = calls.resolve.mock.calls.map(([request]) => request as TextPatchResolution);
  expect(requests[0]).toMatchObject({ action: "edit", requestId: installationId });
  expect(requests.slice(1).map(request => [request.action, request.historyRequestId, request.direction])).toEqual([
    ["restore", installationId, "undo"], ["restore", installationId, "redo"], ["restore", installationId, "undo"],
  ]);
  expect(state.onChange).not.toHaveBeenCalled();
});

it("recovers the first Agent-installation undo with its original receipt before redoing the pending Diff", async () => {
  const state = await mounted(null);
  await act(async () => { state.view(review()); notifyTextPatchesChanged(); }); await visibleDiff(state.host);
  const installationId = readNoteReviewHistory(state.editor.prosemirrorState)?.installationId;
  calls.resolve.mockRejectedValueOnce({ status: "unknown", message: "Undo receipt unavailable" });
  await act(async () => { state.editor.undo(); });
  await vi.waitFor(() => expect(state.host.querySelector(".note-review-recovery")?.getAttribute("aria-label")).toContain("恢复原操作"));
  const first = calls.resolve.mock.calls[0]![0] as TextPatchResolution;
  expect(first).toMatchObject({ action: "edit", requestId: installationId });
  calls.resolve.mockImplementation(async (request: TextPatchResolution) => {
    const next = request.action === "edit" ? review(original, original, 2, "reject") : review(candidate, original, 3);
    state.view(next); return receipt(request, next);
  });
  await act(async () => button(state.host, "恢复原操作").click());
  await vi.waitFor(() => expect(getAppStorage().getItem(`note-inline-review:${original.pageId}`)).toBeNull());
  expect(calls.resolve.mock.calls[1]![0]).toEqual(first);
  await act(async () => { expect(state.editor.redo()).toBe(true); }); await visibleDiff(state.host);
  expect(calls.resolve.mock.calls[2]![0]).toMatchObject({ action: "restore", historyRequestId: installationId, direction: "undo" });
  expect(body(state.port())).toBe("Agent argument"); expect(state.onChange).not.toHaveBeenCalled();
});

it("preserves later D when a control RPC arrives and retains the admitted recovery identity", async () => {
  const state = await mounted(); await visibleDiff(state.host); state.onChange.mockClear();
  const operation = deferred<TextPatchResolutionReceipt>();
  calls.resolve.mockImplementation(() => operation.promise);
  await act(async () => button(state.host, "Undo all").click());
  await vi.waitFor(() => expect(calls.resolve).toHaveBeenCalledOnce());
  const admitted = calls.resolve.mock.calls[0]![0] as TextPatchResolution;
  // Simulates a transaction admitted immediately before the UI fence. The true
  // BlockNote instance changes; no native response or replacement editor does.
  await act(async () => state.editor.updateBlock("argument", { content: "Input after admission" }));
  await act(async () => operation.resolve(receipt(admitted, review(original, original, 2, "reject"))));
  expect(body(state.port())).toBe("Input after admission");
  expect(state.host.querySelector(".note-review-recovery")?.getAttribute("aria-label")).toContain("审阅期间出现了新的输入");
  expect(state.onChange).not.toHaveBeenCalled();
  const saved = JSON.parse(getAppStorage().getItem(`note-inline-review:${original.pageId}`)!);
  expect(saved.request.requestId).toBe(admitted.requestId);
  expect(saved.expectedDocument.blocks[0].content.items[0].text).toBe("Agent argument");
});

it("recovers an unknown candidate edit with its original request and resumes the private queue without formal autosave", async () => {
  const state = await mounted(); await visibleDiff(state.host); state.onChange.mockClear();
  calls.resolve.mockRejectedValueOnce({ status: "unknown", message: "Receipt unavailable" });
  await act(async () => state.editor.updateBlock("argument", { content: "Preserved first input" }));
  await act(async () => { await flushNoteReview(original.pageId).catch(() => {}); });
  await vi.waitFor(() => expect(state.host.querySelector(".note-review-recovery")?.getAttribute("aria-label")).toContain("恢复原操作"));
  const first = calls.resolve.mock.calls[0]![0] as TextPatchResolution;
  expect(first.action).toBe("edit"); expect(body(state.port())).toBe("Preserved first input");
  expect(getAppStorage().getItem(`note-inline-review:${original.pageId}`)).not.toBeNull();
  await act(async () => button(state.host, "恢复原操作").click());
  await vi.waitFor(() => expect(calls.resolve.mock.calls.length).toBe(2));
  expect(calls.resolve.mock.calls[1]![0]).toEqual(first);
  await vi.waitFor(() => expect(getAppStorage().getItem(`note-inline-review:${original.pageId}`)).toBeNull());
  await act(async () => state.editor.updateBlock("argument", { content: "Subsequent input" }));
  await act(async () => { await flushNoteReview(original.pageId); });
  expect(calls.resolve.mock.calls[2]![0].action).toBe("edit");
  expect(calls.resolve.mock.calls[2]![0].requestId).not.toBe(first.requestId);
  expect(body(state.port())).toBe("Subsequent input"); expect(state.onChange).not.toHaveBeenCalled();
});

it("undoes Keep all through the original editor history proof while D and the editor DOM stay unchanged", async () => {
  const state = await mounted(); await visibleDiff(state.host); state.onChange.mockClear();
  const dom = state.host.querySelector(".bn-editor"), before = body(state.port());
  let keepRequest: TextPatchResolution | undefined;
  calls.resolve.mockImplementation(async (request: TextPatchResolution) => {
    if (request.action === "keep") {
      keepRequest = request; const accepted = review(candidate, candidate, 2, "accept"); state.view(accepted); return receipt(request, accepted);
    }
    const pending = review(candidate, original, 3); state.view(pending); return receipt(request, pending);
  });
  await act(async () => button(state.host, "Keep all").click());
  await vi.waitFor(() => expect(state.host.querySelector(".note-diff-chunk-actions")).toBeNull());
  expect(readNoteReviewHistory(state.editor.prosemirrorState)?.baseline.blocks[0]?.content).toEqual(candidate.blocks[0]?.content);
  expect(state.host.querySelector(".bn-editor")).toBe(dom); expect(body(state.port())).toBe(before);
  await act(async () => { expect(state.editor.undo()).toBe(true); });
  await visibleDiff(state.host);
  const restore = calls.resolve.mock.calls.find(([request]) => request.action === "restore")?.[0] as TextPatchResolution | undefined;
  expect(restore?.historyRequestId).toBe(keepRequest?.requestId); expect(restore?.direction).toBe("undo");
  expect(readNoteReviewHistory(state.editor.prosemirrorState)?.baseline.blocks[0]?.content).toEqual(original.blocks[0]?.content);
  expect(state.host.querySelector(".bn-editor")).toBe(dom); expect(body(state.port())).toBe(before);
  expect(state.onChange).not.toHaveBeenCalled();
});

it("fences editor Undo during an unknown Keep and records the recovered original Keep as baseline history", async () => {
  const state = await mounted(); await visibleDiff(state.host); state.onChange.mockClear();
  const edited = documentValue("Edited agent argument"), dom = state.host.querySelector(".bn-editor");
  let keepRequest: TextPatchResolution | undefined, keepAttempts = 0;
  calls.resolve.mockImplementation(async (request: TextPatchResolution) => {
    if (request.action === "edit") {
      const pending = review(edited, original, 2); state.view(pending); return receipt(request, pending);
    }
    if (request.action === "keep") {
      keepRequest ??= request;
      if (++keepAttempts === 1) throw { status: "unknown", message: "Receipt unavailable" };
      const accepted = review(edited, edited, 3, "accept"); state.view(accepted); return receipt(request, accepted);
    }
    const pending = review(edited, original, 4); state.view(pending); return receipt(request, pending);
  });
  // Prior input supplies an actual undoable PM event; a pending Keep must not
  // let history consume that event while its native result is unknown.
  await act(async () => state.editor.updateBlock("argument", { content: "Edited agent argument" }));
  await act(async () => { await flushNoteReview(original.pageId); });
  await act(async () => button(state.host, "Keep all").click());
  await vi.waitFor(() => expect(state.host.querySelector(".note-review-recovery")?.getAttribute("aria-label")).toContain("恢复原操作"));
  const historyBefore = readNoteReviewHistory(state.editor.prosemirrorState);
  const callsBefore = calls.resolve.mock.calls.length;
  await act(async () => { state.editor.undo(); });
  expect(body(state.port())).toBe("Edited agent argument");
  expect(readNoteReviewHistory(state.editor.prosemirrorState)).toBe(historyBefore);
  expect(calls.resolve.mock.calls.length).toBe(callsBefore);
  await act(async () => button(state.host, "恢复原操作").click());
  await vi.waitFor(() => expect(state.host.querySelector(".note-diff-chunk-actions")).toBeNull());
  const keeps = calls.resolve.mock.calls.filter(([request]) => request.action === "keep");
  expect(keeps).toHaveLength(2); expect(keeps[1]![0]).toEqual(keeps[0]![0]);
  expect(readNoteReviewHistory(state.editor.prosemirrorState)?.historyProofRequestId).toBe(keepRequest?.requestId);
  expect(readNoteReviewHistory(state.editor.prosemirrorState)?.baseline.blocks[0]?.content).toEqual(edited.blocks[0]?.content);
  await act(async () => { expect(state.editor.undo()).toBe(true); });
  await visibleDiff(state.host);
  const restores = calls.resolve.mock.calls.filter(([request]) => request.action === "restore");
  expect(restores).toHaveLength(1);
  expect(restores[0]![0]).toMatchObject({ historyRequestId: keepRequest?.requestId, direction: "undo" });
  expect(readNoteReviewHistory(state.editor.prosemirrorState)?.baseline.blocks[0]?.content).toEqual(original.blocks[0]?.content);
  expect(body(state.port())).toBe("Edited agent argument"); expect(state.host.querySelector(".bn-editor")).toBe(dom);
  expect(state.onChange).not.toHaveBeenCalled();
});

it("recovers the independent baseline when a saved typing receipt is reopened before acknowledgment", async () => {
  const request: TextPatchResolution = { conversationId: "conversation", proposalId: "proposal", generation: 1,
    requestId: "interrupted-edit", pageId: original.pageId, action: "edit", document: candidate };
  getAppStorage().setItem(`note-inline-review:${original.pageId}`, JSON.stringify({ request, expectedDocument: candidate }));
  const state = await mounted(review(), candidate);
  expect(readNoteReviewHistory(state.editor.prosemirrorState)?.baseline).toEqual(candidate);
  await act(async () => button(state.host, "恢复原操作").click());
  await visibleDiff(state.host);
  expect(calls.resolve.mock.calls[0]![0]).toEqual(request);
  expect(readNoteReviewHistory(state.editor.prosemirrorState)?.baseline).toEqual(original);
  expect(body(state.port())).toBe("Agent argument");
  calls.resolve.mockImplementation(async (request: TextPatchResolution) => {
    const next = request.action === "keep" ? review(candidate, candidate, 3, "accept") : review(candidate, original, 4);
    state.view(next); return receipt(request, next);
  });
  await act(async () => button(state.host, "Keep all").click());
  await vi.waitFor(() => expect(state.host.querySelector(".note-diff-chunk-actions")).toBeNull());
  await act(async () => { expect(state.editor.undo()).toBe(true); });
  await visibleDiff(state.host);
  expect(readNoteReviewHistory(state.editor.prosemirrorState)?.baseline).toEqual(original);
});

it("reconciles a definitively failed typing save and drains newer input with a fresh request", async () => {
  const state = await mounted(); await visibleDiff(state.host);
  calls.resolve.mockRejectedValueOnce({ status: "failed", message: "temporary write failure" });
  await act(async () => state.editor.updateBlock("argument", { content: "First manual input" }));
  await act(async () => { await expect(flushNoteReview(original.pageId)).rejects.toBeDefined(); });
  const failed = calls.resolve.mock.calls[0]![0] as TextPatchResolution;
  expect(getAppStorage().getItem(`note-inline-review:${original.pageId}`)).toBeNull();
  await act(async () => state.editor.updateBlock("argument", { content: "Latest manual input" }));
  await act(async () => { await retryNoteReview(original.pageId); });
  const next = calls.resolve.mock.calls[1]![0] as TextPatchResolution;
  expect(next.requestId).not.toBe(failed.requestId);
  expect(next.document?.blocks[0]?.content).toMatchObject({ items: [{ text: "Latest manual input" }] });
  expect(state.host.querySelector('[role="status"]')?.getAttribute("aria-label")).toBe("已保存");
});

it("restores an uncertain request and saves input newer than its recovered receipt", async () => {
  const { saveNoteRecoveryDraft } = await import("../workspace/noteRecovery");
  const state = await mounted(); await visibleDiff(state.host);
  const saving = deferred<TextPatchResolutionReceipt>();
  calls.resolve.mockImplementationOnce(() => saving.promise);
  await act(async () => state.editor.updateBlock("argument", { content: "Earlier dispatched input" }));
  let flush!: Promise<void>;
  await act(async () => { flush = flushNoteReview(original.pageId); });
  await vi.waitFor(() => expect(calls.resolve).toHaveBeenCalledOnce());
  await act(async () => state.editor.updateBlock("argument", { content: "Latest input before close" }));
  await act(async () => { saving.reject({ status: "unknown", message: "response lost" }); await expect(flush).rejects.toBeDefined(); });
  const recovery = noteReviewRecovery(original.pageId)!;
  saveNoteRecoveryDraft(original.pageId, 1, recovery.document, recovery);
  await cleanup.pop()!();
  const reopened = await mounted(review(), recovery.document);
  expect(body(reopened.port())).toBe("Latest input before close");
  expect(readNoteReviewHistory(reopened.editor.prosemirrorState)?.baseline).toEqual(recovery.baseline);
  await act(async () => { await retryNoteReview(original.pageId); });
  expect(calls.resolve.mock.calls[1]![0]).toEqual(recovery.request);
  expect(calls.resolve.mock.calls[2]![0].document).toMatchObject(recovery.document);
  expect(body(reopened.port())).toBe("Latest input before close");
  expect(reopened.host.querySelector('[role="status"]')?.getAttribute("aria-label")).toBe("已保存");
});

it("treats a completed obsolete review as history, not as a conflict or an edit owner", async () => {
  const state = await mounted({ ...review(candidate, candidate, 2, "accept"), baselineMatches: false }, documentValue("Latest saved"));
  expect(body(state.port())).toBe("Latest saved"); expect(state.editor.isEditable).toBe(true);
  expect(state.host.querySelector(".note-save-state")?.getAttribute("aria-label")).toBe("已保存");
  await act(async () => state.editor.updateBlock("argument", { content: "New ordinary input" }));
  expect(state.onChange).toHaveBeenCalledOnce(); expect(calls.resolve).not.toHaveBeenCalled();
});
