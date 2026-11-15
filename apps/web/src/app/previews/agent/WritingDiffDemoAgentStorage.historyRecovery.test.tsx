/** @vitest-environment happy-dom */
import { act } from "react";
import { expect, it, vi } from "vitest";
import type { SourceNavigationRequest } from "../../../core/public/sourceNavigation";
import { initialChanges } from "../../../core/features/agent/WritingDiffFixtures";
import * as f from "./WritingDiffDemoAgentStorage.integrationHarness";
import { scope, Harness, mount, view, journal, settled } from "./WritingDiffDemoAgentStorage.integrationHarness";
const SOURCE_NAVIGATION_EVENT = "research-assistant:navigate-source";

it("opens the current resource from withdrawn history before and after restart without changing saved files or review decisions", async () => {
  vi.spyOn(HTMLDialogElement.prototype, "showModal").mockImplementation(function (this: HTMLDialogElement) {
    this.setAttribute("open", "");
  });
  vi.spyOn(HTMLDialogElement.prototype, "close").mockImplementation(function (this: HTMLDialogElement) {
    this.removeAttribute("open");
  });
  const originalView = await mount();
  await settled(() => expect(f.documents.agentDelivery.pending).toBe(false));
  await act(async () => f.host.querySelector<HTMLButtonElement>('[aria-label="Keep all"]')!.click());
  await act(async () => f.root.render(<Harness historyOpen />));
  await settled(() => expect(f.host.querySelector<HTMLButtonElement>(".writing-agent-inline-diff-undo")?.disabled).toBe(false));
  await act(async () => f.host.querySelector<HTMLButtonElement>(".writing-agent-inline-diff-undo")!.click());
  await settled(() => expect(initialChanges.every(change => f.documents.saveState(change.path) === "saved")).toBe(true));
  const { request, state: originalApplication } = await journal();
  await settled(() => expect(f.documents.undoRecovery.pending).toBe(false));
  const withdrawn = await f.agent.gateways.application.undos.find(originalApplication!.scope);
  expect(withdrawn?.result).toMatchObject({ status: "applied" });
  const savedCheckpoints = await f.storage.checkpoints.list(scope.manuscriptId);
  const savedFiles = await Promise.all(initialChanges.map(change => f.storage.files.readFile(scope.manuscriptId, change.path)));
  for (const [index, change] of initialChanges.entries()) expect(savedFiles[index]?.content).toBe(change.before);
  const file = f.documents.files.all()[0]!;
  expect(file.read()).toMatchObject({ content: initialChanges[0]!.before, reviewBaseline: initialChanges[0]!.after });
  expect(file.pending).toBe(true);

  async function readHistoryAndCheckUnchanged() {
    const currentView = view(), snapshot = f.documents.files.all().map(session => session.read());
    await act(async () => f.root.render(<Harness historyOpen />));
    await settled(() => expect(f.host.querySelector(".writing-agent-inline-diff-list button")).not.toBeNull());
    const navigate = vi.fn((event: Event) => {
      const request = (event as CustomEvent<SourceNavigationRequest>).detail;
      event.preventDefault(); request.resolve();
    });
    window.addEventListener(SOURCE_NAVIGATION_EVENT, navigate);
    try {
      await act(async () => f.host.querySelector<HTMLButtonElement>(".writing-agent-inline-diff-list button")!.click());
      expect(navigate).toHaveBeenCalledOnce();
      const request = (navigate.mock.calls[0]![0] as CustomEvent<SourceNavigationRequest>).detail;
      expect(request.link.path).toEqual(["writings", scope.manuscriptId, initialChanges[0]!.path]);
      expect(document.querySelector("dialog .cm-editor")).toBeNull();
    } finally { window.removeEventListener(SOURCE_NAVIGATION_EVENT, navigate); }
    expect(view()).toBe(currentView);
    expect(f.documents.files.all().map(session => session.read())).toEqual(snapshot);
    expect(await f.storage.checkpoints.list(scope.manuscriptId)).toEqual(savedCheckpoints);
    expect(await Promise.all(initialChanges.map(change => f.storage.files.readFile(scope.manuscriptId, change.path)))).toEqual(savedFiles);
    expect(await f.agent.gateways.application.applications.find(request)).toEqual(originalApplication);
    expect(await f.agent.gateways.application.undos.find(originalApplication!.scope)).toEqual(withdrawn);
  }
  await readHistoryAndCheckUnchanged();
  await act(async () => { expect(await f.documents.restart()).toBe(true); });
  await settled(() => expect(f.documents.ready && !f.documents.agentDelivery.pending).toBe(true));
  expect(view()).not.toBe(originalView);
  expect(f.documents.files.all()[0]!.read()).toMatchObject({ content: initialChanges[0]!.before, reviewBaseline: initialChanges[0]!.after });
  await readHistoryAndCheckUnchanged();
  await act(async () => { expect(await f.documents.restart()).toBe(true); });
  await settled(() => expect(f.documents.ready && !f.documents.agentDelivery.pending).toBe(true));
  expect(f.documents.files.all()[0]!.read()).toMatchObject({ content: initialChanges[0]!.before, reviewBaseline: initialChanges[0]!.after });
  expect(await f.storage.checkpoints.list(scope.manuscriptId)).toEqual(savedCheckpoints);
});
