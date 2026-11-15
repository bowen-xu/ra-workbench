import { expect, it } from "vitest";
import { agentDraftInput, insertAgentReference, trimmedAgentDraftInput, type AgentQueueState, type AgentReferenceInput } from "@research-assistant/core/agent";
import { ConversationDraftStore, conversationDraftKey } from "./conversationDraftStore";
import { localConversationDraftPersistence } from "./conversationDraftPersistence";
import { queueDraftEdit } from "./queueComposerDraft";

const id = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
const key = conversationDraftKey({hostKind: "manuscript", hostId: id(10), conversationId: id(20)});
const references: readonly AgentReferenceInput[] = [
  {type: "skill", name: "review", identity: {environment_id: "app", source_id: "review/SKILL.md", revision: "v1"}},
  {type: "mention", name: "tools", target: {kind: "tool", serverId: "mcp", environmentId: "app", revision: "v2"}},
  {type: "mention", name: "main.tex", target: {kind: "file", sourceId: "workspace", resourceId: id(10), revision: "v3"}},
];
function fixture() {
  let draft = {text: "😀 修改 ", bindings: []} as ReturnType<typeof insertAgentReference>;
  for (const reference of references) draft = insertAgentReference(draft, reference);
  const input = {inputId: id(2), text: draft.text, userInput: agentDraftInput(draft), paused: true, origin: {kind: "manuscript" as const, resourceId: id(10)}};
  const queue: AgentQueueState = {schemaVersion: 1, conversationId: id(20), revision: 1, paused: false, preparingInputId: null,
    inputs: [{inputId: id(1), text: "first"}, input, {inputId: id(3), text: "last"}]};
  const store = new ConversationDraftStore();
  store.update(key, "replace this draft");
  store.restoreQueue(key, {input, index: 1, preceding: [id(1)], following: [id(3)]});
  return {store, queue, input};
}

it("replaces the composer with all bound resources and persists the original slot and provenance", () => {
  const {store, input} = fixture();
  expect(store.read(key)).toBe(input.text);
  expect(agentDraftInput(store.readStructured(key))).toEqual(input.userInput);
  store.update(key, `前言 ${input.text}`);
  expect(store.readStructured(key).bindings.map(binding => binding.input)).toEqual(references);
  let bytes: string | null = null;
  const persistence = localConversationDraftPersistence({getItem: () => bytes, setItem: (_key, value) => {bytes = value;}});
  persistence.save(new Map([[key, {draft: store.readStructured(key), revision: null, originKey: null, sourceQueueRevision: null}]]));
  const restored = new ConversationDraftStore(persistence);
  expect(restored.readStructured(key)).toEqual(store.readStructured(key));
  expect(restored.readStructured(key).queuePlacement?.input).toEqual(input);
  restored.update(key, "");
  expect(restored.readStructured(key).queuePlacement?.index).toBe(1);
});

it("returns an edited message before its surviving successor after earlier messages dispatch", () => {
  const {store, queue} = fixture();
  const draft = store.readStructured(key);
  const current = {...queue, inputs: queue.inputs.slice(2)};
  expect(queueDraftEdit(draft, current, draft.text.trim(), trimmedAgentDraftInput(draft), () => id(4)))
    .toMatchObject({type: "restore", sourceInputId: id(2), inputId: id(4), index: 0});
  expect(queueDraftEdit(draft, {...queue, inputs: queue.inputs.slice(0, 1)}, "edited", [], () => id(4)))
    .toMatchObject({type: "restore", index: 1});
  expect(queueDraftEdit(draft, queue, "removal rejected; edit the retained item", [], () => id(4)))
    .toMatchObject({type: "update", inputId: id(2)});
});

it("clears the slot only on acceptance and preserves a newer draft as a fresh tail input", () => {
  const {store, queue} = fixture();
  const ack = store.captureAcknowledgment(key, store.read(key).trim());
  store.update(key, `${store.read(key)} new text`);
  const newer = store.read(key);
  ack();
  expect(store.read(key)).toBe(newer);
  expect(store.readStructured(key).queuePlacement).toBeUndefined();
  expect(queueDraftEdit(store.readStructured(key), queue, newer.trim(), store.input(key, newer.trim()), () => id(4)))
    .toMatchObject({type: "add", inputId: id(4)});
  const clean = fixture().store;
  clean.captureAcknowledgment(key, clean.read(key).trim())();
  expect(clean.readStructured(key)).toEqual({text: "", bindings: []});
});

it("retains legacy resource items without visible span bindings through editing and storage", () => {
  const {store} = fixture();
  const input = [{type: "text" as const, text: "legacy", textElements: []}, ...references];
  store.restoreQueue(key, {input: {inputId: id(2), text: "legacy", userInput: input}, index: 0, preceding: [], following: []});
  store.update(key, "edited legacy");
  expect(store.input(key, "edited legacy").slice(1)).toEqual(references);
});
