/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { EditorView } from "@codemirror/view";
import { redo, undo } from "@codemirror/commands";
import { getChunks, getOriginalDoc } from "@codemirror/merge";
import { createHash } from "node:crypto";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { WritingEditorPane } from "../../../../../../plugins/writing_projects/src/features/writing/editor/WritingEditorPane";
import { WritingFileSessions } from "../../../../../../plugins/writing_projects/src/features/writing/editor/writingFileSessions";
import { WritingEditorDiffDemo } from "./WritingEditorDiffDemo";
import { EDITOR_AUTOSAVE_IDLE_MS } from "../../../core/services/editorAutosave";
import { initialChanges } from "../../../core/features/agent/WritingDiffFixtures";

const feedback = vi.hoisted(() => ({ error: vi.fn(), warning: vi.fn() }));
vi.mock("../../../core/services/notifications", () => ({ replayVisibleNotifications: vi.fn(), notify: feedback }));

// Keep this integration test focused on the real editor and diff lifecycle.
// The runtime panel is a large async surface whose transcript is already
// covered by its own tests; the demo still supplies its production editor,
// summary and the shared LatexEditor diff mode below.
vi.mock("../../../../../../plugins/writing_projects/src/features/writing/WritingAgentRuntimePanel", () => ({
  WritingAgentRuntimePanel: () => <div data-testid="mock-runtime-panel" />,
}));

// Gateway history behavior has its own tests; this suite mounts the production editor.
vi.mock("../../../../../../plugins/writing_projects/src/features/writing/agent/WritingWorkspaceReview", () => ({
  WritingWorkspaceChangeSummary: () => <div data-testid="mock-change-summary" />,
}));

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root | null = null;
let host: HTMLDivElement | null = null;

// Keep mock I/O acknowledgement deterministic within React's act boundary.
// Hash bytes are identical to WebCrypto; no editor or persistence controller is mocked.
beforeEach(() => vi.spyOn(crypto.subtle, "digest").mockImplementation(async (_algorithm, data) =>
  Uint8Array.from(createHash("sha256").update(new Uint8Array(data as ArrayBuffer)).digest()).buffer));

afterEach(() => {
  act(() => root?.unmount());
  root = null;
  host?.remove();
  host = null;
  vi.restoreAllMocks();
  vi.clearAllMocks();
});

async function mountDemo(): Promise<EditorView> {
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => root?.render(<WritingEditorDiffDemo />));
  await vi.waitFor(async () => {
    await act(async () => { await Promise.resolve(); });
    expect(host?.querySelector(".writing-editor-host .cm-editor")).not.toBeNull();
  });
  return currentView();
}
function restartButton(): HTMLButtonElement {
  return [...document.querySelectorAll("button")].find((button) => button.textContent === "模拟退出并重开")!;
}
async function waitForRestart(previous: EditorView): Promise<EditorView> {
  await vi.waitFor(async () => {
    await act(async () => { await Promise.resolve(); });
    expect(host?.querySelector(".writing-editor-host .cm-editor")).not.toBeNull();
    expect(currentView() === previous).toBe(false);
  });
  return currentView();
}
function button(label: string): HTMLButtonElement {
  const found = [...document.querySelectorAll("button")].find((candidate) => candidate.textContent === label);
  if (!found) throw new Error(`Expected button ${label}`);
  return found;
}
async function externalConflict(view: EditorView): Promise<void> {
  await act(async () => button("模拟外部文件修改").click());
  vi.useFakeTimers();
  try {
    await act(async () => view.dispatch({ changes: { from: 0, insert: "% buffered edit\n" }, userEvent: "input.type" }));
    await act(async () => vi.advanceTimersByTimeAsync(EDITOR_AUTOSAVE_IDLE_MS));
  } finally { vi.useRealTimers(); }
  await vi.waitFor(async () => {
    await act(async () => { await Promise.resolve(); });
    expect(document.querySelector('[aria-label="解决稿件文件冲突"]')).not.toBeNull();
  }, { timeout: 3000 });
}
it("routes external edits to the production conflict dialog and preserves the editor when continuing", async () => {
  const view = await mountDemo();
  await externalConflict(view);
  const buffered = view.state.doc.toString(), baseline = getOriginalDoc(view.state).toString();
  expect(document.querySelector(".writing-conflict-comparison")?.textContent).toContain("% changed outside editor");
  await act(async () => button("继续编辑").click());
  expect(currentView()).toBe(view);
  expect(view.state.doc.toString()).toBe(buffered);
  expect(getOriginalDoc(view.state).toString()).toBe(baseline);
  expect(document.querySelector('[aria-label="解决稿件文件冲突"]')).toBeNull();
});
it("reloads the chosen disk text and keeps that choice through production restart recovery", async () => {
  const view = await mountDemo();
  await externalConflict(view);
  await act(async () => button("重新载入磁盘版本").click());
  expect(currentView()).toBe(view);
  expect(view.state.doc.toString()).toBe("% changed outside editor\n");
  expect(getOriginalDoc(view.state).toString()).toBe(view.state.doc.toString());
  await act(async () => restartButton().click());
  const restored = await waitForRestart(view);
  expect(restored.state.doc.toString()).toBe("% changed outside editor\n");
  expect(document.querySelector('[aria-label="解决稿件文件冲突"]')).toBeNull();
});
it("saves a conflict copy in the real controller and supports subsequent edits and restart", async () => {
  const view = await mountDemo();
  await externalConflict(view);
  const buffered = view.state.doc.toString();
  await act(async () => button("保留缓冲并另存").click());
  await vi.waitFor(async () => {
    await act(async () => { await Promise.resolve(); });
    expect(host?.querySelector(".writing-editor-status")?.textContent).toContain("main-conflict-copy.tex");
  });
  const copy = currentView();
  expect(copy.state.doc.toString()).toBe(buffered);
  await act(async () => copy.dispatch({ changes: { from: 0, insert: "% copy edit\n" }, userEvent: "input.type" }));
  const updated = copy.state.doc.toString();
  await act(async () => restartButton().click());
  const restored = await waitForRestart(copy);
  expect(restored.state.doc.toString()).toBe(updated);
  expect(host?.querySelector(".writing-editor-status")?.textContent).toContain("main-conflict-copy.tex");
  expect(document.querySelector(".writing-conflict-comparison")?.textContent).toContain("% changed outside editor");
});
it("keeps the buffer and dialog available when saving a conflict copy fails", async () => {
  const view = await mountDemo();
  await externalConflict(view);
  const buffered = view.state.doc.toString();
  const select = host!.querySelector<HTMLSelectElement>('[aria-label="模拟故障"]')!;
  await act(async () => { select.value = "file"; select.dispatchEvent(new Event("change", { bubbles: true })); });
  await act(async () => button("保留缓冲并另存").click());
  expect(feedback.error).toHaveBeenCalledWith("模拟正式文件保存失败");
  expect(button("保留缓冲并另存").disabled).toBe(false);
  expect(document.querySelector('[aria-label="解决稿件文件冲突"]')).not.toBeNull();
  expect(currentView()).toBe(view);
  expect(view.state.doc.toString()).toBe(buffered);
  await act(async () => { select.value = "none"; select.dispatchEvent(new Event("change", { bubbles: true })); });
  await act(async () => button("保留缓冲并另存").click());
  expect(document.querySelector('[aria-label="解决稿件文件冲突"]')).toBeNull();
  expect(currentView().state.doc.toString()).toBe(buffered);
});
it("restores a kept file through production persistence after simulated exit without reinstalling fixture history", async () => {
  const view = await mountDemo();
  const kept = view.state.doc.toString();
  await act(async () => (host?.querySelector('[aria-label="Keep all"]') as HTMLButtonElement).click());
  expect(getChunks(view.state)?.chunks).toHaveLength(0);
  await act(async () => restartButton().click());
  const restored = await waitForRestart(view);
  expect(restored.state.doc.toString()).toBe(kept);
  expect(getChunks(restored.state)?.chunks ?? []).toHaveLength(0);
  // A second exit cannot revive the initial fixture completion either.
  await act(async () => restartButton().click());
  const next = await waitForRestart(restored);
  expect(next.state.doc.toString()).toBe(kept);
  expect(host?.querySelector('[aria-label="Keep all"]')).toBeNull();
});
it.each(["fileReceipt", "checkpointReceipt"])("restores Keep through the preview fault control after a committed reply is lost: %s", async fault => {
  const view = await mountDemo();
  // Finish initial delivery before arming the next storage reply fault.
  await act(async () => restartButton().click());
  const ready = await waitForRestart(view);
  const select = host!.querySelector<HTMLSelectElement>('[aria-label="模拟故障"]')!;
  await act(async () => { select.value = fault; select.dispatchEvent(new Event("change", { bubbles: true })); });
  await act(async () => (host?.querySelector('[aria-label="Keep all"]') as HTMLButtonElement).click());
  const kept = ready.state.doc.toString();
  await vi.waitFor(async () => {
    await act(async () => { await Promise.resolve(); });
    expect(feedback.error.mock.calls.some(([message]) => message.includes("已写入但回执丢失"))).toBe(true);
  });
  await act(async () => restartButton().click());
  const restored = await waitForRestart(ready);
  expect(restored.state.doc.toString()).toBe(kept);
  expect(getOriginalDoc(restored.state).toString()).toBe(kept);
  expect(getChunks(restored.state)?.chunks ?? []).toHaveLength(0);
  expect(document.querySelector('[aria-label="解决稿件文件冲突"]')).toBeNull();
});
it("does not open blank file sessions while resetting mock storage", async () => {
  const view = await mountDemo();
  await act(async () => (host?.querySelector('[aria-label="Keep all"]') as HTMLButtonElement).click());
  await act(async () => ([...document.querySelectorAll("button")].find((button) => button.textContent === "重置")!).click());
  const reset = await waitForRestart(view);
  expect(reset.state.doc.toString()).toBe(initialChanges[0]!.after);
  expect(getOriginalDoc(reset.state).toString()).toBe(initialChanges[0]!.before);
  expect(getChunks(reset.state)?.chunks).toHaveLength(2);
});
it("blocks preview exit on dual storage failure and restores kept text after a reliable-checkpoint fallback", async () => {
  const view = await mountDemo();
  const select = host!.querySelector<HTMLSelectElement>('[aria-label="模拟故障"]')!;
  await act(async () => { select.value = "both"; select.dispatchEvent(new Event("change", { bubbles: true })); });
  await act(async () => view.dispatch({ changes: { from: 0, insert: "% local edit\n" }, userEvent: "input.type" }));
  await act(async () => (host?.querySelector('[aria-label="Keep all"]') as HTMLButtonElement).click());
  const kept = view.state.doc.toString();
  await act(async () => restartButton().click());
  await vi.waitFor(async () => {
    await act(async () => { await Promise.resolve(); });
    expect(feedback.error).toHaveBeenCalledWith(expect.stringContaining("未能可靠暂存"));
  });
  expect(currentView()).toBe(view); expect(view.state.doc.toString()).toBe(kept);
  await act(async () => { select.value = "file"; select.dispatchEvent(new Event("change", { bubbles: true })); });
  await act(async () => restartButton().click());
  const restored = await waitForRestart(view);
  expect(restored.state.doc.toString()).toBe(kept);
  expect(getChunks(restored.state)?.chunks ?? []).toHaveLength(0);
  expect(feedback.warning).toHaveBeenCalledWith(expect.stringContaining("可靠暂存"));
  expect(restored.state.doc.toString()).not.toBe(initialChanges[0]!.before);
});

function files(before: string, after: string): WritingFileSessions {
  const sessions = new WritingFileSessions();
  const file = sessions.open({ manuscriptId: "demo", relativePath: "main.tex" }, before);
  file.applyAgent("operation", file.read().version, after);
  return sessions;
}

function pane(props: {
  readonly content: string;
  readonly fileSessions: WritingFileSessions;
  readonly onChange: (value: string) => void;
}): React.JSX.Element {
  return <WritingEditorPane
    activeManuscriptId="demo"
    activePath="main.tex"
    completionData={{ citationKeys: [], files: ["main.tex"], labels: [] }}
    content={props.content}
    cursorOffset={0}
    diagnostics={[]}
    focusRequest={undefined}
    insertionRequest={undefined}
    openPaths={["main.tex"]}
    saveState="saved"
    onChange={props.onChange}
    onClose={vi.fn()}
    onCompile={vi.fn()}
    onOpenCommands={vi.fn()}
    onCursorChange={vi.fn()}
    onSelect={vi.fn()}
    preferences={{ fontSize: 14, lineWrapping: true, showLineNumbers: true, tabSize: 2 }}
    onInsertCitation={vi.fn()}
    syncActions={{ forward: vi.fn(), backward: vi.fn(), forwardDisabled: undefined, backwardDisabled: undefined }}
    codeCheckEnabled={false}
    hasCodeCheckErrors={false}
    onCodeCheckResult={vi.fn()}
    fileSessions={props.fileSessions}
  />;
}

function currentView(): EditorView {
  const element = host?.querySelector(".writing-editor-host .cm-editor");
  const view = element instanceof HTMLElement ? EditorView.findFromDOM(element) : null;
  if (!view) throw new Error("Expected embedded diff EditorView");
  return view;
}

it("keeps one real diff EditorView while edits, Keep, and parent rerenders share undo/redo history", async () => {
  host = document.createElement("div");
  document.body.append(host);
  let value = "agent";
  const onChange = (next: string): void => { value = next; };
  const sessions = files("old", value);
  root = createRoot(host);
  await act(async () => root?.render(pane({ content: value, fileSessions: sessions, onChange })));

  const view = currentView();
  expect(view.state.doc.toString()).toBe("agent");
  expect(getOriginalDoc(view.state).toString()).toBe("old");
  expect(getChunks(view.state)?.chunks).toHaveLength(1);
  const retainedSource = host.querySelector<HTMLDivElement>('.writing-editor-host > .writing-retained-source');
  expect(retainedSource?.hidden).toBe(false);
  expect(retainedSource?.querySelector('.writing-text-diff [aria-label="逐行差异编辑器"]')).not.toBeNull();
  expect(host.querySelector('[aria-label="应用当前变更"]')).not.toBeNull();
  expect(host.querySelector('[aria-label="撤销当前变更"]')).not.toBeNull();
  expect(host.querySelector('[aria-label="第 1 处变更操作"]')).not.toBeNull();

  await act(async () => view.dispatch({ changes: { from: view.state.doc.length, insert: " edit" } }));
  expect(value).toBe("agent edit");
  await act(async () => root?.render(pane({ content: value, fileSessions: sessions, onChange })));
  expect(currentView()).toBe(view);

  await act(async () => undo(view));
  expect(view.state.doc.toString()).toBe("agent");
  await act(async () => undo(view));
  expect(view.state.doc.toString()).toBe("old");
  expect(getOriginalDoc(view.state).toString()).toBe("old");
  await act(async () => redo(view));
  expect(view.state.doc.toString()).toBe("agent");
  await act(async () => redo(view));
  expect(view.state.doc.toString()).toBe("agent edit");

  await act(async () => (host?.querySelector('[aria-label="应用当前变更"]') as HTMLButtonElement).click());
  expect(getChunks(view.state)?.chunks).toHaveLength(0);
  expect(value).toBe("agent edit");
  // Bulk resolution is owned by the shared editor. It must not fall through
  // to the legacy proposal application callback after the local transaction.
  await act(async () => undo(view));
  expect(getChunks(view.state)?.chunks).toHaveLength(1);
  expect(view.state.doc.toString()).toBe("agent edit");
  expect(currentView()).toBe(view);
});

it("resolves all chunks in the shared editor without dispatching the legacy host action", async () => {
  host = document.createElement("div");
  document.body.append(host);
  const sessions = files("old", "agent");
  let value = "sentinel";
  root = createRoot(host);
  await act(async () => root?.render(pane({ content: value, fileSessions: sessions, onChange: (next) => { value = next; } })));
  const view = currentView();
  await act(async () => (host?.querySelector('[aria-label="Keep all"]') as HTMLButtonElement).click());
  expect(getChunks(view.state)?.chunks).toHaveLength(0);
  expect(value).toBe("agent");
  expect(host.querySelector('[aria-label="变更控制"]')).toBeNull();
  await act(async () => undo(view));
  expect(getChunks(view.state)?.chunks).toHaveLength(1);
});

it("keeps file-owned review decisions when a later conversation edits the current text", () => {
  const sessions = files("original", "first conversation");
  const file = sessions.get({ manuscriptId: "demo", relativePath: "main.tex" })!;
  file.resolveAll("keep");
  expect(file.applyAgent("conversation-b", file.read().version, "second conversation")).toBe("applied");
  expect(file.read()).toMatchObject({ reviewBaseline: "first conversation", content: "second conversation" });
  expect(sessions.open(file.identity, "stale historical candidate")).toBe(file);
  expect(file.read().content).toBe("second conversation");
});

it("does not replace the demo's real diff EditorView when an edit updates mock data", async () => {
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  await act(async () => root?.render(<WritingEditorDiffDemo />));
  // The summary first resolves its inventory and then its selected file. Each
  // continuation is deterministic and does not depend on wall-clock timing.
  for (let attempt = 0; attempt < 12 && !host.querySelector(".writing-text-diff .cm-editor"); attempt += 1) {
    await act(async () => { await Promise.resolve(); });
  }
  const view = currentView();
  const before = view.state.doc.toString();
  await act(async () => view.dispatch({ changes: { from: view.state.doc.length, insert: "\n% manual edit" } }));
  for (let attempt = 0; attempt < 4; attempt += 1) await act(async () => { await Promise.resolve(); });
  expect(currentView()).toBe(view);
  expect(view.state.doc.toString()).toBe(`${before}\n% manual edit`);
  await act(async () => undo(view));
  expect(view.state.doc.toString()).toBe(before);
  await act(async () => redo(view));
  expect(view.state.doc.toString()).toBe(`${before}\n% manual edit`);
});
