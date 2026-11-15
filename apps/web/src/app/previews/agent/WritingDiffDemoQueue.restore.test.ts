import { expect, it } from "vitest";
import { WritingDiffDemoQueue } from "./WritingDiffDemoQueue";
import type { AgentSubmissionGateway } from "@research-assistant/data/agent";
const id = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
it("restores a removed paused item at its old position with exact receipt retry", async () => {
  // Queue-only harness: submission functions must not run during an edit.
  const unused = async () => {throw new Error("unexpected model submission");};
  const submission: AgentSubmissionGateway = {prepareWriting: unused, prepareWritingStatus: unused, startStatus: unused, startWriting: unused, restoreWriting: unused};
  const queue = new WritingDiffDemoQueue(submission), scope = {manuscriptId: id(10), conversationId: id(20)};
  const edit = async (edit: Parameters<typeof queue.gateway.edit>[0]["edit"]) => queue.gateway.edit({...scope, requestId: crypto.randomUUID(), expectedRevision: (await queue.gateway.read(scope)).revision, edit});
  await edit({type: "add", inputId: id(1), text: "first"});
  await edit({type: "add", inputId: id(2), text: "tail"});
  await edit({type: "pauseInput", inputId: id(1)});
  await edit({type: "remove", inputId: id(1)});
  const request = {...scope, requestId: crypto.randomUUID(), expectedRevision: (await queue.gateway.read(scope)).revision,
    edit: {type: "restore" as const, inputId: id(3), sourceInputId: id(1), index: 0, text: "edited"}};
  const restored = await queue.gateway.edit(request);
  expect(restored.inputs).toMatchObject([{inputId: id(3), text: "edited", paused: true}, {inputId: id(2), text: "tail"}]);
  expect(await queue.gateway.edit(request)).toEqual(restored);
  await expect(queue.gateway.edit({...request, edit: {...request.edit, index: 1}})).rejects.toMatchObject({ code: "conflict" });
});
