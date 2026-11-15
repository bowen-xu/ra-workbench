/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { webcrypto } from "node:crypto";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { WritingEditorDiffDemo } from "./WritingEditorDiffDemo";
import { WritingAgentPanel } from "../../../core/features/agent/presentation/WritingAgentPanel";
import { EditorView } from "@codemirror/view";
import { initialChanges } from "../../../core/features/agent/WritingDiffFixtures";

vi.mock("../../../core/features/agent/presentation/WritingTextDiff", () => ({ WritingTextDiff: ({ before, after }: { before: string; after: string }) =>
  <div data-frozen-diff><pre data-before>{before}</pre><pre data-after>{after}</pre></div> }));

const nativeDigest = webcrypto.subtle.digest.bind(webcrypto.subtle);
const native = vi.hoisted(() => ({ invoke: vi.fn(async () => { throw new Error("Preview must not invoke desktop I/O"); }) }));
vi.mock("@tauri-apps/api/core", async importOriginal => ({ ...await importOriginal<typeof import("@tauri-apps/api/core")>(), ...native, isTauri: () => true }));
let root: Root;
let host: HTMLDivElement;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  native.invoke.mockClear();
  vi.spyOn(crypto.subtle, "digest").mockImplementation((algorithm, data) => nativeDigest(algorithm, new Uint8Array(data as ArrayBuffer)));
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); });

it("places production file changes after response text and the turn footer after all response content", async () => {
  await act(async () => root.render(<WritingEditorDiffDemo />));
  await vi.waitFor(async () => {
    await act(async () => { await Promise.resolve(); });
    const card = host.querySelector(".writing-agent-inline-diff-card");
    expect(card?.textContent).toContain("Edited 3 files");
    const message = card?.closest(".writing-agent-message.assistant");
    expect(message).not.toBeNull();
    const bubble = message?.querySelector(".writing-agent-message-bubble");
    const summary = card?.closest('[aria-label="补丁结果"]');
    expect(summary).not.toBeNull();
    expect(summary?.querySelectorAll(".writing-agent-inline-diff-card")).toHaveLength(1);
    expect(bubble?.lastElementChild).toBe(summary);
    expect(bubble?.firstElementChild).not.toBe(summary);
    expect(bubble?.textContent).toContain("已准备 3 个文件的修改。");
    const footer = message?.querySelector(".writing-agent-message-meta");
    expect(message?.lastElementChild).toBe(footer);
    expect(footer?.querySelector("time")).not.toBeNull();
    expect(footer?.querySelector('[aria-label="复制消息"]')).not.toBeNull();
    expect(footer?.querySelector('[aria-label^="从此轮创建分支"]')).not.toBeNull();
    expect(card?.querySelector(".writing-agent-inline-diff-undo")).not.toBeNull();
    expect(host.querySelectorAll(".writing-agent-inline-diff-card")).toHaveLength(1);
  }, { timeout: 4000 });
  expect(native.invoke).not.toHaveBeenCalled();
});

it("attaches changes to the owning turn response and keeps the footer after asynchronously loaded content", async () => {
  const messages = [
    { id: "earlier", role: "assistant" as const, content: "Earlier reply" },
    { id: "user", role: "user" as const, content: "Edit the file" },
    { id: "tool", role: "assistant" as const, kind: "tool-result" as const, content: "{}" },
    { id: "latest", role: "assistant" as const, content: "Latest reply" },
  ];
  const render = (ready: boolean) => root.render(<WritingAgentPanel open messages={messages}
    turnAttachment={turnId => ready && turnId === "legacy:user" ? <section aria-label="File changes"><button type="button">Undo</button></section> : null}
    messageActions={id => id === "latest" ? <button type="button">Branch</button> : null} />);
  await act(async () => render(false));
  const message = host.querySelector('[data-agent-message-id="latest"]');
  const footer = message?.querySelector(".writing-agent-message-meta");
  await act(async () => render(true));
  expect(host.querySelector('[aria-label="File changes"]')?.closest("article")).toBe(message);
  expect(message?.querySelector(".writing-agent-message-bubble")?.lastElementChild?.getAttribute("aria-label")).toBe("File changes");
  expect(message?.lastElementChild).toBe(footer);
  expect(footer?.textContent).toContain("Branch");
  expect(host.querySelector('[data-agent-message-id="earlier"] [aria-label="File changes"]')).toBeNull();
});

it("retains changes when an ended turn has no assistant text", async () => {
  await act(async () => root.render(<WritingAgentPanel open
    messages={[{ id: "user", role: "user", content: "Edit the file" }]}
    turnAttachment={() => <section aria-label="File changes">Edited files</section>}
    historyActions={<button type="button">Branch empty turn</button>} />));
  const transcript = host.querySelector(".writing-agent-transcript");
  expect(transcript?.textContent).toContain("Edited files");
  const card = transcript?.querySelector('[aria-label="File changes"]');
  expect(card?.closest(".writing-agent-transcript")).toBe(transcript);
  const footer = transcript?.querySelector('[data-agent-history-actions]') ?? [...transcript!.querySelectorAll("button")].find(button => button.textContent === "Branch empty turn");
  expect(footer).toBeDefined();
  expect(Boolean(card && footer && card.compareDocumentPosition(footer) & Node.DOCUMENT_POSITION_FOLLOWING)).toBe(true);
});

it("keeps per-turn cards and compares the next modification with current text, including unconfirmed changes", async () => {
  await act(async () => root.render(<WritingEditorDiffDemo />));
  async function settle(check: () => void): Promise<void> {
    let failure: unknown;
    for (let attempt = 0; attempt < 150; attempt++) {
      await act(async () => { await new Promise(resolve => setTimeout(resolve, 40)); });
      try { check(); return; } catch (error) { failure = error; }
    }
    throw failure;
  }
  await settle(() => expect(host.querySelectorAll(".writing-agent-inline-diff-card")).toHaveLength(1));
  const original = host.querySelector(".writing-agent-inline-diff-card")!;
  await settle(() => expect(original.querySelector(".writing-agent-inline-diff-counts")?.textContent).toBe("+9 −3"));
  const originalMessage = original.closest("article");
  const input = host.querySelector<HTMLInputElement>('[aria-label="模拟运行时长（秒）"]')!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "0.5");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  const view = EditorView.findFromDOM(host.querySelector('[aria-label="编辑器 Diff 预览"] .cm-editor') as HTMLElement)!;
  await settle(() => expect(view.state.doc.toString()).toBe(initialChanges[0]!.after));
  // Also preserve a manual edit in the next turn's frozen starting text.
  await act(async () => view.dispatch({ changes: { from: view.state.doc.length, insert: "\n% manual" } }));
  const current = view.state.doc.toString();
  async function send(text: string) {
    const composer = host.querySelector<HTMLTextAreaElement>('textarea[aria-label="给 AI 写作助手发送消息"]')!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(composer, text);
      composer.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await settle(() => expect(host.querySelector<HTMLButtonElement>('[aria-label="发送消息"]')?.disabled).toBe(false));
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="发送消息"]')!.click());
  }
  await send("继续修改文件");
  await settle(() => expect(host.querySelector('[data-action="stop"]')).not.toBeNull());
  expect(host.querySelector(".writing-agent-inline-diff-card")).toBe(original);
  expect(original.closest("article")).toBe(originalMessage);
  const historicalRow = original.querySelector<HTMLButtonElement>(".writing-agent-inline-diff-list button")!;
  expect(historicalRow.disabled).toBe(false);
  expect(original.querySelector<HTMLButtonElement>(".writing-agent-inline-diff-undo")?.disabled).toBe(true);
  const jump = vi.spyOn(HTMLElement.prototype, "scrollIntoView");
  await act(async () => historicalRow.click());
  await vi.waitFor(() => expect(jump).toHaveBeenCalledWith({behavior: "smooth", block: "start"}));
  expect(document.querySelector("[data-frozen-diff], dialog")).toBeNull();
  expect(EditorView.findFromDOM(host.querySelector('[aria-label="编辑器 Diff 预览"] .cm-editor') as HTMLElement)).toBe(view);
  expect(view.state.doc.toString()).toBe(current);
  // This selector affects only future sends, not the admitted modification.
  const selector = host.querySelector<HTMLSelectElement>('[aria-label="模拟下一轮回复"]')!;
  await act(async () => { selector.value = "text"; selector.dispatchEvent(new Event("change", { bubbles: true })); });
  await settle(() => expect(host.querySelectorAll(".writing-agent-inline-diff-card")).toHaveLength(2));
  const second = host.querySelectorAll(".writing-agent-inline-diff-card")[1]!;
  await settle(() => expect(second.querySelector(".writing-agent-inline-diff-counts")?.textContent).toBe("+3 −0"));
  expect(second.closest("article")).not.toBe(originalMessage);
  expect(original.querySelector(".writing-agent-inline-diff-counts")?.textContent).toBe("+9 −3");
  const reviewButton = second.querySelector<HTMLButtonElement>(".writing-agent-inline-diff-list button")!;
  await settle(() => expect(reviewButton.disabled).toBe(false));
  jump.mockClear();
  await act(async () => reviewButton.click());
  await vi.waitFor(() => expect(jump).toHaveBeenCalledWith({behavior: "smooth", block: "start"}));
  expect(document.querySelector("[data-frozen-diff], dialog")).toBeNull();
  expect(EditorView.findFromDOM(host.querySelector('[aria-label="编辑器 Diff 预览"] .cm-editor') as HTMLElement)).toBe(view);
  expect(view.state.doc.toString()).toBe(`${current}\n% Mock Agent: 继续修改文件`);
  await send("只聊一下");
  await settle(() => expect(host.querySelector('[data-action="stop"]')).not.toBeNull());
  expect(host.querySelectorAll(".writing-agent-inline-diff-card")).toHaveLength(2);
  await settle(() => {
    expect(host.textContent).toContain("已收到消息。本轮只回复文本，不修改文件。");
    expect(host.querySelector('[data-agent-activity="thinking"]')).toBeNull();
    expect(host.querySelector('[aria-label="停止生成"]')).toBeNull();
  });
  expect(host.querySelectorAll(".writing-agent-inline-diff-card")).toHaveLength(2);
  expect(host.querySelector(".writing-agent-inline-diff-card")).toBe(original);
  expect(view.state.doc.toString()).toBe(`${current}\n% Mock Agent: 继续修改文件`);
  expect(native.invoke).not.toHaveBeenCalled();
}, 15000);
