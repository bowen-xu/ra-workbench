// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { AgentContextPreview } from "@research-assistant/core/agent";
import { WritingDiffDemoHistory } from "../../previews/agent/WritingDiffDemoHistory";
import { createMockTextPatchGateway, TEXT_PATCH_PREVIEW_CONVERSATION, TEXT_PATCH_PREVIEW_PAGE } from "./mockTextPatchGateway";
import { textPatchPreviewResponse } from "./textPatchAppFixture";

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());
function fixture(text: string) {
  const patch = createMockTextPatchGateway(true);
  const install = vi.fn(() => undefined);
  patch.bindEditor({ isInstalling: false, read: () => patch.snapshot().document,
    prepare: () => install, prepareStructure: () => install, dispose() {} });
  const history = new WritingDiffDemoHistory();
  const preview: AgentContextPreview = { schemaVersion: 1, conversationId: TEXT_PATCH_PREVIEW_CONVERSATION,
    requestId: crypto.randomUUID(), turnId: crypto.randomUUID(), contextSnapshotId: crypto.randomUUID(),
    input: { text, model: {providerId: "fixture", modelId: "note-preview", revision: "1"}, reasoningEffort: null, context: [] } };
  const admission = { scope: { manuscriptId: "independent", conversationId: preview.conversationId,
    requestId: preview.requestId, contextSnapshotId: preview.contextSnapshotId }, turnId: preview.turnId };
  history.start(admission, text, preview.input.model);
  const controller = new AbortController();
  const response = textPatchPreviewResponse(patch);
  const run = () => response.run(preview, history, admission, controller.signal);
  const events = async () => (await history.gateway.readHistory({conversationId: preview.conversationId, afterSeq: 0, limit: 100})).events.map(event => event.payload);
  return { patch, install, controller, run, events };
}

it("executes a typed combined request through read and patch events and leaves two editable review chunks", async () => {
  const f = fixture("请润色这篇笔记并补充验证清单");
  const initial = f.patch.snapshot().document;
  const operation = f.run();
  await vi.runAllTimersAsync(); await operation;
  const events = await f.events();
  const calls = events.flatMap(event => event.type === "itemStarted" && event.kind.type === "toolCall" ? [event.kind.name] : []);
  expect(calls).toEqual(["research_read_file", "research_apply_patch"]);
  expect(f.patch.read("saved")).toContain("累积局部证据更新判断");
  expect(f.patch.read("saved")).toContain("下一步验证");
  expect(f.patch.snapshot().review?.pages[0]?.chunks).toHaveLength(2);
  expect(f.patch.snapshot().review?.pages[0]?.reviewDocument).toEqual(initial);
  expect(f.install).toHaveBeenCalledOnce();
  // Use the existing review gateway to Keep one chunk and Undo the other.
  let review = f.patch.snapshot().review!;
  await f.patch.resolve({ conversationId: TEXT_PATCH_PREVIEW_CONVERSATION, proposalId: review.proposalId,
    pageId: TEXT_PATCH_PREVIEW_PAGE, generation: review.generation, requestId: crypto.randomUUID(),
    action: "keep", chunkId: review.pages[0]!.chunks![0]!.id });
  review = f.patch.snapshot().review!;
  await f.patch.resolve({ conversationId: TEXT_PATCH_PREVIEW_CONVERSATION, proposalId: review.proposalId,
    pageId: TEXT_PATCH_PREVIEW_PAGE, generation: review.generation, requestId: crypto.randomUUID(), action: "undo" });
  expect(f.patch.read("saved")).toContain("累积局部证据更新判断");
  expect(f.patch.read("saved")).not.toContain("下一步验证");
  expect(f.patch.snapshot().review?.pages[0]?.chunks).toHaveLength(0);
});

it("reads a note without modifying it when the user explicitly requests read-only", async () => {
  const f = fixture("不要改写研究问题，只读取并总结笔记");
  const initial = f.patch.snapshot().document;
  const operation = f.run(); await vi.runAllTimersAsync(); await operation;
  expect(f.patch.snapshot().document).toBe(initial);
  expect(f.install).not.toHaveBeenCalled();
  expect((await f.events()).filter(event => event.type === "itemStarted" && event.kind.type === "toolCall")).toHaveLength(1);
});

it("does not partially save a combined patch when one requested edit no longer applies", async () => {
  const f = fixture("改写研究问题");
  await f.patch.simulate("rewrite");
  const before = f.patch.snapshot().document;
  await expect(f.patch.simulate(["checklist", "rewrite"])).rejects.toThrow("已改写");
  expect(f.patch.snapshot().document).toBe(before);
  expect(f.patch.read("saved")).not.toContain("下一步验证");
  expect(f.install).toHaveBeenCalledOnce();
});

it("does not fabricate a patch for an unsupported chat message", async () => {
  const f = fixture("你好"); await f.run();
  expect(f.install).not.toHaveBeenCalled();
  expect(f.patch.snapshot().review).toBeNull();
  expect((await f.events()).some(event => event.type === "textDelta" && event.text.includes("预设工具结果"))).toBe(true);
});

it("stops before writing when interrupted during the staged tool process", async () => {
  const f = fixture("改写研究问题");
  const operation = f.run(); await vi.advanceTimersByTimeAsync(700);
  f.controller.abort(); await vi.runAllTimersAsync(); await operation;
  expect(f.install).not.toHaveBeenCalled();
  expect(f.patch.snapshot().review).toBeNull();
});

it("reports a failed tool and rejects the run so the execution owner marks failure", async () => {
  const f = fixture("改写研究问题");
  const failure = new Error("模拟保存失败（fixture_save_failed）");
  vi.spyOn(f.patch, "simulate").mockRejectedValueOnce(failure);
  const result = f.run().catch(error => error);
  await vi.runAllTimersAsync(); expect(await result).toBe(failure);
  expect(f.patch.snapshot().review).toBeNull();
  expect((await f.events()).some(event => event.type === "textDelta" && event.text.includes("fixture_save_failed"))).toBe(true);
});
