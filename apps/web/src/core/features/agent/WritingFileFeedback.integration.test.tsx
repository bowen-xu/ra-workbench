/** @vitest-environment happy-dom */
import { act } from "react";
import { expect, it, vi } from "vitest";
import * as f from "../../../app/previews/agent/WritingDiffDemoAgentStorage.integrationHarness";
import { Harness, journal, notificationSpies, scope, settled } from "../../../app/previews/agent/WritingDiffDemoAgentStorage.integrationHarness";
import { initialChanges } from "./WritingDiffFixtures";

it("hides a failed delivery without save notifications and retries the original operation without reapplying text", async () => {
  const native = f.agent.gateways.application.applications, execute = native.execute;
  vi.spyOn(native, "execute").mockImplementation(async request => {
    const result = await execute(request);
    if (request.action === "confirmEditor" && result.kind === "state" && result.status === "applied") f.storage.fault = "file";
    return result;
  });
  await act(async () => f.root.render(<Harness historyOpen />));
  await settled(() => expect(f.host.textContent).toContain("重试保存文件结果"));
  expect(f.host.querySelector('[aria-label="消息中的文件改动"]')).toBeNull();
  expect(f.host.textContent).not.toContain("文件修改待保存");
  expect((await journal()).state?.status).toBe("applied");
  for (const change of initialChanges) {
    expect(f.documents.files.get({ manuscriptId: scope.manuscriptId, relativePath: change.path })!.read().content).toBe(change.after);
    expect((await f.storage.files.readFile(scope.manuscriptId, change.path)).content).toBe(change.before);
  }
  expect(notificationSpies().error).not.toHaveBeenCalled();
  expect(notificationSpies().warning).not.toHaveBeenCalled();
  const dispatches = f.documents.files.all().map(file => vi.spyOn(file.applicationPort, "apply"));
  f.storage.fault = "none";
  await act(async () => [...f.host.querySelectorAll<HTMLButtonElement>("button")]
    .find(button => button.textContent === "重试保存文件结果")!.click());
  await settled(() => expect(f.documents.agentDelivery.pending).toBe(false));
  expect(f.host.querySelector('[aria-label="消息中的文件改动"]')?.textContent).toContain("Edited 3 files");
  expect(dispatches.every(call => call.mock.calls.length === 0)).toBe(true);
  for (const change of initialChanges) expect((await f.storage.files.readFile(scope.manuscriptId, change.path)).content).toBe(change.after);
});
