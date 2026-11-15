/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { webcrypto } from "node:crypto";
import { afterEach, expect, it, vi } from "vitest";
import { applyAgentEvents, createAgentConversationProjection } from "@research-assistant/core/agent";
import { WritingDiffDemoStorage } from "./WritingDiffDemoStorage";
import { WritingDiffDemoAgentStorage } from "./WritingDiffDemoAgentStorage";
import { WritingDiffDemoSubmission } from "./WritingDiffDemoSubmission";
import { projectConversationMessages } from "../../../core/features/agent/projectConversationMessages";
import { WritingAgentTranscript } from "../../../core/features/agent/presentation/WritingAgentTranscript";
import { initialChanges } from "../../../core/features/agent/WritingDiffFixtures";
import type { WritingDemoPatchFailure } from "./WritingDemoResponseSequence";

const manuscriptId = "00000000-0000-4000-8000-000000000003";
const conversationId = "00000000-0000-4000-8000-000000000001";
const live: { execution: WritingDiffDemoSubmission; agent: WritingDiffDemoAgentStorage }[] = [];
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
afterEach(() => {
  for (const f of live.splice(0)) { f.execution.dispose(); f.agent.history.dispose(); }
  vi.useRealTimers(); vi.restoreAllMocks();
});
async function fixture(failure: WritingDemoPatchFailure, progressive: boolean) {
  const digest = webcrypto.subtle.digest.bind(webcrypto.subtle);
  vi.spyOn(crypto.subtle, "digest").mockImplementation((algorithm, data) => digest(algorithm, new Uint8Array(data as ArrayBuffer)));
  const storage = new WritingDiffDemoStorage(manuscriptId, initialChanges);
  const agent = new WritingDiffDemoAgentStorage(storage, undefined);
  const execution = new WritingDiffDemoSubmission(storage, agent, manuscriptId);
  live.push({ execution, agent });
  execution.patchFailure = failure; execution.progressiveFeedback = progressive; execution.delayMs = 90;
  execution.setTextPacing({ charactersPerChunk: 1000, intervalMs: 10 });
  const files = await storage.files.listFiles(manuscriptId);
  const buffers = await Promise.all(files.map(async file => ({ path: file.relativePath, expectedDiskSha256: file.sha256,
    sessionId: crypto.randomUUID(), revision: crypto.randomUUID(), text: (await storage.files.readFile(manuscriptId, file.relativePath)).content })));
  const start = async () => {
    const preview = await execution.submission.prepareWriting({ conversationId, requestId: crypto.randomUUID(), text: "修改稿件",
      context: { manuscriptId, files: files.map(file => ({ relativePath: file.relativePath, expectedDiskSha256: file.sha256 })) } });
    const scope = { manuscriptId, conversationId, requestId: preview.requestId, contextSnapshotId: preview.contextSnapshotId };
    await execution.workspace.prepare({ scope, buffers });
    await execution.submission.startWriting({ ...scope, model: preview.input.model, mode: "deepSeekNonThinking" });
    return { scope, turnId: preview.turnId };
  };
  vi.useFakeTimers();
  const admitted = await start();
  const read = async () => projectConversationMessages(applyAgentEvents(createAgentConversationProjection(conversationId),
    (await agent.history.gateway.readHistory({ conversationId, afterSeq: 0, limit: 500 })).events));
  return { storage, agent, execution, start, read, ...admitted };
}

it.each([false, true])("reports a tool error, verifies unchanged text, and ends without a change card (streaming %s)", async progressive => {
  const f = await fixture("persistent", progressive);
  // Changing the selector cannot rewrite an already admitted model/tool scenario.
  f.execution.patchFailure = undefined;
  await vi.advanceTimersByTimeAsync(5000);
  expect((await f.agent.reviews.review(f.scope)).changes).toEqual([]);
  const messages = await f.read();
  const error = messages.find(message => message.toolError);
  expect(error?.toolName).toBe("research_apply_patch");
  expect(error?.content).toContain('"code":"file_write_failed"');
  const final = messages.find(message => message.phase === "finalAnswer");
  expect(final?.content).toContain("本轮修改失败");
  expect(final?.turnOutcome).toBe("completed");
  const calls = messages.filter(message => message.kind === "tool-call");
  expect(calls.map(message => message.toolName)).toEqual(["research_read_file", "research_apply_patch", "research_read_file"]);
  const results = messages.filter(message => message.kind === "tool-result" && message.toolName === "research_read_file");
  expect(results[1]?.content).toBe(results[0]?.content);
  for (const change of initialChanges) expect((await f.storage.files.readFile(manuscriptId, change.path)).content).toBe(change.before);

  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  try {
    await act(async () => root.render(<WritingAgentTranscript messages={messages} isRunning={false} />));
    expect(host.textContent).toContain("本轮修改失败");
    expect(host.querySelector('[aria-label="补丁结果"]')).toBeNull();
    expect(host.querySelector('[aria-label="消息中的文件改动"]')).toBeNull();
    expect(host.textContent).not.toContain("已准备");
    // A handled tool error is not an unresolved turn or admission lock.
    expect(f.agent.history.activeTurn(conversationId)).toBeNull();
    const next = await f.start();
    await act(async () => { await vi.advanceTimersByTimeAsync(5000); await f.agent.reviews.review(next.scope); });
    expect(f.agent.history.activeTurn(conversationId)).toBeNull();
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it("retries a known unwritten patch only once and publishes only the successful modification", async () => {
  const f = await fixture("once", true);
  await vi.advanceTimersByTimeAsync(5000);
  const review = await f.agent.reviews.review(f.scope), messages = await f.read();
  const calls = messages.filter(message => message.kind === "tool-call" && message.toolName === "research_apply_patch");
  expect(calls).toHaveLength(2);
  expect(calls[1]?.content).toBe(calls[0]?.content);
  expect(messages.filter(message => message.toolError)).toHaveLength(1);
  const results = messages.filter(message => message.kind === "tool-result" && message.toolName === "research_apply_patch");
  expect(JSON.parse(results[1]!.content).targets[0].path).toBe(JSON.parse(results[0]!.content).targets[0].path);
  expect(messages.find(message => message.phase === "finalAnswer")?.content).toBe("检查通过，本轮修改已完成。");
  expect(review.changes).toHaveLength(1);
  // The fixture publishes an execution-copy result; production delivery still owns formal writes.
  expect((await f.storage.files.readFile(manuscriptId, "main.tex")).content).toBe(initialChanges[0]!.before);
});
