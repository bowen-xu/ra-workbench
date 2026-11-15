/** @vitest-environment happy-dom */
import { act } from "react";
import { expect, it, vi } from "vitest";
import { initialChanges } from "../../../core/features/agent/WritingDiffFixtures";
import * as f from "./WritingDiffDemoAgentStorage.integrationHarness";
import { scope, Harness, journal, settled } from "./WritingDiffDemoAgentStorage.integrationHarness";

async function deleteOriginAndRebuild() {
  await act(async () => f.root.unmount()); f.resetRoot();
  const history = await f.agent.history.gateway.readHistory({ conversationId: scope.conversationId, afterSeq: 0, limit: 128 });
  await f.agent.deleteConversation(scope.conversationId, history.watermark);
  f.agent.recoverOnStartup();
  Object.defineProperty(f.storage, "initialOpenPaths", { value: ["main.tex"] });
  const historyOpen = vi.spyOn(f.agent.gateways.delivery.sessions, "open");
  await act(async () => f.root.render(<Harness />));
  await settled(() => expect(f.documents.ready && f.documents.files.all().length === 3).toBe(true));
  expect(historyOpen).not.toHaveBeenCalled();
  expect(f.documents.openPaths).toEqual(["main.tex"]);
}

it("discovers an admitted original application after its chat is deleted and compensates through the file workspace", async () => {
  await f.agent.initialized;
  const native = f.agent.gateways.application.applications, execute = native.execute;
  const calls = vi.spyOn(native, "execute").mockImplementation(request =>
    request.action === "prepareEditor" && request.current.path !== "main.tex"
      ? Promise.reject(new Error("interrupt original application")) : execute(request));
  await act(async () => f.root.render(<Harness />));
  await settled(() => expect(f.host.querySelector('[aria-label="处理文件操作"], [aria-label="处理 Agent 文件修改"]')).not.toBeNull());
  const original = (await journal()).state!;
  expect(original.files.map(file => file.phase)).toEqual(["acknowledged", "pending", "pending"]);
  await act(async () => f.root.unmount()); f.resetRoot();
  const history = await f.agent.history.gateway.readHistory({ conversationId: scope.conversationId, afterSeq: 0, limit: 128 });
  await f.agent.deleteConversation(scope.conversationId, history.watermark);
  f.agent.recoverOnStartup(); calls.mockImplementation(execute); calls.mockClear();
  Object.defineProperty(f.storage, "initialOpenPaths", { value: ["main.tex"] });
  await act(async () => f.root.render(<Harness />));
  await settled(() => expect(f.documents.ready && f.documents.files.all().length === 3).toBe(true));
  await settled(() => expect(f.host.textContent).toContain("恢复应用前内容"));
  expect(f.documents.agentDelivery.pending).toBe(true);
  expect(calls.mock.calls.every(([request]) => request.action === "read")).toBe(true);
  const apply = vi.spyOn(f.documents.files.all()[0]!.applicationPort, "apply");
  await act(async () => [...f.host.querySelectorAll<HTMLButtonElement>("button")]
    .find(button => button.textContent === "恢复应用前内容")!.click());
  await settled(() => expect(f.documents.agentDelivery.pending).toBe(false));
  expect(apply).toHaveBeenCalledOnce();
  expect(await native.execute({ action: "read", scope: original.scope })).toMatchObject({ status: "rolledBack" });
  for (const change of initialChanges) expect((await f.storage.files.readFile(scope.manuscriptId, change.path)).content).toBe(change.before);
  expect(f.host.textContent).not.toContain("应用与恢复记录");
});

it("continues the original wholly pending application after chat deletion only on an explicit command", async () => {
  f.agent.failBeforeDispatch = true;
  await act(async () => f.root.render(<Harness />));
  await settled(() => expect(f.host.querySelector('[aria-label="处理文件操作"], [aria-label="处理 Agent 文件修改"]')).not.toBeNull());
  const original = (await journal()).state!;
  expect(original.files.every(file => file.phase === "pending")).toBe(true);
  f.agent.failBeforeDispatch = false;
  await deleteOriginAndRebuild();
  await settled(() => expect(f.host.textContent).toContain("继续原文件修改"));
  expect(f.documents.files.all().map(file => file.readOperations().length)).toEqual([0, 0, 0]);
  await act(async () => [...f.host.querySelectorAll<HTMLButtonElement>("button")]
    .find(button => button.textContent === "继续原文件修改")!.click());
  await settled(() => expect(f.documents.agentDelivery.pending).toBe(false));
  expect(await f.agent.gateways.application.applications.execute({ action: "read", scope: original.scope })).toMatchObject({ status: "applied", scope: original.scope });
  expect(f.documents.files.all().map(file => file.readOperations().length)).toEqual([1, 1, 1]);
  for (const change of initialChanges) expect((await f.storage.files.readFile(scope.manuscriptId, change.path)).content).toBe(change.after);
});

it("rediscovers a terminal original application with unsaved receipts after chat deletion and retries saving without replay", async () => {
  const native = f.agent.gateways.application.applications, execute = native.execute;
  vi.spyOn(native, "execute").mockImplementation(async request => {
    const result = await execute(request);
    if (request.action === "confirmEditor" && result.kind === "state" && result.status === "applied") f.storage.fault = "file";
    return result;
  });
  await act(async () => f.root.render(<Harness />));
  await settled(() => expect(f.host.textContent).toContain("重试保存文件结果"));
  const original = (await journal()).state!;
  expect(original.status).toBe("applied");
  const save = vi.spyOn(f.storage.files, "saveFile"), checkpoint = vi.spyOn(f.storage.checkpoints, "save");
  expect((await f.agent.gateways.pendingApplications.list({ manuscriptId: scope.manuscriptId, after: null, limit: 64 })).items[0]?.application.scope).toEqual(original.scope);
  expect(save).not.toHaveBeenCalled(); expect(checkpoint).not.toHaveBeenCalled();
  await deleteOriginAndRebuild();
  await settled(() => expect(f.host.textContent).toContain("重试保存文件结果"));
  const writes = f.documents.files.all().map(file => vi.spyOn(file.applicationPort, "apply"));
  f.storage.fault = "none";
  await act(async () => [...f.host.querySelectorAll<HTMLButtonElement>("button")]
    .find(button => button.textContent === "重试保存文件结果")!.click());
  await settled(() => expect(f.documents.agentDelivery.pending).toBe(false));
  expect(writes.every(call => call.mock.calls.length === 0)).toBe(true);
  expect((await f.agent.gateways.pendingApplications.list({ manuscriptId: scope.manuscriptId, after: null, limit: 64 })).items).toEqual([]);
  for (const change of initialChanges) expect((await f.storage.files.readFile(scope.manuscriptId, change.path)).content).toBe(change.after);
  f.storage.fault = "file";
  await act(async () => {
    f.documents.files.all()[0]!.dispatch({ changes: { from: 0, insert: "% later ordinary input\n" }, userEvent: "input.type" });
    await f.documents.applicationActions.accept(original.scope);
  });
  expect((await f.agent.gateways.pendingApplications.list({ manuscriptId: scope.manuscriptId, after: null, limit: 64 })).items).toEqual([]);
});
