/** @vitest-environment happy-dom */
import { act } from "react";
import { getChunks } from "@codemirror/merge";
import { expect, it } from "vitest";
import { resolveWritingChunk } from "../../../../../../plugins/writing_projects/src/features/writing/editor/writingDiffCommands";
import { initialChanges } from "./WritingDiffFixtures";
import * as f from "../../../app/previews/agent/WritingDiffDemoAgentStorage.integrationHarness";
import { scope, mount, view, journal, settled } from "../../../app/previews/agent/WritingDiffDemoAgentStorage.integrationHarness";

it("preserves the remaining manuscript diff across two chats, partial Keep and reopening", async () => {
  const current = await mount();
  await settled(() => expect(f.documents.agentDelivery.pending).toBe(false));
  const file = f.documents.files.all()[0]!, first = initialChanges[0]!;
  expect(file.read()).toMatchObject({ content: first.after, reviewBaseline: first.before });
  const second = { scope: { ...scope, conversationId: crypto.randomUUID(), requestId: crypto.randomUUID(),
    contextSnapshotId: crypto.randomUUID() }, turnId: crypto.randomUUID() };
  const next = file.read().content.replace("This paper presents", "This revision presents")
    .replace("The proposed framework clarifies", "The second dialogue clarifies");
  await f.agent.complete(second, [{ ...first, before: file.read().content, after: next }]);
  await act(async () => f.documents.agentDelivery.register(second));
  await settled(() => {
    expect(file.readOperations()).toHaveLength(2);
    expect(f.documents.agentDelivery.pending).toBe(false);
    expect(f.documents.saveState(first.path)).toBe("saved");
  });
  // Formal save persists the latest text without accepting either chat's changes.
  expect(file.read()).toMatchObject({ content: next, reviewBaseline: first.before });
  expect(getChunks(current.state)?.chunks).toHaveLength(2);
  expect(file.readOperations().map(operation => operation.conversationId))
    .toEqual([scope.conversationId, second.scope.conversationId]);
  const firstApplication = await journal();
  const secondProposal = (await f.agent.gateways.delivery.proposals.latest(second.scope))!;
  const secondRequest = { manuscriptId: scope.manuscriptId, conversationId: second.scope.conversationId,
    proposalId: secondProposal.proposalId };
  const secondApplication = await f.agent.gateways.application.applications.find(secondRequest);

  await act(async () => {
    expect(resolveWritingChunk(current, next.indexOf("This revision presents"), "keep")).toBe(true);
  });
  const oldSummary = first.before.split("\n").find(line => line.startsWith("\\summary"))!;
  const acceptedSummary = next.split("\n").find(line => line.startsWith("\\summary"))!;
  const baseline = first.before.replace(oldSummary, acceptedSummary);
  expect(file.read()).toMatchObject({ content: next, reviewBaseline: baseline });
  expect(file.pending).toBe(true);
  expect(getChunks(current.state)?.chunks).toHaveLength(1);
  await settled(() => expect(f.documents.saveState(first.path)).toBe("saved"));
  const operations = file.readOperations();

  let restarted = false;
  await act(async () => { restarted = await f.documents.restart(); });
  expect(restarted).toBe(true);
  await settled(() => {
    expect(f.documents.ready).toBe(true);
    expect(view()).not.toBe(current);
    expect(f.documents.agentDelivery.pending).toBe(false);
  });
  const restored = f.documents.files.all()[0]!;
  expect(restored.read()).toMatchObject({ content: next, reviewBaseline: baseline });
  expect(restored.pending).toBe(true);
  expect(getChunks(view().state)?.chunks).toHaveLength(1);
  expect(restored.readOperations()).toEqual(operations);
  expect((await journal()).state).toEqual(firstApplication.state);
  expect(await f.agent.gateways.application.applications.find(secondRequest)).toEqual(secondApplication);
  expect(f.host.querySelector(".recovery-banner")).toBeNull();
});
