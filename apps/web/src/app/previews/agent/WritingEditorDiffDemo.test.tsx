/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { EditorView } from "@codemirror/view";
import { undo } from "@codemirror/commands";
import { getChunks } from "@codemirror/merge";
import { afterEach, expect, it, vi } from "vitest";
import { WritingEditorDiffDemo } from "./WritingEditorDiffDemo";

vi.mock("../../../core/features/agent/presentation/WritingTextDiff", () => ({
  WritingTextDiff: () => <div data-testid="review-diff">review diff</div>,
}));

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root | null = null;
afterEach(() => { act(() => root?.unmount()); root = null; document.body.innerHTML = ""; });

async function mountDemo(): Promise<HTMLDivElement> {
  const host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => root?.render(<WritingEditorDiffDemo />));
  await vi.waitFor(async () => {
    await act(async () => { await Promise.resolve(); });
    expect(host.querySelector('[aria-label="编辑器 Diff 预览"] [aria-label="逐行差异编辑器"]')).not.toBeNull();
  }, { timeout: 5000 });
  return host;
}

it("renders production agent and workspace review components with mock gateways", async () => {
  const host = await mountDemo();
  const chat = host.querySelector(".agent-canvas-layer .writing-agent-panel");
  expect(chat).not.toBeNull();
  expect(chat?.closest('.writing-editor-diff-demo-chat-host')).not.toBeNull();
  expect(chat?.closest('.agent-canvas-shell')?.getAttribute('data-placement')).toBe("contained");
  expect(chat?.querySelector(".writing-agent-header")?.textContent).toBe("完善摘要");
  expect(chat?.querySelector('[aria-label="新建会话"]')).not.toBeNull();
  expect(chat?.querySelector('.agent-canvas-header-divider')).not.toBeNull();
  expect(chat?.querySelector('[aria-label="关闭当前对话"]')).toBeNull();
  // The showcase stays open while interacting with the editor or demo controls.
  const outside = host.querySelector<HTMLElement>(".writing-editor-diff-demo-header")!;
  await act(async () => {
    outside.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    outside.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    outside.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
    outside.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
    outside.click();
  });
  expect(host.querySelector('.writing-agent-panel[role="dialog"]')).toBe(chat);
  expect(host.textContent).toContain("帮我完善摘要部分");
  expect(chat?.querySelector('[aria-label="消息内嵌编辑器"]')).toBeNull();
  expect(host.querySelector('[aria-label="编辑器 Diff 预览"] [aria-label="逐行差异编辑器"]')).not.toBeNull();

  await vi.waitFor(async () => {
    await act(async () => { await Promise.resolve(); });
    expect(chat?.querySelector(".writing-agent-inline-diff")?.textContent).toContain("Edited 3 files");
  });
  const summary = chat?.querySelector(".writing-agent-inline-diff");
  expect(summary?.textContent).toContain("Edited 3 files");
  expect(summary?.querySelector('button')).not.toBeNull();
  expect(chat?.textContent).not.toContain("Keep all");
  const editor = host.querySelector('[aria-label="编辑器 Diff 预览"]');
  const keepAll = editor?.querySelector('[aria-label="Keep all"]');
  expect(keepAll).not.toBeNull();
  await act(async () => (keepAll as HTMLButtonElement).click());
  expect(editor?.querySelector('[aria-label="变更控制"]')).toBeNull();

  expect(summary?.textContent).not.toContain("View changes");
  const reviewButton = summary?.querySelector<HTMLButtonElement>(".writing-agent-inline-diff-list button");
  expect(reviewButton?.textContent).toContain("main.tex");
  const originalView = EditorView.findFromDOM(host.querySelector('[aria-label="编辑器 Diff 预览"] .cm-editor') as HTMLElement);
  const jump = vi.spyOn(HTMLElement.prototype, "scrollIntoView");
  await act(async () => (reviewButton as HTMLButtonElement).click());
  await vi.waitFor(() => expect(jump).toHaveBeenCalledWith({behavior: "smooth", block: "start"}));
  expect(document.querySelector('[aria-label="副本变更审阅"]')).toBeNull();
  expect(EditorView.findFromDOM(host.querySelector('[aria-label="编辑器 Diff 预览"] .cm-editor') as HTMLElement)).toBe(originalView);
});

it("preserves cursor and the same editor after the agent panel closes", async () => {
  const host = await mountDemo();
  const editor = host.querySelector('[aria-label="编辑器 Diff 预览"]');
  const keepAll = editor?.querySelector('[aria-label="Keep all"]');
  expect(keepAll).not.toBeNull();
  await act(async () => (keepAll as HTMLButtonElement).click());

  const diffEditor = host.querySelector('[aria-label="编辑器 Diff 预览"] .cm-editor');
  expect(diffEditor).not.toBeNull();
  const view = diffEditor instanceof HTMLElement ? EditorView.findFromDOM(diffEditor) : null;
  expect(view?.state.doc.toString()).toContain("This paper presents \\emph{Logical O-learning");
  expect(view).not.toBeNull();
  await act(async () => view?.dispatch({ selection: { anchor: 24 } }));
  const cursor = view?.state.selection.main.head;
  const hide = host.querySelector('[aria-label="隐藏 AI 写作助手"]');
  expect(hide).not.toBeNull();
  await act(async () => (hide as HTMLButtonElement).click());
  expect(view?.state.selection.main.head).toBe(cursor);

  const reopen = host.querySelector('[aria-label="打开对话"]');
  expect(reopen).not.toBeNull();
  await act(async () => (reopen as HTMLButtonElement).click());
  expect(host.querySelector('[aria-label="编辑器 Diff 预览"] .cm-editor')).toBe(diffEditor);
  expect(view?.state.selection.main.head).toBe(cursor);
  const reopenedDiffEditor = host.querySelector('[aria-label="编辑器 Diff 预览"] .cm-editor');
  expect(reopenedDiffEditor).not.toBeNull();
  const reopenedView = reopenedDiffEditor instanceof HTMLElement ? EditorView.findFromDOM(reopenedDiffEditor) : null;
  expect(reopenedView).toBe(view);
  expect(reopenedView?.state.selection.main.head).toBe(cursor);
  await act(async () => { if (view) expect(undo(view)).toBe(true); });
  expect(view && getChunks(view.state)?.chunks).toHaveLength(2);
  expect(editor?.querySelector('[aria-label="Keep all"]')).not.toBeNull();
});

it("toggles composer preparation through the production panel without losing its draft or editor", async () => {
  const host = await mountDemo();
  const composer = host.querySelector<HTMLTextAreaElement>('textarea[aria-label="给 AI 写作助手发送消息"]')!;
  await vi.waitFor(async () => {
    await act(async () => { await Promise.resolve(); });
    expect(composer.disabled).toBe(false);
  });
  const editor = host.querySelector('[aria-label="编辑器 Diff 预览"] .cm-editor');
  const toggle = () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(button =>
    button.textContent === "模拟准备中" || button.textContent === "恢复输入")!;
  await act(async () => toggle().click());
  expect(toggle().textContent).toBe("恢复输入");
  expect(toggle().getAttribute("aria-pressed")).toBe("true");
  expect(composer.disabled).toBe(true);
  expect(composer.placeholder).toBe("准备中……");
  expect(host.querySelector(".writing-agent-composer-status")).toBeNull();
  expect(host.querySelector(".writing-agent-composer-footer")?.textContent).not.toContain("正在读取对话历史");

  await act(async () => toggle().click());
  expect(composer.disabled).toBe(false);
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set?.call(composer, "保留模拟草稿");
    composer.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => toggle().click());
  expect(composer.disabled).toBe(true);
  expect(composer.value).toBe("保留模拟草稿");
  await act(async () => toggle().click());
  expect(toggle().getAttribute("aria-pressed")).toBe("false");
  expect(composer.disabled).toBe(false);
  expect(composer.placeholder).toBe("有什么可以帮您");
  expect(composer.value).toBe("保留模拟草稿");
  expect(host.querySelector(".writing-agent-composer-status")).toBeNull();
  expect(host.querySelector('[aria-label="编辑器 Diff 预览"] .cm-editor')).toBe(editor);
  expect(host.querySelector('textarea[aria-label="给 AI 写作助手发送消息"]')).toBe(composer);
});
