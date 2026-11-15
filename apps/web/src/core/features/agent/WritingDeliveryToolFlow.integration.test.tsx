/** @vitest-environment happy-dom */
import { act } from "react";
import { afterEach, expect, it, vi } from "vitest";
import * as f from "../../../app/previews/agent/WritingDiffDemoAgentStorage.integrationHarness";
import { scope, settled, notificationSpies } from "../../../app/previews/agent/WritingDiffDemoAgentStorage.integrationHarness";
import { WritingDiffDemoSubmission } from "../../../app/previews/agent/WritingDiffDemoSubmission";
import type { WritingDiffDemoFault } from "../../../app/previews/agent/WritingDiffDemoStorage";
import { projectConversationMessages } from "./projectConversationMessages";
import { applyAgentEvents, createAgentConversationProjection } from "@research-assistant/core/agent";
import { readWritingSubmissionBuffers } from "../../../../../../plugins/writing_projects/src/features/writing/agent/writingSubmissionSources";
afterEach(() => vi.useRealTimers());

it("reconciles the original lost save receipt on a subsequent delivery tool call without applying text again", async () => {
  await f.mount(); await settled(() => expect(f.documents.agentDelivery.pending).toBe(false));
  const buffers = f.documents.readFileBuffers();
  const admission = { scope: { ...scope, requestId: crypto.randomUUID(), contextSnapshotId: crypto.randomUUID() }, turnId: crypto.randomUUID() };
  const changes = buffers.map(buffer => ({ path: buffer.path, kind: "modified" as const,
    before: buffer.text, after: buffer.text + "\n% next", additions: 1, deletions: 0 }));
  f.agent.history.start(admission, "修改并核对保存结果");
  await f.agent.complete(admission, changes, "completed", buffers, true, true);
  const effects = f.documents.files.all().map(file => vi.spyOn(file.applicationPort, "apply"));
  f.storage.fault = "fileReceipt";
  await act(async () => expect((await f.agent.tools.request(admission)).complete).toBe(false));
  await act(async () => expect((await f.agent.tools.request(admission)).complete).toBe(true));
  f.agent.history.complete(admission, changes, true);
  await settled(() => expect(f.documents.agentDelivery.pending).toBe(false));
  expect(effects.every(effect => effect.mock.calls.length === 1)).toBe(true);
  for (const change of changes) expect((await f.storage.files.readFile(scope.manuscriptId, change.path)).content).toBe(change.after);
});

it.each(["file", "checkpoint", "both", "fileReceipt", "checkpointReceipt", "dispatch", "editorConfirm", "diskConfirm", "delayed"] as const)(
  "returns actual %s delivery errors to the model fixture before ending, without replaying or notifying", async fault => {
    await f.mount();
    await settled(() => expect(f.documents.agentDelivery.pending).toBe(false));
    const execution = new WritingDiffDemoSubmission(f.storage, f.agent, scope.manuscriptId);
    execution.delayMs = 1; execution.setTextPacing({ charactersPerChunk: 1000, intervalMs: 10 });
    const effects = f.documents.files.all().map(file => vi.spyOn(file.applicationPort, "apply"));
    const start = async (text: string) => {
      const buffers = await readWritingSubmissionBuffers(scope.manuscriptId, f.documents.files, f.storage.files);
      const preview = await execution.submission.prepareWriting({ conversationId: scope.conversationId, requestId: crypto.randomUUID(), text,
        context: { manuscriptId: scope.manuscriptId, files: buffers.map(buffer => ({ relativePath: buffer.path, expectedDiskSha256: buffer.expectedDiskSha256,
          buffer: { text: buffer.text, revision: buffer.revision } })) } });
      const current = { ...scope, requestId: preview.requestId, contextSnapshotId: preview.contextSnapshotId };
      await execution.workspace.prepare({ scope: current, buffers });
      await execution.submission.startWriting({ ...current, model: preview.input.model, mode: "deepSeekNonThinking" });
      return { scope: current, turnId: preview.turnId };
    };
    try {
      f.storage.fault = fault as WritingDiffDemoFault;
      f.storage.ioDelayMs = 10;
      f.agent.failBeforeDispatch = fault === "dispatch";
      f.agent.failEditorConfirmation = fault === "editorConfirm";
      let turn!: Awaited<ReturnType<typeof start>>;
      vi.useFakeTimers();
      await act(async () => { turn = await start("修改稿件"); });
      await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
      expect(f.agent.history.activeTurn(scope.conversationId)).toBeNull();
      const events = (await f.agent.history.gateway.readHistory({ conversationId: scope.conversationId, afterSeq: 0, limit: 500 })).events;
      const messages = projectConversationMessages(applyAgentEvents(createAgentConversationProjection(scope.conversationId), events)).filter(message => message.turnId === turn.turnId);
      const result = messages.find(message => message.toolName === "research_deliver_files" && message.kind === "tool-result");
      expect(result).toBeDefined();
      // A lost checkpoint reply may already be reconciled by the persistence
      // owner. The model receives that actual outcome, never a scripted error.
      if (fault !== "checkpointReceipt" && fault !== "delayed") {
        expect(result?.toolError).toBe(true);
        expect(messages.find(message => message.phase === "finalAnswer")?.content).toContain("未完整完成");
      }
      expect(effects.every(effect => effect.mock.calls.length <= 1)).toBe(true);
      expect(notificationSpies().error).not.toHaveBeenCalled(); expect(notificationSpies().warning).not.toHaveBeenCalled();
      // A subsequent text-only turn must run despite the retained file journal.
      execution.singleTextResponse = "已收到，继续讨论文件错误。";
      await act(async () => { await start("解释错误"); await vi.advanceTimersByTimeAsync(2000); });
      expect(f.agent.history.activeTurn(scope.conversationId)).toBeNull();
    } finally { execution.dispose(); }
  }, 10000,
);
