import { afterEach, expect, it, vi } from "vitest";
import type { AgentWritingQueuedCompactionPrepareRequest } from "@research-assistant/core/agent";
import { WritingDiffDemoStorage } from "./WritingDiffDemoStorage";
import { WritingDiffDemoAgentStorage } from "./WritingDiffDemoAgentStorage";
import { createWritingDiffDemoRuntime } from "./writingDiffDemoRuntime";
import { initialChanges } from "../../../core/features/agent/WritingDiffFixtures";

const manuscriptId = "11111111-1111-4111-8111-111111111111";
const conversationId = "22222222-2222-4222-8222-222222222222";
const cleanups: (() => void)[] = [];
afterEach(() => { for (const cleanup of cleanups.splice(0)) cleanup(); vi.restoreAllMocks(); });

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>(accept => { resolve = accept; });
  return { promise, resolve };
}
async function fixture() {
  const storage = new WritingDiffDemoStorage(manuscriptId, initialChanges);
  const agent = new WritingDiffDemoAgentStorage(storage, undefined);
  const runtime = createWritingDiffDemoRuntime(storage, agent, manuscriptId, conversationId);
  cleanups.push(() => runtime.dispose());
  const file = (await storage.files.listFiles(manuscriptId)).find(value => value.relativePath === "main.tex")!;
  const inputId = crypto.randomUUID();
  const queue = await runtime.queue.gateway.edit({ manuscriptId, conversationId, requestId: crypto.randomUUID(), expectedRevision: 0,
    edit: { type: "add", inputId, text: "/compact" } });
  const request: AgentWritingQueuedCompactionPrepareRequest = {
    inputId, previousRequestId: null, expectedQueueRevision: queue.revision, expectedRevision: 0,
    submission: { conversationId, requestId: crypto.randomUUID(), text: "/compact", context: { manuscriptId,
      files: [{ relativePath: file.relativePath, expectedDiskSha256: file.sha256 }] } },
  };
  const receipt = { conversationId, manuscriptId, requestId: request.submission.requestId };
  const holdSourceRead = () => {
    const entered = deferred(), release = deferred(), read = storage.files.readFile.bind(storage.files);
    vi.spyOn(storage.files, "readFile").mockImplementationOnce(async (...args) => {
      entered.resolve(); await release.promise; return read(...args);
    });
    return { entered: entered.promise, release: release.resolve };
  };
  return { runtime, agent, request, receipt, queue, holdSourceRead };
}

it.each(["queue", "history"] as const)("does not publish a frozen submission when %s changes during source materialization", async change => {
  const f = await fixture(), held = f.holdSourceRead();
  const preparing = f.runtime.compaction.gateway.prepareWritingQueued(f.request);
  const rejected = change === "queue" ? expect(preparing).rejects.toMatchObject({ code: "conflict" })
    : expect(preparing).rejects.toBe("conflict");
  await held.entered;
  if (change === "queue") {
    await f.runtime.queue.gateway.edit({ manuscriptId, conversationId, requestId: crypto.randomUUID(), expectedRevision: f.queue.revision,
      edit: { type: "add", inputId: crypto.randomUUID(), text: "concurrent tail" } });
  } else {
    const admission = { scope: { manuscriptId, conversationId, requestId: crypto.randomUUID(), contextSnapshotId: crypto.randomUUID() }, turnId: crypto.randomUUID() };
    f.agent.history.start(admission, "concurrent turn");
    f.agent.history.finish(admission, "canceled");
  }
  const current = await f.runtime.queue.gateway.read(f.receipt);
  held.release(); await rejected;
  expect(await f.runtime.queue.gateway.read(f.receipt)).toEqual(current);
  expect(current.preparingInputId).toBeNull();
  expect(f.runtime.compaction.hasPrepared(f.receipt)).toBe(false);
  await expect(f.runtime.gateways.submission.restoreWriting(f.receipt)).rejects.toThrow("模拟冻结提交不存在");
  await expect(f.runtime.compaction.gateway.restoreWriting(f.receipt)).rejects.toMatchObject({ code: "notFound" });
  expect(await f.runtime.compaction.gateway.readWritingQueued({ ...f.receipt, inputId: f.request.inputId })).toBeNull();
  expect(f.runtime.execution.snapshot().some(([, value]) => value.request.requestId === f.receipt.requestId)).toBe(false);
});

it.each(["before", "during"] as const)("does not adopt or delete an ordinary R prepared %s compaction materialization", async timing => {
  const f = await fixture();
  let ordinary;
  if (timing === "before") ordinary = await f.runtime.gateways.submission.prepareWriting(f.request.submission);
  const held = timing === "during" ? f.holdSourceRead() : null;
  const preparing = f.runtime.compaction.gateway.prepareWritingQueued(f.request);
  const rejected = expect(preparing).rejects.toMatchObject({ code: "confirmationMismatch" });
  if (held) {
    await held.entered;
    ordinary = await f.runtime.gateways.submission.prepareWriting(f.request.submission);
    held.release();
  }
  await rejected;
  expect(await f.runtime.gateways.submission.restoreWriting(f.receipt)).toEqual(ordinary);
  expect(await f.runtime.queue.gateway.read(f.receipt)).toEqual(f.queue);
  expect(f.runtime.compaction.hasPrepared(f.receipt)).toBe(false);
  await expect(f.runtime.compaction.gateway.restoreWriting(f.receipt)).rejects.toMatchObject({ code: "notFound" });
});

it("publishes the frozen input and compact purpose in the same synchronous acceptance", async () => {
  const f = await fixture(), prepared = await f.runtime.compaction.gateway.prepareWritingQueued(f.request);
  expect(await f.runtime.gateways.submission.restoreWriting(f.receipt)).toEqual(prepared.preview);
  expect((await f.runtime.queue.gateway.read(f.receipt)).preparingInputId).toBe(f.request.inputId);
  await expect(f.runtime.gateways.submission.startWriting({ ...f.receipt, contextSnapshotId: prepared.preview.contextSnapshotId,
    model: prepared.preview.input.model, mode: "deepSeekNonThinking" })).rejects.toMatchObject({ code: "confirmationMismatch" });
  expect(await f.runtime.compaction.gateway.prepareWritingQueued(f.request)).toEqual(prepared);
});
