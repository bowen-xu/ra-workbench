/** @vitest-environment happy-dom */
import { act } from "react";
import { createHash } from "node:crypto";
import { undo, redo } from "@codemirror/commands";
import { expect, it, vi } from "vitest";
import { initialChanges, mixedFileChanges, binaryFileChanges, type DemoChange } from "../../../core/features/agent/WritingDiffFixtures";
import { WritingDiffDemoAgentStorage } from "./WritingDiffDemoAgentStorage";
import { writingDiffDemoBytes } from "./WritingDiffDemoBytes";
import * as f from "./WritingDiffDemoAgentStorage.integrationHarness";
import { scope, admission, Harness, mount, view, journal, settled } from "./WritingDiffDemoAgentStorage.integrationHarness";
it("rediscovers a prepared Undo after rebuilding the production workspace with chat closed", async () => {
  await mount();
  await settled(() => expect(initialChanges.every(change => f.documents.saveState(change.path) === "saved")).toBe(true));
  const original = (await journal()).state!, undoId = crypto.randomUUID();
  await f.agent.gateways.application.undos.execute({ scope: original.scope, undoId,
    command: { action: "prepare", buffers: f.documents.readFileBuffers() } });
  // The old React workspace never observed or recorded this Undo identity.
  const discovery = vi.spyOn(f.agent.gateways.pendingUndos, "list"), write = vi.spyOn(f.storage.files, "saveFile");
  const command = vi.spyOn(f.agent.gateways.application.undos, "execute");
  await act(async () => f.root.unmount()); f.resetRoot();
  await act(async () => f.root.render(<Harness historyOpen={false} />));
  await settled(() => expect(f.host.querySelector('[aria-label="文件撤销恢复"]')).not.toBeNull());
  await settled(() => expect(f.documents.ready).toBe(true));
  expect(discovery).toHaveBeenCalledWith({ manuscriptId: scope.manuscriptId, after: null, limit: 64 });
  expect(view().state.doc.toString()).toBe(initialChanges[0]!.after);
  expect(command).not.toHaveBeenCalled(); expect(write).not.toHaveBeenCalled();
  const cancel = [...f.host.querySelectorAll("button")].find(button => button.textContent === "取消未执行撤销")!;
  await settled(() => expect(cancel.disabled).toBe(false));
  await act(async () => cancel.click());
  await settled(() => expect(f.host.querySelector('[aria-label="文件撤销恢复"]')).toBeNull());
  expect(command.mock.calls.map(([request]) => request.command.action)).toEqual(["read", "cancelPending"]);
  expect(command.mock.calls.every(([request]) => request.undoId === undoId)).toBe(true);
  expect(view().state.doc.toString()).toBe(initialChanges[0]!.after);
  expect(write).not.toHaveBeenCalled();
});

async function interruptedMixed(changes: readonly DemoChange[] = mixedFileChanges) {
  await f.agent.initialized; f.agent.history.dispose();
  f.configure(changes);
  f.agent.failDiskConfirmation = true;
  await act(async () => f.root.render(<Harness />));
  await settled(() => expect(f.host.querySelector('[aria-label="处理文件操作"], [aria-label="处理 Agent 文件修改"]')).not.toBeNull());
  return (await journal()).state!;
}
it.each([{ changes: mixedFileChanges }, { changes: binaryFileChanges }])("rolls back interrupted text and disk plans through production compensation (%#)", async ({ changes }) => {
  const state = await interruptedMixed(changes), editor = view();
  let result;
  await act(async () => { result = await f.documents.applicationActions.rollback(state.scope); });
  expect(result).toMatchObject({ kind: "state", state: { status: "rolledBack" } });
  expect(view()).toBe(editor);
  expect(editor.state.doc.toString()).toBe(mixedFileChanges[0]!.before);
  expect((await f.storage.files.readFile(scope.manuscriptId, "main.tex")).content).toBe(mixedFileChanges[0]!.before);
  for (const change of changes.slice(1)) {
    if (change.kind === "added") expect((await f.storage.files.listFiles(scope.manuscriptId)).some(file => file.relativePath === change.path)).toBe(false);
    else expect(await f.storage.files.readBytes(scope.manuscriptId, change.path)).toEqual(writingDiffDemoBytes(change, "before"));
  }
  const apply = vi.spyOn(f.documents.files.all()[0]!.applicationPort, "apply"), disk = vi.spyOn(f.storage.files, "restoreChange");
  await act(async () => { result = await f.documents.applicationActions.rollback(state.scope); });
  expect(result).toMatchObject({ kind: "state", state: { status: "rolledBack" } });
  expect(apply).not.toHaveBeenCalled(); expect(disk).not.toHaveBeenCalled();
});
it("preserves later editor input when an interrupted plan is explicitly rolled back", async () => {
  const state = await interruptedMixed(), editor = view();
  await act(async () => editor.dispatch({ changes: { from: editor.state.doc.length, insert: "later input" } }));
  let result;
  await act(async () => { result = await f.documents.applicationActions.rollback(state.scope); });
  expect(result).toMatchObject({ kind: "unconfirmed" });
  expect(editor.state.doc.toString()).toBe(mixedFileChanges[0]!.after + "later input");
  expect((await f.storage.files.listFiles(scope.manuscriptId)).some(file => file.relativePath === "sections/new.tex")).toBe(true);
});
it("reconciles a lost disk rollback receipt without a second editor restoration", async () => {
  const state = await interruptedMixed(), editor = view();
  f.storage.fault = "fileReceipt";
  let result;
  await act(async () => { result = await f.documents.applicationActions.rollback(state.scope); });
  expect(result).toMatchObject({ kind: "unconfirmed" });
  expect(editor.state.doc.toString()).toBe(mixedFileChanges[0]!.before);
  const apply = vi.spyOn(f.documents.files.all()[0]!.applicationPort, "apply");
  f.storage.fault = "none";
  await act(async () => { result = await f.documents.applicationActions.rollback(state.scope); });
  expect(result).toMatchObject({ kind: "state", state: { status: "rolledBack" } });
  expect(apply).not.toHaveBeenCalled();
  expect(await f.storage.files.readBytes(scope.manuscriptId, "assets/old.json")).toEqual(writingDiffDemoBytes(mixedFileChanges[2]!, "before"));
});
it("confirms the original restoration receipt after later input but stops remaining disk compensation", async () => {
  const state = await interruptedMixed(), editor = view();
  f.agent.restorationStorage.failConfirmation = true;
  let result;
  await act(async () => { result = await f.documents.applicationActions.rollback(state.scope); });
  expect(result).toMatchObject({ kind: "unconfirmed" });
  expect(editor.state.doc.toString()).toBe(mixedFileChanges[0]!.before);
  await act(async () => editor.dispatch({ changes: { from: editor.state.doc.length, insert: "retained after restoration" } }));
  const apply = vi.spyOn(f.documents.files.all()[0]!.applicationPort, "apply");
  f.agent.restorationStorage.failConfirmation = false;
  await act(async () => { result = await f.documents.applicationActions.rollback(state.scope); });
  expect(result).toMatchObject({ kind: "unconfirmed", state: { status: "recoveryRequired", files: expect.arrayContaining([
    expect.objectContaining({path: "main.tex", phase: "restored"}),
  ]) } });
  expect(apply).not.toHaveBeenCalled();
  expect(editor.state.doc.toString()).toBe(mixedFileChanges[0]!.before + "retained after restoration");
  expect((await f.storage.files.listFiles(scope.manuscriptId)).some(file => file.relativePath === "sections/new.tex")).toBe(true);
  expect((await f.storage.files.listFiles(scope.manuscriptId)).some(file => file.relativePath === "assets/old.json")).toBe(false);
});
it("stops disk compensation at an external version without overwriting it", async () => {
  const state = await interruptedMixed();
  await f.storage.externalEdit("sections/new.tex", "external actor");
  let result;
  await act(async () => { result = await f.documents.applicationActions.rollback(state.scope); });
  expect(result).toMatchObject({ kind: "unconfirmed", state: { status: "recoveryRequired" } });
  expect((await f.storage.files.readFile(scope.manuscriptId, "sections/new.tex")).content).toBe("external actor");
});
it("restores the card Undo through the shared multi-file controller and persists background files", async () => {
  const current = await mount();
  await settled(() => expect(f.documents.agentDelivery.pending).toBe(false));
  const original = (await journal()).state!;
  await act(async () => f.root.render(<Harness historyOpen />));
  await settled(() => expect(f.host.querySelector(".writing-agent-inline-diff-undo")).not.toBeNull());
  const calls = vi.spyOn(f.agent.gateways.application.undos, "execute");
  await act(async () => f.host.querySelector<HTMLButtonElement>(".writing-agent-inline-diff-undo")!.click());
  await settled(() => expect(calls.mock.calls.length).toBeGreaterThan(1));
  await calls.mock.results[1]!.value;
  await settled(() => expect(calls.mock.calls.map(([request]) => request.command.action)).toContain("confirmEditor"));
  await settled(() => expect(current.state.doc.toString()).toBe(initialChanges[0]!.before));
  await settled(() => expect(f.documents.undoRecovery.pending).toBe(false));
  for (const change of initialChanges) {
    expect(f.documents.files.get({ manuscriptId: scope.manuscriptId, relativePath: change.path })!.read().content).toBe(change.before);
    expect((await f.storage.files.readFile(scope.manuscriptId, change.path)).content).toBe(change.before);
  }
  const undone = await f.agent.gateways.application.undos.find(original.scope);
  expect(undone?.result).toMatchObject({ status: "applied" });
  expect(f.host.textContent).not.toContain("应用与恢复记录");
  await act(async () => f.host.querySelector<HTMLButtonElement>(".writing-agent-inline-diff-undo")!.click());
  expect((await f.agent.gateways.application.undos.find(original.scope))?.undoId).toBe(undone?.undoId);
  await act(async () => f.root.render(<Harness />));
  expect(view()).toBe(current);
  await act(async () => { expect(undo(current)).toBe(true); });
  expect(current.state.doc.toString()).toBe(initialChanges[0]!.after);
  await act(async () => { expect(redo(current)).toBe(true); });
  expect(current.state.doc.toString()).toBe(initialChanges[0]!.before);
});

it("rejects card Undo before any file changes when a background file has later input", async () => {
  const current = await mount();
  await settled(() => expect(f.host.textContent).not.toContain("等待稿件可用"));
  const changed = f.documents.files.get({ manuscriptId: scope.manuscriptId, relativePath: initialChanges[1]!.path })!;
  await act(async () => changed.dispatch({ changes: { from: changed.state.doc.length, insert: "later input" } }));
  const { request, state } = await journal();
  let result;
  await act(async () => { result = await f.documents.undoProposal(request); });
  expect(result).toMatchObject({ kind: "unconfirmed" });
  expect(current.state.doc.toString()).toBe(initialChanges[0]!.after);
  expect(changed.read().content).toBe(initialChanges[1]!.after + "later input");
  expect(await f.agent.gateways.application.undos.find(state!.scope)).toBeNull();
});
it.each([{ changes: mixedFileChanges }, { changes: binaryFileChanges }])("undoes mixed disk and editor changes through native protocol mocks", async ({ changes }) => {
  await f.agent.initialized; f.agent.history.dispose();
  f.configure(changes);
  await act(async () => f.root.render(<Harness historyOpen />));
  await settled(() => expect(f.documents.agentDelivery.pending).toBe(false));
  await settled(() => expect(f.host.querySelector(".writing-agent-inline-diff-undo")).not.toBeNull());
  const { request, state } = await journal();
  let result;
  await act(async () => { result = await f.documents.undoProposal(request); });
  expect(result).toMatchObject({ kind: "state", state: { status: "applied" } });
  const listing = await f.storage.files.listFiles(scope.manuscriptId);
  for (const change of changes) {
    if (change.kind === "added") expect(listing.some(file => file.relativePath === change.path)).toBe(false);
    else expect(await f.storage.files.readBytes(scope.manuscriptId, change.path)).toEqual(writingDiffDemoBytes(change, "before"));
  }
  expect((await f.agent.gateways.application.applications.find(request))?.scope.operationId).toBe(state!.scope.operationId);
  expect((await f.agent.gateways.application.undos.find(state!.scope))?.undoId).not.toBe(state!.scope.operationId);
});
it("recovers a lost mixed Undo write receipt and resumes only the original pending targets", async () => {
  await f.agent.initialized; f.agent.history.dispose();
  f.configure(mixedFileChanges);
  await act(async () => f.root.render(<Harness />));
  await settled(() => expect(f.documents.agentDelivery.pending).toBe(false));
  const { request, state } = await journal();
  f.storage.fault = "fileReceipt";
  let result;
  await act(async () => { result = await f.documents.undoProposal(request); });
  expect(result).toMatchObject({ kind: "unconfirmed", state: { status: "recoveryRequired" } });
  const failed = await f.agent.gateways.application.undos.find(state!.scope);
  expect(failed?.result).toMatchObject({ files: [{ phase: "acknowledged" }, { phase: "dispatched" }, { phase: "pending" }] });
  f.storage.fault = "none";
  await vi.waitFor(async () => expect((await f.storage.files.readFile(scope.manuscriptId, "main.tex")).content).toBe(mixedFileChanges[0]!.before));
  const writes = vi.spyOn(f.storage.files, "applyChange");
  await act(async () => { result = await f.documents.undoProposal(request); });
  expect(result).toMatchObject({ kind: "state", state: { status: "applied" } });
  expect(writes.mock.calls.map(call => call[1])).toEqual([mixedFileChanges[2]!.path]);
  expect((await f.agent.gateways.application.undos.find(state!.scope))?.undoId).toBe(failed?.undoId);
});
it("keeps interrupted Undo recovery outside chat and compensates through the production file host", async () => {
  await f.agent.initialized; f.agent.history.dispose();
  f.configure(mixedFileChanges);
  await act(async () => f.root.render(<Harness historyOpen />));
  await settled(() => expect(f.documents.agentDelivery.pending).toBe(false));
  const current = view(), { request, state } = await journal();
  f.storage.fault = "fileReceipt";
  await act(async () => { await f.documents.undoProposal(request); });
  await act(async () => f.root.render(<Harness />));
  expect(view()).toBe(current);
  expect(f.host.querySelector(".writing-undo-recovery")).not.toBeNull();
  expect(f.documents.agentDelivery.pending).toBe(true);
  const interrupted = await f.agent.gateways.application.undos.find(state!.scope);
  f.storage.fault = "none";
  await vi.waitFor(async () => expect((await f.storage.files.readFile(scope.manuscriptId, "main.tex")).content).toBe(mixedFileChanges[0]!.before));
  await act(async () => [...f.host.querySelectorAll("button")].find(button => button.textContent === "恢复撤销前内容")!.click());
  await settled(() => expect(f.documents.undoRecovery.pending).toBe(false));
  expect(view()).toBe(current);
  expect(current.state.doc.toString()).toBe(mixedFileChanges[0]!.after);
  for (const change of mixedFileChanges) {
    if (change.kind === "deleted") expect((await f.storage.files.listFiles(scope.manuscriptId)).some(file => file.relativePath === change.path)).toBe(false);
    else expect(await f.storage.files.readBytes(scope.manuscriptId, change.path)).toEqual(writingDiffDemoBytes(change, "after"));
  }
  expect((await f.agent.gateways.application.undos.find(state!.scope))?.undoId).toBe(interrupted?.undoId);
  expect((await f.agent.gateways.application.applications.find(request))?.status).toBe("applied");
  const checkpoint = (await f.storage.checkpoints.list(scope.manuscriptId)).find(row => row.checkpoint.relativePath === "main.tex")!.checkpoint;
  expect(checkpoint.editorReceipt?.text).toBe(mixedFileChanges[0]!.after);
  expect(checkpoint.editorReceipt?.operationId).not.toBe(state!.scope.operationId);
  expect(checkpoint.editorReceipt?.operationId).not.toBe(interrupted!.undoId);
  expect(checkpoint.operations.at(-1)?.operationId).toBe(checkpoint.editorReceipt?.operationId);
  expect(checkpoint.operations).toHaveLength(3);
  await act(async () => { expect(undo(current)).toBe(true); expect(redo(current)).toBe(true); });
  expect(current.state.doc.toString()).toBe(mixedFileChanges[0]!.after);
});
it("preserves later input when workspace Undo compensation is explicitly requested", async () => {
  await f.agent.initialized; f.agent.history.dispose();
  f.configure(mixedFileChanges);
  await act(async () => f.root.render(<Harness />));
  await settled(() => expect(f.documents.agentDelivery.pending).toBe(false));
  const { request } = await journal(); f.storage.fault = "fileReceipt";
  await act(async () => { await f.documents.undoProposal(request); });
  f.storage.fault = "none";
  const current = view();
  await act(async () => current.dispatch({ changes: { from: current.state.doc.length, insert: "later input" } }));
  const text = current.state.doc.toString();
  await act(async () => [...f.host.querySelectorAll("button")].find(button => button.textContent === "恢复撤销前内容")!.click());
  expect(current.state.doc.toString()).toBe(text);
  expect(f.documents.undoRecovery.pending).toBe(true);
  expect(f.notificationSpies().error).toHaveBeenCalledWith(expect.stringContaining("仍未确认"), expect.anything());
});
it("receives the frozen multi-file completion through production delivery, journals, checkpoints and formal saves", async () => {
  const current = await mount();
  await settled(() => expect(f.host.textContent).not.toContain("等待稿件可用"));
  const { state } = await journal();
  expect(state?.status).toBe("applied");
  expect(state?.files.map((file) => [file.path, file.phase, file.target.kind])).toEqual(initialChanges.map((change) => [change.path, "acknowledged", "editor"]));
  for (const change of initialChanges) {
    const file = f.documents.files.get({ manuscriptId: scope.manuscriptId, relativePath: change.path })!;
    expect(file.read()).toMatchObject({ content: change.after, reviewBaseline: change.before });
    expect(file.readOperations()).toEqual([{ operationId: state!.scope.operationId, conversationId: scope.conversationId, requestId: scope.requestId,
      resultSha256: createHash("sha256").update(change.after).digest("hex") }]);
    expect((await f.storage.files.readFile(scope.manuscriptId, change.path)).content).toBe(change.after);
  }
  const records = await f.storage.checkpoints.list(scope.manuscriptId);
  expect(records).toHaveLength(3);
  expect(current.state.doc.toString()).toBe(initialChanges[0]!.after);
  expect((await f.agent.gateways.delivery.deliveries.list({ manuscriptId: scope.manuscriptId, after: null, limit: 64 })).items).toHaveLength(0);
});
it("delivers mixed editor, added and deleted targets while preserving closed disk targets", async () => {
  await f.agent.initialized;
  f.agent.history.dispose();
  const changes: readonly DemoChange[] = [initialChanges[0]!,
    { path: "sections/new.tex", kind: "added", before: "", after: "\\section{New}\r\n😀", additions: 2, deletions: 0 },
    { path: "assets/old.json", kind: "deleted", before: "old asset", after: "", additions: 0, deletions: 1 }];
  f.configure(changes);
  await act(async () => f.root.render(<Harness />));
  await settled(() => expect(f.documents.files.all().map(file => file.readOperations().length)).toEqual([1]));
  await settled(() => expect(f.host.querySelector(".writing-file-recovery-actions, .writing-undo-recovery")).toBeNull());
  const original = (await journal()).state!;
  expect(original.status).toBe("applied");
  expect(original.files.map(file => file.target.kind)).toEqual(["editor", "disk", "disk"]);
  expect(f.documents.fileList.map(file => file.relativePath)).toEqual(["main.tex", "sections/new.tex"]);
  expect((await f.storage.files.readFile(scope.manuscriptId, "sections/new.tex")).content).toBe(changes[1]!.after);
  await act(async () => f.documents.select("sections/new.tex"));
  await settled(() => expect(view().state.doc.toString()).toBe(changes[1]!.after));
  const current = view();
  await act(async () => current.dispatch({ changes: { from: current.state.doc.length, insert: "\n% user edit" } }));
  await act(async () => { expect(await f.documents.restart()).toBe(true); });
  await settled(() => expect(f.documents.ready).toBe(true));
  expect(view().state.doc.toString()).toBe(`${changes[1]!.after}\n% user edit`);
  expect(f.documents.fileList.some(file => file.relativePath === "assets/old.json")).toBe(false);
  expect((await journal()).state).toEqual(original);
});
it("rejects a colliding disk target before modifying another file in the same batch", async () => {
  await f.agent.initialized; f.agent.history.dispose();
  const changes: readonly DemoChange[] = [initialChanges[0]!,
    { path: "sections/collision.tex", kind: "added", before: "", after: "agent addition", additions: 1, deletions: 0 }];
  f.configure(changes);
  await f.agent.initialized;
  await f.storage.files.createFile(scope.manuscriptId, changes[1]!.path, "external replacement");
  await act(async () => f.root.render(<Harness />));
  await settled(() => expect(f.host.querySelector('[aria-label="处理文件操作"], [aria-label="处理 Agent 文件修改"]')).not.toBeNull());
  expect(view().state.doc.toString()).toBe(changes[0]!.before);
  expect(f.documents.files.all().map(file => file.readOperations().length)).toEqual([0]);
  expect((await journal()).state).toBeNull();
  expect((await f.storage.files.readFile(scope.manuscriptId, changes[1]!.path)).content).toBe("external replacement");
});
it("reconstructs file sessions and explicitly resumes the original wholly pending application through production controllers", async () => {
  f.agent.failBeforeDispatch = true;
  await act(async () => f.root.render(<Harness />));
  await settled(() => expect(f.host.querySelector('[aria-label="处理文件操作"], [aria-label="处理 Agent 文件修改"]')).not.toBeNull());
  const current = view(), original = (await journal()).state;
  expect(original?.status).toBe("applying");
  expect(original?.files.every(file => file.phase === "pending")).toBe(true);
  for (const change of initialChanges) expect((await f.storage.files.readFile(scope.manuscriptId, change.path)).content).toBe(change.before);
  f.agent.failBeforeDispatch = false;
  await act(async () => { expect(await f.documents.restart()).toBe(true); });
  await settled(() => expect(f.host.textContent).toContain("继续原文件修改"));
  expect(f.documents.files.all().map(file => file.readOperations().length)).toEqual([0, 0, 0]);
  await act(async () => [...f.host.querySelectorAll<HTMLButtonElement>("button")]
    .find(button => button.textContent === "继续原文件修改")!.click());
  await settled(() => expect(f.documents.files.all().map(file => file.readOperations().length)).toEqual([1, 1, 1]));
  await settled(() => expect(f.host.querySelector(".writing-file-recovery-actions, .writing-undo-recovery")).toBeNull());
  expect(view()).not.toBe(current);
  const restored = (await journal()).state;
  expect(restored?.scope.operationId).toBe(original?.scope.operationId);
  expect(restored?.status).toBe("applied");
  for (const file of restored!.files) {
    expect(file.phase).toBe("acknowledged");
    const previous = original!.files.find(item => item.path === file.path)!;
    if (file.target.kind !== "editor" || previous.target.kind !== "editor") throw new Error("Expected editor targets");
    expect(file.target.sessionId).not.toBe(previous.target.sessionId);
  }
  for (const change of initialChanges) {
    expect((await f.storage.files.readFile(scope.manuscriptId, change.path)).content).toBe(change.after);
    expect(f.documents.files.get({ manuscriptId: scope.manuscriptId, relativePath: change.path })!.read()).toMatchObject({ content: change.after, reviewBaseline: change.before });
  }
});
it("retries a lost acknowledgment by reading the original journal without another editor transaction", async () => {
  f.agent.loseAcknowledgmentOnce = true;
  const current = await mount();
  await settled(() => expect(f.host.querySelector('[aria-label="处理文件操作"], [aria-label="处理 Agent 文件修改"]')).not.toBeNull());
  const before = f.documents.files.all().map((file) => file.read()), original = (await journal()).state;
  await act(async () => [...f.host.querySelectorAll("button")].find((button) => button.textContent === "重试确认记录")!.click());
  await settled(() => expect(f.host.textContent).not.toContain("重试确认记录"));
  expect(f.documents.files.all().map((file) => file.read())).toEqual(before);
  expect((await journal()).state).toEqual(original);
  expect(view()).toBe(current);
});
it("reconciles a dispatched editor receipt after later input and Keep without replaying any file", async () => {
  f.agent.failEditorConfirmation = true;
  const current = await mount();
  await settled(() => expect(f.host.querySelector('[aria-label="处理文件操作"], [aria-label="处理 Agent 文件修改"]')).not.toBeNull());
  const original = (await journal()).state!;
  expect(original.status).toBe("recoveryRequired");
  expect(original.files.map(file => file.phase)).toEqual(["acknowledged", "acknowledged", "dispatched"]);
  const last = original.files.at(-1)!;
  await act(async () => f.documents.select(last.path));
  const selected = view(), file = f.documents.files.get({ manuscriptId: scope.manuscriptId, relativePath: last.path })!;
  const receipt = file.applicationPort.readReceipt(original.scope.operationId)!;
  expect(receipt).not.toBeNull();
  await act(async () => {
    selected.dispatch({ changes: { from: selected.state.doc.length, insert: "\n% later user input" } });
    f.host.querySelector<HTMLButtonElement>('[aria-label="Keep all"]')!.click();
  });
  const latest = file.read(), dispatch = vi.spyOn(f.agent.gateways.application.applications, "execute");
  f.agent.failEditorConfirmation = false;
  await act(async () => [...f.host.querySelectorAll("button")].find(button => button.textContent === "核对文件结果")!.click());
  await settled(() => expect(f.host.textContent).toContain("重试保存文件结果"));
  expect(file.read()).toEqual(latest);
  await act(async () => [...f.host.querySelectorAll("button")].find(button => button.textContent === "重试保存文件结果")!.click());
  await settled(() => expect(f.host.querySelector(".writing-file-recovery-actions, .writing-undo-recovery")).toBeNull());
  expect(dispatch.mock.calls.some(([request]) => request.action === "prepareEditor")).toBe(false);
  expect(file.read()).toEqual(latest);
  expect(file.applicationPort.readReceipt(original.scope.operationId)).toEqual(receipt);
  expect(f.documents.files.all().map(entry => entry.readOperations().length)).toEqual([1, 1, 1]);
  expect(view()).toBe(current);
  const confirmed = (await journal()).state!;
  await expect(f.agent.gateways.application.applications.execute({ action: "confirmEditor", scope: original.scope,
    actual: { path: last.path, sessionId: receipt.snapshot.sessionId, revision: crypto.randomUUID(), text: receipt.snapshot.text } })).rejects.toThrow();
  expect((await journal()).state).toEqual(confirmed);
  expect(file.read()).toEqual(latest);
  expect((await f.storage.files.readFile(scope.manuscriptId, last.path)).content).toBe(latest.content);
  await act(async () => { expect(await f.documents.restart()).toBe(true); });
  await settled(() => expect(f.documents.ready).toBe(true));
  expect(f.documents.files.get({ manuscriptId: scope.manuscriptId, relativePath: last.path })!.read()).toMatchObject(latest);
  expect(f.host.querySelector('[aria-label="Keep all"]')).toBeNull();
});
it("reconciles the original dispatched receipt after reconstructing file sessions without replay or losing later Keep", async () => {
  f.agent.failEditorConfirmation = true;
  await mount();
  await settled(() => expect(f.host.querySelector('[aria-label="处理文件操作"], [aria-label="处理 Agent 文件修改"]')).not.toBeNull());
  const original = (await journal()).state!, last = original.files.at(-1)!;
  const file = f.documents.files.get({ manuscriptId: scope.manuscriptId, relativePath: last.path })!;
  const receipt = file.applicationPort.readReceipt(original.scope.operationId);
  await act(async () => {
    file.dispatch({ changes: { from: file.state.doc.length, insert: "\n% later before restart" }, userEvent: "input.type" });
    file.resolveAll("keep");
  });
  const before = f.documents.files.all().map(file => file.read());
  f.agent.failEditorConfirmation = false;
  await act(async () => { expect(await f.documents.restart()).toBe(true); });
  await settled(() => expect(f.documents.ready).toBe(true));
  const restored = f.documents.files.get({ manuscriptId: scope.manuscriptId, relativePath: last.path })!;
  expect(restored.applicationPort.read()?.sessionId).not.toBe(receipt?.snapshot.sessionId);
  expect(restored.applicationPort.readReceipt(original.scope.operationId)).toEqual(receipt);
  const calls = vi.spyOn(f.agent.gateways.application.applications, "execute");
  await act(async () => {
    expect(await f.documents.applicationActions.recover(original.scope, last.path)).toMatchObject({ kind: "state", state: { status: "applied" } });
  });
  expect(calls.mock.calls.some(([request]) => request.action === "prepareEditor")).toBe(false);
  expect(calls.mock.calls.find(([request]) => request.action === "confirmEditor")?.[0]).toMatchObject({ actual: {
    sessionId: receipt?.snapshot.sessionId, revision: receipt?.snapshot.revision, text: receipt?.snapshot.text,
  } });
  expect((await journal()).state).toMatchObject({ status: "applied", scope: original.scope });
  expect(f.documents.files.all().map(file => file.read())).toEqual(before);
  expect(f.documents.files.all().map(file => file.readOperations().length)).toEqual([1, 1, 1]);
});
it("keeps confirmation after reconstructing the host even when the final acknowledgment reply was lost", async () => {
  f.agent.loseAcknowledgmentOnce = true;
  const current = await mount();
  await settled(() => expect(f.host.querySelector('[aria-label="处理文件操作"], [aria-label="处理 Agent 文件修改"]')).not.toBeNull());
  await act(async () => f.host.querySelector<HTMLButtonElement>('[aria-label="Keep all"]')!.click());
  const kept = f.documents.files.all()[0]!.read(), original = (await journal()).state;
  let restarted = false;
  await act(async () => { restarted = await f.documents.restart(); });
  expect(restarted).toBe(true);
  await settled(() => { expect(f.documents.ready).toBe(true); expect(view()).not.toBe(current); });
  expect(f.documents.files.all()[0]!.read()).toMatchObject({ content: kept.content, reviewBaseline: kept.content });
  expect((await journal()).state).toEqual(original);
  expect(f.host.querySelector('[aria-label="Keep all"]')).toBeNull();
  expect(f.host.querySelector(".writing-file-recovery-actions, .writing-undo-recovery")).toBeNull();
});
it("allows another chat to modify latest unconfirmed text and preserves a single file history", async () => {
  const current = await mount(), file = f.documents.files.all()[0]!;
  // Edit the pending new text; this must not rewrite the original review bytes.
  await act(async () => current.dispatch({ changes: { from: current.state.doc.toString().indexOf("This paper"), insert: "Updated: " }, userEvent: "input.type" }));
  const latest = file.read(), second = { scope: { ...scope, conversationId: crypto.randomUUID(), requestId: crypto.randomUUID(), contextSnapshotId: crypto.randomUUID() }, turnId: crypto.randomUUID() };
  const next = latest.content.replace("Updated: This paper", "Updated again: This paper");
  await f.agent.complete(second, [{ ...initialChanges[0]!, before: latest.content, after: next }]);
  await act(async () => f.documents.agentDelivery.register(second));
  await settled(() => expect(file.readOperations()).toHaveLength(2));
  await settled(() => expect(f.host.textContent).not.toContain("等待稿件可用"));
  expect(file.read()).toMatchObject({ content: next, reviewBaseline: initialChanges[0]!.before });
  expect(file.readOperations()[1]?.conversationId).toBe(second.scope.conversationId);
  expect(view()).toBe(current);
  await act(async () => { expect(undo(current)).toBe(true); });
  expect(file.read()).toMatchObject({ content: latest.content, reviewBaseline: latest.reviewBaseline });
  await act(async () => { expect(redo(current)).toBe(true); });
  expect(file.read().content).toBe(next);
});
it("reads historical texts after Keep without reviving the diff, old status panel or replacing the editor", async () => {
  const current = await mount();
  await act(async () => f.host.querySelector<HTMLButtonElement>('[aria-label="Keep all"]')!.click());
  const file = f.documents.files.all()[0]!, kept = file.read();
  await act(async () => f.root.render(<Harness historyOpen />));
  await settled(() => expect(f.host.textContent).toContain("Edited 3 files"));
  expect(f.host.querySelector("details:not(.writing-file-recovery-stack)")).toBeNull();
  expect(f.host.textContent).not.toContain("应用与恢复记录");
  expect(f.host.querySelector(".writing-agent-inline-diff-undo")).not.toBeNull();
  const { request } = await journal();
  const frozen = await f.agent.gateways.delivery.proposals.readFile({ proposal: request, path: "main.tex" });
  expect(frozen.file.before).toEqual({ kind: "text", text: initialChanges[0]!.before });
  expect(frozen.file.after).toEqual({ kind: "text", text: initialChanges[0]!.after });
  expect(file.read()).toEqual(kept); expect(view()).toBe(current);
  await act(async () => f.root.render(<Harness />));
  await act(async () => { expect(undo(current)).toBe(true); });
  expect(file.read().reviewBaseline).toBe(initialChanges[0]!.before);
  expect(file.pending).toBe(true);
});
it("rejects a stale completion without modifying any live file", async () => {
  await mount();
  const before = f.documents.files.all().map((file) => file.read());
  const stale = { scope: { ...scope, requestId: crypto.randomUUID(), contextSnapshotId: crypto.randomUUID() }, turnId: crypto.randomUUID() };
  await f.agent.complete(stale, initialChanges);
  await act(async () => f.documents.agentDelivery.register(stale));
  await settled(() => expect(f.host.querySelector('[aria-label="处理文件操作"], [aria-label="处理 Agent 文件修改"]')).not.toBeNull());
  expect(f.documents.files.all().map((file) => file.read())).toEqual(before);
  const proposal = await f.agent.gateways.delivery.proposals.latest(stale.scope);
  expect(await f.agent.gateways.application.applications.find({ manuscriptId: scope.manuscriptId, conversationId: scope.conversationId, proposalId: proposal!.proposalId })).toBeNull();
  const write = vi.spyOn(f.storage.files, "saveFile");
  const reject = [...f.host.querySelectorAll("button")].find(button => button.textContent === "保留当前稿件，放弃这次修改");
  expect(reject).toBeDefined();
  await act(async () => reject?.click());
  await settled(() => expect(f.documents.agentDelivery.pending).toBe(false));
  expect((await f.agent.gateways.delivery.proposals.latest(stale.scope))?.status).toBe("rejected");
  expect(f.documents.files.all().map(file => file.read())).toEqual(before);
  expect(write).not.toHaveBeenCalled();
  expect((await f.agent.gateways.delivery.deliveries.list({ manuscriptId: scope.manuscriptId, after: null, limit: 64 })).items).toEqual([]);
  const reopened = new WritingDiffDemoAgentStorage(f.storage, undefined);
  reopened.restore(f.agent.snapshot()); reopened.history.restore(f.agent.history.snapshot());
  expect((await reopened.gateways.delivery.proposals.latest(stale.scope))?.status).toBe("rejected");
  expect((await reopened.gateways.delivery.deliveries.list({ manuscriptId: scope.manuscriptId, after: null, limit: 64 })).items).toEqual([]);
  reopened.history.dispose();
});
it("reports a colliding completion but does not poison a later independent turn", async () => {
  await f.agent.initialized;
  await expect(f.agent.complete(admission, [{ ...initialChanges[0]!, after: "collision" }])).rejects.toThrow("身份冲突");
  const next = { scope: { ...scope, requestId: crypto.randomUUID(), contextSnapshotId: crypto.randomUUID() }, turnId: crypto.randomUUID() };
  await f.agent.complete(next, initialChanges);
  expect((await f.agent.gateways.delivery.deliveries.list({ manuscriptId: scope.manuscriptId, after: null, limit: 64 })).items).toHaveLength(2);
});
it("pages frozen review history without mixing another chat or changing earlier entries", async () => {
  await f.agent.initialized;
  const query = { manuscriptId: scope.manuscriptId, conversationId: scope.conversationId, cursor: null, limit: 1 };
  const original = await f.agent.reviews.list(query);
  const next = { scope: { ...scope, requestId: crypto.randomUUID(), contextSnapshotId: crypto.randomUUID() }, turnId: crypto.randomUUID() };
  await f.agent.complete(next, initialChanges);
  const other = { scope: { ...scope, conversationId: crypto.randomUUID(), requestId: crypto.randomUUID(), contextSnapshotId: crypto.randomUUID() }, turnId: crypto.randomUUID() };
  await f.agent.complete(other, initialChanges);
  const first = await f.agent.reviews.list(query);
  expect(first.entries).toHaveLength(1); expect(first.nextCursor).toBe(first.entries[0]!.scope.requestId);
  const second = await f.agent.reviews.list({ ...query, cursor: first.nextCursor });
  expect(second.entries).toHaveLength(1); expect(second.nextCursor).toBeNull();
  const all = [...first.entries, ...second.entries];
  expect(new Set(all.map(({ scope }) => scope.requestId))).toEqual(new Set([admission.scope.requestId, next.scope.requestId]));
  expect(all.find(({ scope }) => scope.requestId === admission.scope.requestId)).toEqual(original.entries[0]);
  await expect(f.agent.reviews.list({ ...query, cursor: other.scope.requestId })).rejects.toThrow("分页身份");
});
