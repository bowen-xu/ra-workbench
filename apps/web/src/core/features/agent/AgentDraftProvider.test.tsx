/** @vitest-environment happy-dom */
import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { AgentDraftProvider, useAgentDraftStore, useConversationDraft } from "./AgentDraftProvider";
import { ConversationDraftStore, conversationDraftKey, type ConversationDraftScope } from "./conversationDraftStore";
import { insertAgentReference, serializeAgentSourceReferences } from "@research-assistant/core/agent";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const scope: ConversationDraftScope = { hostKind: "manuscript", hostId: "one", conversationId: "a" };

it.each([false, true])("acknowledges a draft captured before conversation creation, preserving later edits (%s)", edited => {
  const store = new ConversationDraftStore(), pending = conversationDraftKey({...scope, conversationId: null});
  const saved = conversationDraftKey(scope);
  store.update(pending, "hi"); const acknowledge = store.captureAcknowledgment(pending, "hi");
  store.adoptPending(scope, "a");
  if (edited) { store.update(saved, ""); store.update(saved, "hi"); }
  acknowledge();
  expect(store.read(saved)).toBe(edited ? "hi" : "");
});

it("acknowledges source-link wire text against its exact composer revision", () => {
  const store = new ConversationDraftStore(), key = conversationDraftKey(scope);
  const draft = insertAgentReference({text: "", bindings: []}, {type: "mention", name: "PDF", target: {kind: "file", sourceId: 'context:{"kind":"paper","resourceId":"p","attachmentId":"f"}', resourceId: "p", revision: "current"}});
  store.updateStructured(key, draft);
  const text = serializeAgentSourceReferences(draft).text;
  const old = store.captureAcknowledgment(key, text);
  store.update(key, "later"); old(); expect(store.read(key)).toBe("later");
  store.updateStructured(key, draft);
  store.captureAcknowledgment(key, text)(); expect(store.read(key)).toBe("");
});

it("clears only the exact acknowledged draft revision, never identical retyped text",()=>{
  const store=new ConversationDraftStore(),key=conversationDraftKey(scope);
  store.update(key,"original");const first=store.captureAcknowledgment(key,"original");
  store.update(key,"changed");store.update(key,"original");first();expect(store.read(key)).toBe("original");
  const second=store.captureAcknowledgment(key,"original");second();expect(store.read(key)).toBe("");
  store.update(key,"original");second();expect(store.read(key)).toBe("original");
  const deleted=store.captureAcknowledgment(key,"original");store.forgetConversation("a");deleted();expect(store.read(key)).toBe("");
});

it("forgets a deleted conversation across hosts and fences delayed draft updates", () => {
  const store = new ConversationDraftStore();
  const keys = [scope, { ...scope, hostId: "two" }, { ...scope, hostKind: "note" as const },
    { ...scope, conversationId: "b" }, { ...scope, conversationId: null }].map(conversationDraftKey);
  keys.forEach((key) => store.update(key, "draft"));
  const changed = vi.fn(); store.subscribe(keys[0]!, changed);
  store.forgetConversation("a");
  expect(keys.map((key) => store.read(key))).toEqual(["", "", "", "draft", "draft"]);
  store.update(keys[0]!, "late voice result");
  expect(store.read(keys[0]!)).toBe(""); expect(changed).toHaveBeenCalledOnce();
});

it("isolates scopes and roots, removes empty drafts and unsubscribes only the relevant listener", () => {
  const store = new ConversationDraftStore();
  const keys = [scope, { ...scope, hostId: "two" }, { ...scope, conversationId: "b" },
    { ...scope, hostKind: "note" as const }, { ...scope, hostKind: "readingNote" as const }, { ...scope, conversationId: null }].map(conversationDraftKey);
  const changed = vi.fn(), other = vi.fn();
  const unsubscribe = store.subscribe(keys[0]!, changed);
  store.subscribe(keys[1]!, other);
  keys.forEach((key, index) => store.update(key, `draft ${index}`));
  expect(keys.map((key) => store.read(key))).toEqual(keys.map((_, index) => `draft ${index}`));
  expect(changed).toHaveBeenCalledOnce(); expect(other).toHaveBeenCalledOnce();
  unsubscribe(); store.update(keys[0]!, "");
  expect(store.read(keys[0]!)).toBe(""); expect(changed).toHaveBeenCalledOnce();
  expect(new ConversationDraftStore().read(keys[1]!)).toBe("");
  expect(conversationDraftKey({ ...scope, hostId: "x:y", conversationId: "z" }))
    .not.toBe(conversationDraftKey({ ...scope, hostId: "x", conversationId: "y:z" }));
});

it("retains drafts across host unmounts and scopes delayed acknowledgments to the original draft", async () => {
  let current!: ReturnType<typeof useConversationDraft>;
  function Probe({ value }: { value: ConversationDraftScope }) {
    current = useConversationDraft(useAgentDraftStore(), value); return <p>{current[0]}</p>;
  }
  const container = document.createElement("div"), root = createRoot(container);
  const render = (value: ConversationDraftScope | null) => root.render(<StrictMode><AgentDraftProvider>
    {value && <Probe key={conversationDraftKey(value)} value={value} />}
  </AgentDraftProvider></StrictMode>);
  try {
    await act(async () => render(scope));
    const updateOriginal = current[1];
    await act(async () => updateOriginal("sent text"));
    await act(async () => render({ ...scope, conversationId: "b" }));
    expect(current[0]).toBe("");
    await act(async () => current[1]("second draft"));
    await act(async () => updateOriginal("newer original draft"));
    await act(async () => updateOriginal((value) => value === "sent text" ? "" : value));
    expect(current[0]).toBe("second draft");
    await act(async () => render(null));
    await act(async () => render(scope));
    expect(current[0]).toBe("newer original draft");
    await act(async () => render({ ...scope, conversationId: "b" }));
    expect(current[0]).toBe("second draft");
  } finally { await act(async () => root.unmount()); }
});
