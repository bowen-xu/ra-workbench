/** @vitest-environment happy-dom */
import { act } from "react";
import { expect, it, vi } from "vitest";
import { initialChanges, mixedFileChanges } from "../../../core/features/agent/WritingDiffFixtures";
import * as f from "./WritingDiffDemoAgentStorage.integrationHarness";
import { scope, Harness, journal, settled } from "./WritingDiffDemoAgentStorage.integrationHarness";

async function interruptedApply() {
  await f.agent.initialized;
  const native = f.agent.gateways.application.applications, execute = native.execute;
  const calls = vi.spyOn(native, "execute").mockImplementation(async request => {
    if (request.action === "prepareEditor" && request.current.path !== "main.tex") throw new Error("interrupt pending apply");
    return execute(request);
  });
  await act(async () => f.root.render(<Harness />));
  await settled(() => expect(f.documents.files.all().map(file => file.readOperations().length)).toEqual([1, 0, 0]));
  await settled(() => expect(f.host.querySelector('[aria-label="处理文件操作"], [aria-label="处理 Agent 文件修改"]')).not.toBeNull());
  const { state } = await journal();
  expect(state!.files.map(file => file.phase)).toEqual(["acknowledged", "pending", "pending"]);
  await act(async () => f.root.unmount()); f.resetRoot(); f.agent.recoverOnStartup();
  calls.mockImplementation(execute); calls.mockClear();
  Object.defineProperty(f.storage, "initialOpenPaths", {value: ["main.tex"]});
  await act(async () => f.root.render(<Harness />));
  await settled(() => expect(f.documents.ready && !!f.host.querySelector('[aria-label="处理文件操作"]') && f.documents.files.all().length === 3).toBe(true));
  const file = f.documents.files.get({manuscriptId: scope.manuscriptId, relativePath: "main.tex"})!;
  return {calls, native, original: state!, file};
}
it("compensates the original interrupted apply after owner rebuild without changing its old receipt", async () => {
  const {calls, native, original, file} = await interruptedApply();
  const apply = vi.spyOn(file.applicationPort, "apply");
  expect(file.applicationPort.read()?.sessionId).not.toBe(original.files[0]!.target.kind === "editor" && original.files[0]!.target.sessionId);
  await act(async () => expect(await f.documents.applicationActions.rollback(original.scope)).toMatchObject({kind: "state", state: {status: "rolledBack"}}));
  expect(apply).toHaveBeenCalledOnce();
  const completed = await native.execute({action: "read", scope: original.scope});
  expect(completed).toMatchObject({status: "rolledBack"});
  expect(completed.kind === "state" && completed.files[0]!.target).toEqual(original.files[0]!.target);
  expect(calls.mock.calls.filter(([request]) => request.action === "prepareRollback")).toHaveLength(1);
  expect(calls.mock.calls.filter(([request]) => request.action === "rollbackRemainder")).toHaveLength(1);
  for (const change of initialChanges) expect((await f.storage.files.readFile(scope.manuscriptId, change.path)).content).toBe(change.before);
});
it("offers protected original compensation in the workspace without a chat and settles its delivery only after saving", async () => {
  const {original, file} = await interruptedApply(), apply = vi.spyOn(file.applicationPort, "apply");
  const restore = [...f.host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "恢复应用前内容");
  expect(restore).toBeDefined();
  expect(f.host.textContent).not.toContain("应用与恢复记录");
  await act(async () => { restore!.click(); restore!.click(); });
  await settled(() => expect(f.documents.agentDelivery.pending).toBe(false));
  expect(apply).toHaveBeenCalledOnce();
  expect(await f.agent.gateways.application.applications.execute({action: "read", scope: original.scope})).toMatchObject({status: "rolledBack"});
  expect(await f.agent.gateways.delivery.deliveries.list({manuscriptId: scope.manuscriptId, after: null, limit: 64})).toMatchObject({items: []});
  for (const change of initialChanges) expect((await f.storage.files.readFile(scope.manuscriptId, change.path)).content).toBe(change.before);
});
it.each(["later input", "checkpoint failure", "later pending file"])("refuses original rebuilt compensation on %s before any write", async fault => {
  const {original, file} = await interruptedApply(), apply = vi.spyOn(file.applicationPort, "apply");
  await act(async () => {
    const target = fault === "later pending file" ? f.documents.files.all()[1]! : file;
    target.dispatch({changes: {from: 0, insert: "% later input\n"}, userEvent: "input.type"});
    if (fault === "checkpoint failure") f.storage.fault = "checkpoint";
    expect(await f.documents.applicationActions.rollback(original.scope)).toMatchObject({kind: "unconfirmed"});
  });
  expect(apply).not.toHaveBeenCalled();
  expect(file.read().content).toBe(`${fault === "later pending file" ? "" : "% later input\n"}${initialChanges[0]!.after}`);
  expect(await f.agent.gateways.application.applications.execute({action: "read", scope: original.scope})).toMatchObject({
    files: original.files, status: "recoveryRequired"});
  f.storage.fault = "none";
});
it("reconciles a rebuilt original compensation's lost confirmation without another editor write", async () => {
  const {original} = await interruptedApply(), native = f.agent.gateways.application.restorations, execute = native.execute;
  const calls = vi.spyOn(native, "execute").mockImplementation(request => request.action === "confirm"
    ? Promise.reject(new Error("lost restoration confirmation")) : execute(request));
  await act(async () => expect(await f.documents.applicationActions.rollback(original.scope)).toMatchObject({kind: "unconfirmed"}));
  const receipt = (await f.storage.checkpoints.list(scope.manuscriptId)).find(row => row.checkpoint.relativePath === "main.tex")!.checkpoint.editorReceipt!;
  expect(receipt.operationId).not.toBe(original.scope.operationId);
  expect(receipt.text).toBe(initialChanges[0]!.before);
  calls.mockImplementation(execute);
  await act(async () => f.root.unmount()); f.resetRoot();
  await act(async () => f.root.render(<Harness />));
  await settled(() => expect(f.documents.ready && f.documents.files.all().length === 3 && !!f.host.querySelector('[aria-label="处理文件操作"]')).toBe(true));
  const file = f.documents.files.all()[0]!, apply = vi.spyOn(file.applicationPort, "apply");
  expect(file.applicationPort.read()?.sessionId).not.toBe(receipt.sessionId);
  await act(async () => expect(await f.documents.applicationActions.rollback(original.scope)).toMatchObject({kind: "state", state: {status: "rolledBack"}}));
  expect(apply).not.toHaveBeenCalled();
  const completed = await f.agent.gateways.application.applications.execute({action: "read", scope: original.scope});
  expect(completed.kind === "state" && completed.files[0]!.target).toEqual(original.files[0]!.target);
});
async function rebuiltMixedApply() {
  await f.agent.initialized; f.configure(mixedFileChanges); f.agent.failDiskConfirmation = true;
  await act(async () => f.root.render(<Harness />));
  await settled(() => expect(f.host.querySelector('[aria-label="处理文件操作"], [aria-label="处理 Agent 文件修改"]')).not.toBeNull());
  const {state} = await journal();
  expect(state!.files.map(file => file.phase)).toEqual(["acknowledged", "acknowledged", "dispatched"]);
  await settled(() => expect(f.documents.saveState("main.tex")).toBe("saved"));
  expect((await f.storage.files.readFile(scope.manuscriptId, "main.tex")).content).toBe(mixedFileChanges[0]!.after);
  await act(async () => f.root.unmount()); f.resetRoot(); f.agent.recoverOnStartup();
  await act(async () => f.root.render(<Harness />));
  await settled(() => expect(f.documents.ready && !!f.host.querySelector('[aria-label="处理文件操作"]')).toBe(true));
  return state!;
}
it("compensates a rebuilt mixed apply whose editor result was already formally saved", async () => {
  const state = await rebuiltMixedApply(), file = f.documents.files.all()[0]!, apply = vi.spyOn(file.applicationPort, "apply");
  await act(async () => expect(await f.documents.applicationActions.rollback(state.scope)).toMatchObject({kind: "state", state: {status: "rolledBack"}}));
  expect(apply).toHaveBeenCalledOnce();
  expect((await f.storage.files.readFile(scope.manuscriptId, "main.tex")).content).toBe(mixedFileChanges[0]!.before);
  expect((await f.storage.files.listFiles(scope.manuscriptId)).map(file => file.relativePath)).not.toContain("sections/new.tex");
  expect((await f.storage.files.readFile(scope.manuscriptId, "assets/old.json")).content).toBe(mixedFileChanges[2]!.before);
  const current = await f.agent.gateways.application.applications.execute({action: "read", scope: state.scope});
  expect(current.kind === "state" && current.files[0]!.target).toEqual(state.files[0]!.target);
});
it("protects disk files if input follows rebuilt editor restoration", async () => {
  const state = await rebuiltMixedApply();
  const file = f.documents.files.all()[0]!, restore = f.agent.gateways.application.restorations.execute;
  vi.spyOn(f.agent.gateways.application.restorations, "execute").mockImplementation(async request => {
    const result = await restore(request);
    if (request.action === "confirm") file.dispatch({changes: {from: 0, insert: "% later input\n"}, userEvent: "input.type"});
    return result;
  });
  await act(async () => expect(await f.documents.applicationActions.rollback(state.scope)).toMatchObject({kind: "unconfirmed"}));
  expect(file.read().content).toBe(`% later input\n${mixedFileChanges[0]!.before}`);
  expect((await f.storage.files.listFiles(scope.manuscriptId)).map(file => file.relativePath)).toContain("sections/new.tex");
  expect((await f.storage.files.listFiles(scope.manuscriptId)).map(file => file.relativePath)).not.toContain("assets/old.json");
  const current = await f.agent.gateways.application.applications.execute({action: "read", scope: state.scope});
  expect(current.kind === "state" && current.files.map(file => file.phase)).toEqual(["restored", "acknowledged", "dispatched"]);
});

it.each(["surviving", "deleted"])("retains original compensation after formal save fails and owner rebuild with a %s chat, then retries saving without replay", async chat => {
  const {original} = await interruptedApply();
  const restorations = f.agent.gateways.application.restorations, execute = restorations.execute;
  vi.spyOn(restorations, "execute").mockImplementation(async request => {
    const result = await execute(request);
    if (request.action === "confirm") f.storage.fault = "file";
    return result;
  });
  await act(async () => [...f.host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "恢复应用前内容")!.click());
  await settled(() => expect(f.host.textContent).toContain("重试保存文件结果"));
  expect(f.documents.agentDelivery.pending).toBe(true);
  expect(await f.agent.gateways.application.applications.execute({action: "read", scope: original.scope})).toMatchObject({status: "rolledBack"});
  await act(async () => f.root.unmount()); f.resetRoot(); f.agent.recoverOnStartup();
  if (chat === "deleted") {
    const history = await f.agent.history.gateway.readHistory({ conversationId: scope.conversationId, afterSeq: 0, limit: 128 });
    await f.agent.deleteConversation(scope.conversationId, history.watermark);
  }
  await act(async () => f.root.render(<Harness />));
  await settled(() => expect(f.documents.ready && f.documents.files.all().length === 3).toBe(true));
  await settled(() => expect(f.host.textContent).toContain("重试保存文件结果"));
  expect(f.documents.agentDelivery.pending).toBe(true);
  const apply = vi.spyOn(f.documents.files.all()[0]!.applicationPort, "apply");
  f.storage.fault = "none";
  await act(async () => [...f.host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "重试保存文件结果")!.click());
  await settled(() => expect(f.documents.agentDelivery.pending).toBe(false));
  expect(apply).not.toHaveBeenCalled();
  for (const change of initialChanges) expect((await f.storage.files.readFile(scope.manuscriptId, change.path)).content).toBe(change.before);
});
