/** @vitest-environment happy-dom */
import { act } from "react";
import { expect, it, vi } from "vitest";
import { initialChanges, mixedFileChanges } from "../../../core/features/agent/WritingDiffFixtures";
import * as f from "./WritingDiffDemoAgentStorage.integrationHarness";
import { scope, Harness, mount, view, journal, settled } from "./WritingDiffDemoAgentStorage.integrationHarness";
async function rebuiltPartialUndo() {
  await mount();
  await settled(() => expect(initialChanges.every(change => f.documents.saveState(change.path) === "saved")).toBe(true));
  const { request, state } = await journal(), native = f.agent.gateways.application.undos;
  const execute = native.execute;
  const calls = vi.spyOn(native, "execute").mockImplementation(async input => {
    if (input.command.action === "prepareEditor" && input.command.current.path !== "main.tex") throw new Error("interrupt pending file");
    return execute(input);
  });
  await act(async () => expect((await f.documents.undoProposal(request)).kind).toBe("unconfirmed"));
  const interrupted = (await native.find(state!.scope))!;
  expect(interrupted.result?.kind === "state" && interrupted.result.files.map(file => file.phase)).toEqual(["acknowledged", "pending", "pending"]);
  await act(async () => f.root.unmount()); f.resetRoot();
  Object.defineProperty(f.storage, "initialOpenPaths", { value: ["main.tex"] });
  calls.mockImplementation(execute); calls.mockClear();
  await act(async () => f.root.render(<Harness />));
  await settled(() => expect(f.host.textContent).toContain("继续原撤销"));
  const file = f.documents.files.get({ manuscriptId: scope.manuscriptId, relativePath: "main.tex" })!;
  return { calls, native, state, interrupted, file };
}
it("continues a partially confirmed Undo after rebuilding owners without replaying its confirmed file", async () => {
  const { calls, native, state, interrupted, file } = await rebuiltPartialUndo();
  const apply = vi.spyOn(file.applicationPort, "apply");
  await act(async () => [...f.host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "继续原撤销")!.click());
  await settled(() => expect(f.documents.undoRecovery.pending).toBe(false));
  expect(apply).not.toHaveBeenCalled();
  expect(calls.mock.calls.filter(([input]) => input.command.action === "prepare" || input.command.action === "prepareEditor")
    .map(([input]) => input.command.action === "prepareEditor" && input.command.current.path)).toEqual(initialChanges.slice(1).map(change => change.path));
  const completed = (await native.find(state!.scope))!;
  expect(completed.undoId).toBe(interrupted.undoId);
  if (completed.result?.kind !== "state" || interrupted.result?.kind !== "state") throw new Error("missing Undo");
  expect(completed.result.files[0]).toEqual(interrupted.result.files[0]);
  for (const change of initialChanges) expect((await f.storage.files.readFile(scope.manuscriptId, change.path)).content).toBe(change.before);
});
it("compensates a partial Undo after rebuilding owners while preserving the original Undo receipt", async () => {
  const { native, state, interrupted, file } = await rebuiltPartialUndo();
  const apply = vi.spyOn(file.applicationPort, "apply");
  await act(async () => [...f.host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "恢复撤销前内容")!.click());
  await settled(() => expect(f.documents.undoRecovery.pending).toBe(false));
  expect(apply).toHaveBeenCalledOnce();
  const completed = (await native.find(state!.scope))!;
  expect(completed).toMatchObject({ undoId: interrupted.undoId, result: { status: "rolledBack" } });
  expect(completed.result?.kind === "state" && completed.result.files[0]?.target).toEqual(interrupted.result?.kind === "state" && interrupted.result.files[0]?.target);
  for (const change of initialChanges) expect((await f.storage.files.readFile(scope.manuscriptId, change.path)).content).toBe(change.after);
});
it.each(["later input", "checkpoint failure"])("refuses rebuilt compensation on %s before restoring any file", async fault => {
  const { native, state, interrupted, file } = await rebuiltPartialUndo();
  const apply = vi.spyOn(file.applicationPort, "apply");
  await act(async () => {
    if (fault === "later input") file.dispatch({ changes: { from: 0, insert: "later input\n" }, userEvent: "input.type" });
    else {
      // A new state version with identical bytes still requires a reliable CAS checkpoint.
      file.dispatch({ changes: { from: 0, insert: "temporary" }, userEvent: "input.type" });
      file.dispatch({ changes: { from: 0, to: 9, insert: "" }, userEvent: "delete.backward" });
      f.storage.fault = "checkpoint";
    }
    [...f.host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "恢复撤销前内容")!.click();
  });
  await settled(() => expect(f.notificationSpies().error).toHaveBeenCalledWith(expect.stringContaining("结果仍未确认"), expect.anything()));
  expect(apply).not.toHaveBeenCalled();
  expect(file.read().content).toBe(`${fault === "later input" ? "later input\n" : ""}${initialChanges[0]!.before}`);
  expect(await native.find(state!.scope)).toEqual(interrupted);
  for (const change of initialChanges.slice(1)) expect(f.documents.files.get({ manuscriptId: scope.manuscriptId, relativePath: change.path })!.read().content).toBe(change.after);
  f.storage.fault = "none";
});
it("recovers a rebuilt compensation's lost confirmation after another rebuild without redispatch", async () => {
  const { native, state, interrupted } = await rebuiltPartialUndo();
  const factory = f.agent.gateways.application.undoRestorations;
  const gateway = vi.spyOn(f.agent.gateways.application, "undoRestorations").mockImplementation(scope => {
    const restoration = factory(scope);
    return { execute: request => request.action === "confirm"
      ? Promise.reject(new Error("lost compensation confirmation")) : restoration.execute(request) };
  });
  await act(async () => [...f.host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "恢复撤销前内容")!.click());
  await settled(() => expect(f.notificationSpies().error).toHaveBeenCalledWith(expect.stringContaining("结果仍未确认"), expect.anything()));
  const receipt = (await f.storage.checkpoints.list(scope.manuscriptId))
    .find(row => row.checkpoint.relativePath === "main.tex")!.checkpoint.editorReceipt!;
  expect(receipt.operationId).not.toBe(interrupted.undoId);
  expect(receipt.text).toBe(initialChanges[0]!.after);
  gateway.mockRestore();
  await act(async () => f.root.unmount()); f.resetRoot();
  await act(async () => f.root.render(<Harness />));
  await settled(() => expect(f.host.textContent).toContain("恢复撤销前内容"));
  const file = f.documents.files.get({ manuscriptId: scope.manuscriptId, relativePath: "main.tex" })!;
  expect(file.applicationPort.read()?.sessionId).not.toBe(receipt.sessionId);
  const apply = vi.spyOn(file.applicationPort, "apply");
  await act(async () => [...f.host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "恢复撤销前内容")!.click());
  await settled(() => expect(f.documents.undoRecovery.pending).toBe(false));
  expect(apply).not.toHaveBeenCalled();
  expect(await native.find(state!.scope)).toMatchObject({ undoId: interrupted.undoId, result: { status: "rolledBack" } });
  for (const change of initialChanges) expect((await f.storage.files.readFile(scope.manuscriptId, change.path)).content).toBe(change.after);
});
it.each(["later input", "checkpoint failure"])("stops rebuilt partial Undo on %s without replaying confirmed text", async fault => {
  const { calls, native, state, interrupted, file } = await rebuiltPartialUndo();
  const apply = vi.spyOn(file.applicationPort, "apply");
  await act(async () => {
    file.dispatch({ changes: { from: 0, insert: "later input\n" }, userEvent: "input.type" });
    if (fault === "checkpoint failure") f.storage.fault = "checkpoint";
    [...f.host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "继续原撤销")!.click();
  });
  await settled(() => expect(f.notificationSpies().error).toHaveBeenCalledWith(expect.stringContaining("结果仍未确认"), expect.anything()));
  expect(file.read().content).toBe(`later input\n${initialChanges[0]!.before}`);
  expect(f.documents.undoRecovery.pending).toBe(true);
  expect(apply).not.toHaveBeenCalled();
  expect(calls.mock.calls.filter(([input]) => ["prepare", "prepareEditor", "applyDisk"].includes(input.command.action))).toEqual([]);
  const current = (await native.find(state!.scope))!;
  expect(current).toEqual(interrupted);
  for (const change of initialChanges.slice(1)) expect(f.documents.files.get({ manuscriptId: scope.manuscriptId, relativePath: change.path })!.read().content).toBe(change.after);
  f.storage.fault = "none";
});
it.each(["prepared", "recoveryRequired"] as const)("explicitly resumes a wholly pending %s Undo with restored file sessions", async status => {
  await mount();
  await settled(() => expect(initialChanges.every(change => f.documents.saveState(change.path) === "saved")).toBe(true));
  const { state } = await journal(), original = state!.scope, undoId = crypto.randomUUID();
  const native = f.agent.gateways.application.undos;
  const prepare = await native.execute({ scope: original, undoId, command: { action: "prepare", buffers: f.documents.files.readBuffers(scope.manuscriptId) } });
  if (status === "recoveryRequired") {
    await native.execute({ scope: original, undoId, command: { action: "start", buffers: f.documents.files.readBuffers(scope.manuscriptId) } });
    await native.execute({ scope: original, undoId, command: { action: "defer" } });
  }
  const frozen = structuredClone(prepare);
  await act(async () => f.root.unmount()); f.resetRoot();
  Object.defineProperty(f.storage, "initialOpenPaths", { value: ["main.tex"] });
  await act(async () => f.root.render(<Harness />));
  await settled(() => expect(f.documents.undoRecovery.pending).toBe(true));
  await settled(() => expect(f.host.textContent).toContain("继续原撤销"));
  const buffers = f.documents.files.readBuffers(scope.manuscriptId);
  expect(buffers.every(buffer => prepare.result?.kind === "state" && prepare.result.files.some(file => file.path === buffer.path
    && file.target.kind === "editor" && file.target.sessionId !== buffer.sessionId))).toBe(true);
  const calls = vi.spyOn(native, "execute");
  await act(async () => [...f.host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "继续原撤销")!.click());
  await settled(() => expect(f.documents.undoRecovery.pending).toBe(false));
  expect(calls.mock.calls.filter(([request]) => request.command.action === "prepare")).toEqual([]);
  expect(calls.mock.calls.filter(([request]) => request.command.action === "prepareEditor").map(([request]) => request.undoId)).toEqual(initialChanges.map(() => undoId));
  expect(prepare).toEqual(frozen); expect(f.documents.openPaths).toEqual(["main.tex"]);
  for (const change of initialChanges) expect((await f.storage.files.readFile(scope.manuscriptId, change.path)).content).toBe(change.before);
});
it("reliably checkpoints each Undo receipt before native confirmation", async () => {
  await mount();
  await settled(() => expect(initialChanges.every(change => f.documents.saveState(change.path) === "saved")).toBe(true));
  const { request, state } = await journal();
  const execute = f.agent.gateways.application.undos.execute;
  const confirmations: string[] = [];
  vi.spyOn(f.agent.gateways.application.undos, "execute").mockImplementation(async request => {
    if (request.command.action === "confirmEditor") {
      const actual = request.command.actual;
      const checkpoint = (await f.storage.checkpoints.list(scope.manuscriptId))
        .find(row => row.checkpoint.relativePath === actual.path)!.checkpoint;
      expect(checkpoint.editorReceipt).toEqual({ operationId: request.undoId,
        sessionId: request.command.actual.sessionId, revision: request.command.actual.revision, text: request.command.actual.text,
        stateVersion: checkpoint.version });
      expect(checkpoint.operations.at(-1)).toMatchObject({ operationId: request.undoId,
        conversationId: scope.conversationId, requestId: scope.requestId });
      confirmations.push(request.command.actual.path);
    }
    return execute(request);
  });
  await act(async () => { expect(await f.documents.undoProposal(request)).toMatchObject({ kind: "state", state: { status: "applied" } }); });
  expect(confirmations).toEqual(initialChanges.map(change => change.path));
  expect((await f.agent.gateways.application.applications.find(request))?.scope).toEqual(state!.scope);
});
it("recovers a lost Undo confirmation after rebuilding files without replaying the old text", async () => {
  await mount();
  await settled(() => expect(initialChanges.every(change => f.documents.saveState(change.path) === "saved")).toBe(true));
  const { request, state } = await journal(), execute = f.agent.gateways.application.undos.execute;
  const calls = vi.spyOn(f.agent.gateways.application.undos, "execute").mockImplementation(async request => {
    if (request.command.action === "confirmEditor") throw new Error("lost Undo confirmation");
    return execute(request);
  });
  await act(async () => { expect((await f.documents.undoProposal(request)).kind).toBe("unconfirmed"); });
  const interrupted = (await f.agent.gateways.application.undos.find(state!.scope))!;
  const oldReceipt = f.documents.files.get({ manuscriptId: scope.manuscriptId, relativePath: "main.tex" })!
    .applicationPort.readReceipt(interrupted.undoId)!;
  await act(async () => view().dispatch({ changes: { from: view().state.doc.length, insert: "\nnew input" } }));
  await act(async () => f.root.unmount()); f.resetRoot();
  calls.mockImplementation(execute); calls.mockClear();
  await act(async () => f.root.render(<Harness />));
  await settled(() => expect(f.documents.ready).toBe(true));
  const file = f.documents.files.get({ manuscriptId: scope.manuscriptId, relativePath: "main.tex" })!;
  expect(file.applicationPort.read()!.sessionId).not.toBe(oldReceipt.snapshot.sessionId);
  expect(file.applicationPort.readReceipt(interrupted.undoId)).toEqual(oldReceipt);
  const text = file.read().content, apply = vi.spyOn(file.applicationPort, "apply");
  let result;
  await act(async () => { result = await f.documents.applicationActions.recoverUndo(state!.scope, interrupted.undoId, "main.tex"); });
  expect(result).toMatchObject({ kind: "state", state: { files: expect.arrayContaining([{ path: "main.tex",
    phase: "acknowledged", target: expect.any(Object) }]) } });
  expect(apply).not.toHaveBeenCalled(); expect(file.read().content).toBe(text);
  expect(text).toContain("new input");
  expect(calls.mock.calls.filter(([request]) => request.command.action === "confirmEditor")[0]?.[0])
    .toMatchObject({ undoId: interrupted.undoId, command: { actual: { sessionId: oldReceipt.snapshot.sessionId,
      revision: oldReceipt.snapshot.revision, text: oldReceipt.snapshot.text, path: "main.tex" } } });
});
it("rediscovers a completed Undo awaiting formal save and retries without editor replay", async () => {
  await mount();
  await settled(() => expect(initialChanges.every(change => f.documents.saveState(change.path) === "saved")).toBe(true));
  const { request, state } = await journal();
  const execute = f.agent.gateways.application.undos.execute;
  const calls = vi.spyOn(f.agent.gateways.application.undos, "execute").mockImplementation(async request => {
    const result = await execute(request);
    if (result.result?.kind === "state" && result.result.status === "applied") f.storage.fault = "file";
    return result;
  });
  await act(async () => { expect(await f.documents.undoProposal(request)).toMatchObject({ kind: "unconfirmed", state: { status: "applied" } }); });
  const undoId = (await f.agent.gateways.application.undos.find(state!.scope))!.undoId;
  const save = vi.spyOn(f.storage.files, "saveFile"), checkpoint = vi.spyOn(f.storage.checkpoints, "save");
  expect((await f.agent.gateways.pendingUndos.list({ manuscriptId: scope.manuscriptId, after: null, limit: 64 })).items[0]?.undo.undoId).toBe(undoId);
  expect(save).not.toHaveBeenCalled(); expect(checkpoint).not.toHaveBeenCalled();
  await act(async () => f.root.unmount()); f.resetRoot();
  // Persisted tabs need not include every file changed by the original operation.
  Object.defineProperty(f.storage, "initialOpenPaths", { value: ["main.tex"] });
  calls.mockImplementation(execute); calls.mockClear();
  await act(async () => f.root.render(<Harness />));
  await settled(() => expect(f.documents.ready).toBe(true));
  await settled(() => expect(f.host.textContent).toContain("重试保存撤销结果"));
  await settled(() => expect(f.documents.files.all()).toHaveLength(3));
  expect(f.documents.openPaths).toEqual(["main.tex"]);
  expect(f.documents.undoRecovery.pending).toBe(true);
  const apply = f.documents.files.all().map(file => vi.spyOn(file.applicationPort, "apply"));
  f.storage.fault = "none";
  await act(async () => [...f.host.querySelectorAll<HTMLButtonElement>("button")]
    .find(button => button.textContent === "重试保存撤销结果")!.click());
  await settled(() => expect(f.documents.undoRecovery.pending).toBe(false));
  expect(apply.every(call => call.mock.calls.length === 0)).toBe(true);
  expect(calls.mock.calls.every(([request]) => request.command.action === "read")).toBe(true);
  for (const change of initialChanges) expect((await f.storage.files.readFile(scope.manuscriptId, change.path)).content).toBe(change.before);
  expect((await f.agent.gateways.pendingUndos.list({ manuscriptId: scope.manuscriptId, after: null, limit: 64 })).items).toEqual([]);
  f.storage.fault = "file";
  await act(async () => view().dispatch({ changes: { from: view().state.doc.length, insert: "\nordinary unsaved input" } }));
  const file = f.documents.files.all()[0]!, identity = file.identity;
  // A later ordinary save failure cannot revive an already saved Undo.
  await act(async () => { await expect(f.documents.applicationActions.undo(state!.scope, undoId)).resolves.toMatchObject({ kind: "unconfirmed" }); });
  expect((await f.storage.checkpoints.list(scope.manuscriptId)).find(row => row.checkpoint.relativePath === identity.relativePath)!.checkpoint.content).toContain("ordinary unsaved input");
  expect((await f.agent.gateways.pendingUndos.list({ manuscriptId: scope.manuscriptId, after: null, limit: 64 })).items).toEqual([]);
});
it("rediscovers completed Undo compensation with a pending save after rebuilding files", async () => {
  await f.agent.initialized; f.configure(mixedFileChanges);
  const saveFile = f.storage.files.saveFile;
  let failSave = false;
  vi.spyOn(f.storage.files, "saveFile").mockImplementation(request => failSave
    ? Promise.reject(new Error("formal save failed")) : saveFile(request));
  await act(async () => f.root.render(<Harness />));
  await settled(() => expect(f.documents.agentDelivery.pending).toBe(false));
  const { request, state } = await journal();
  f.storage.fault = "fileReceipt";
  await act(async () => { await f.documents.undoProposal(request); });
  f.storage.fault = "none";
  await vi.waitFor(async () => expect((await f.storage.files.readFile(scope.manuscriptId, "main.tex")).content).toBe(mixedFileChanges[0]!.before));
  failSave = true;
  await act(async () => [...f.host.querySelectorAll<HTMLButtonElement>("button")]
    .find(button => button.textContent === "恢复撤销前内容")!.click());
  await settled(() => expect(f.host.textContent).toContain("重试保存撤销结果"));
  const terminal = (await f.agent.gateways.application.undos.find(state!.scope))!;
  expect(terminal.result).toMatchObject({ kind: "state", status: "rolledBack" });
  const checkpoints = await f.storage.checkpoints.list(scope.manuscriptId);
  const receipt = checkpoints.find(row => row.checkpoint.relativePath === "main.tex")!.checkpoint.editorReceipt!;
  expect(receipt.operationId).not.toBe(terminal.undoId);
  expect((await f.agent.gateways.pendingUndos.list({ manuscriptId: scope.manuscriptId, after: null, limit: 64 })).items[0]?.undo.undoId).toBe(terminal.undoId);
  await act(async () => f.root.unmount()); f.resetRoot();
  await act(async () => f.root.render(<Harness />));
  await settled(() => expect(f.documents.ready).toBe(true));
  await settled(() => expect(f.host.textContent).toContain("重试保存撤销结果"));
  expect(f.documents.files.all()[0]!.applicationPort.readReceipt(receipt.operationId)?.snapshot.text).toBe(receipt.text);
  const apply = vi.spyOn(f.documents.files.all()[0]!.applicationPort, "apply");
  failSave = false;
  await act(async () => [...f.host.querySelectorAll<HTMLButtonElement>("button")]
    .find(button => button.textContent === "重试保存撤销结果")!.click());
  await settled(() => expect(f.documents.undoRecovery.pending).toBe(false));
  expect(apply).not.toHaveBeenCalled();
  expect((await f.storage.files.readFile(scope.manuscriptId, "main.tex")).content).toBe(mixedFileChanges[0]!.after);
  expect((await f.agent.gateways.pendingUndos.list({ manuscriptId: scope.manuscriptId, after: null, limit: 64 })).items).toEqual([]);
});
