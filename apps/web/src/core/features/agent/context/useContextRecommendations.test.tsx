// @vitest-environment happy-dom
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { EMPTY_AGENT_CONTEXT, agentContextKey, selectAgentContext, type AgentContextSelection } from "@research-assistant/core/agent";
import { selectAppStorage } from "../../../runtime/appStorage";
import { contextStorageKey, readConversationContext, writeConversationContext } from "./contextStorage";
import { decodeRecommendationChoices, recommendationDraftKey } from "./contextRecommendationDraft";
import { useContextRecommendations } from "./useContextRecommendations";

const observations = vi.hoisted(() => ({active: [] as readonly AgentContextSelection[], opened: [] as readonly AgentContextSelection[]}));
vi.mock("./AgentContextProvider", () => ({useAgentContextCandidates: () => observations.active, useAgentContextSuggestions: () => observations.opened}));

(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
const paper = {resource: {kind: "paper" as const, resourceId: "paper"}, label: "Paper"};
const note = {resource: {kind: "note" as const, resourceId: "note"}, label: "Note"};
const values = new Map<string, string>();
const storage: Storage = {get length() {return values.size;}, key: index => [...values.keys()][index] ?? null,
  getItem: key => values.get(key) ?? null, setItem: (key, value) => {values.set(key, value);}, removeItem: key => {values.delete(key);}, clear: () => values.clear()};
let dispose: (() => Promise<void>) | undefined;
beforeEach(() => {selectAppStorage(storage); observations.active = []; observations.opened = [];});
afterEach(async () => {await dispose?.(); dispose = undefined; values.clear();});
async function mount(id = "conversation", pendingConversation = false) {
  selectAppStorage(storage);
  const element = document.createElement("div"); document.body.append(element);
  const root = createRoot(element);
  let current!: ReturnType<typeof useContextRecommendations>;
  function Harness() {
    const [context, setContext] = useState(() => readConversationContext(id));
    const [pending, setPending] = useState(pendingConversation);
    current = useContextRecommendations(id, context, pending);
    return <button onClick={() => {setContext(current.commit("request"));setPending(false);}}>accept</button>;
  }
  await act(async () => root.render(<Harness/>));
  dispose = async () => {await act(async () => root.unmount()); element.remove();};
  return {get current() {return current;}, refresh: async () => act(async () => root.render(<Harness/>)), accept: async () => act(async () => element.querySelector("button")!.click())};
}
it("stages recommendations without changing durable context and commits only the captured request", async () => {
  observations.active = [note]; observations.opened = [note];
  writeConversationContext("conversation", selectAgentContext(EMPTY_AGENT_CONTEXT, paper, "persistent"));
  const view = await mount();
  await act(async () => view.current.choose(note, false));
  expect(view.current.selected).toEqual([note]);
  expect(readConversationContext("conversation").persistent).toEqual([paper]);
  expect(view.current.capture("request").persistent).toEqual([paper, note]);
  expect(() => view.current.commit("wrong-request")).toThrow("推荐选择与发送请求不匹配");
  expect(readConversationContext("conversation").persistent).toEqual([paper]);
  expect(view.current.items).toEqual([note]);
  expect(view.current.selected).toEqual([note]);
  await view.accept();
  expect(readConversationContext("conversation").persistent).toEqual([paper, note]);
  expect(view.current.items).toEqual([]);
  await view.accept();
  expect(readConversationContext("conversation").persistent).toEqual([paper, note]);
});
it("preserves unchecked recommendations and queued selection across reopen and isolates conversation identities", async () => {
  const view = await mount();
  await act(async () => {await view.current.choose(paper, false); await view.current.choose(note, false);});
  await act(async () => view.current.choose(paper, true));
  expect(view.current.effective.persistent).toEqual([note]);
  expect(storage.getItem(contextStorageKey("conversation"))).toBeNull();
  view.current.capture("request");
  await dispose?.(); dispose = undefined;
  const restored = await mount();
  expect(restored.current.effective.persistent).toEqual([note]);
  observations.active = [note]; observations.opened = [note];
  await restored.refresh();
  expect(restored.current.items).toEqual([note]);
  expect(restored.current.selected).toEqual([note]);
  await restored.accept();
  expect(readConversationContext("conversation").persistent).toEqual([note]);
  expect(restored.current.items).toEqual([]);
  expect(JSON.parse(storage.getItem(recommendationDraftKey("conversation"))!)).toEqual([{item: paper, selected: false}]);
  expect(storage.getItem(contextStorageKey("other"))).toBeNull();
});
it("removes closed resources from recommendations while retaining authorization and saved deselections across reopen", async () => {
  writeConversationContext("conversation", selectAgentContext(EMPTY_AGENT_CONTEXT, paper, "persistent"));
  storage.setItem("research-assistant:context-auto:conversation", agentContextKey(paper.resource));
  storage.setItem(recommendationDraftKey("conversation"), JSON.stringify([{item: note, selected: false}]));
  observations.active = [note]; observations.opened = [paper, note];
  const view = await mount();
  expect(view.current.items).toEqual([note]);
  expect(view.current.selected).toEqual([]);
  observations.active = []; observations.opened = [];
  await view.refresh();
  expect(view.current.items).toEqual([]);
  expect(view.current.capture("request").persistent).toEqual([paper]);
  await dispose?.(); dispose = undefined;
  const restored = await mount();
  expect(restored.current.items).toEqual([]);
  expect(restored.current.effective.persistent).toEqual([paper]);
  observations.active = [note]; observations.opened = [note];
  await restored.refresh();
  expect(restored.current.items).toEqual([note]);
  expect(restored.current.selected).toEqual([]);
  expect(restored.current.capture("request").persistent).toEqual([paper]);
});
it("rejects duplicate recommendation identities and unsafe resources", () => {
  expect(() => decodeRecommendationChoices([{item: paper, selected: true}, {item: paper, selected: false}])).toThrow("推荐身份重复");
  expect(() => decodeRecommendationChoices([{item: {resource: {kind: "manuscriptFile", resourceId: "manuscript", path: "../secret"}, label: "Unsafe"}, selected: true}])).toThrow();
});
it("shows an automatic selection in a pending draft, then hides it after acceptance and reopen", async () => {
  writeConversationContext("conversation", selectAgentContext(EMPTY_AGENT_CONTEXT, paper, "persistent"));
  observations.active = [paper]; observations.opened = [paper, note];
  const view = await mount("conversation", true);
  expect(view.current.items).toEqual([paper, note]);
  expect(view.current.selected).toEqual([paper]);
  view.current.capture("request");
  await view.accept();
  expect(view.current.items).toEqual([]);
  expect(view.current.effective.persistent).toEqual([paper]);
  observations.active = [note];
  await view.refresh();
  expect(view.current.items).toEqual([note]);
  await dispose?.(); dispose = undefined;
  const restored = await mount();
  expect(restored.current.items).toEqual([note]);
  expect(restored.current.effective.persistent).toEqual([paper]);
});
it("hides a configured group even when only legacy individual members were selected", async () => {
  const group: AgentContextSelection = {label:"Paper group", resource:{kind:"paperGroup",resourceId:"paper",parts:[
    {resource:paper.resource,label:"PDF",selected:true,access:"readOnly"},
  ]}};
  writeConversationContext("conversation", selectAgentContext(EMPTY_AGENT_CONTEXT, paper, "persistent"));
  observations.active = [group]; observations.opened = [group, note];
  const view = await mount();
  expect(view.current.items).toEqual([]);
  expect(view.current.capture("request").persistent).toEqual([paper]);
});
it("only recommends the active document in an established conversation while retaining staged choices across page switches", async () => {
  const activePaper: AgentContextSelection = {label:"Active paper",resource:{kind:"paperGroup",resourceId:"active",newMemberAccess:"readOnly",parts:[]}};
  const backgroundPaper: AgentContextSelection = {label:"Background paper",resource:{kind:"paperGroup",resourceId:"background",newMemberAccess:"readOnly",parts:[]}};
  writeConversationContext("conversation", selectAgentContext(EMPTY_AGENT_CONTEXT, activePaper, "persistent"));
  observations.active = [activePaper]; observations.opened = [activePaper, backgroundPaper];
  const view = await mount();
  expect(view.current.items).toEqual([]);
  expect(view.current.selected).toEqual([]);
  expect(view.current.capture("request").persistent).toEqual([activePaper]);
  observations.active = [];
  await view.refresh();
  expect(view.current.items).toEqual([]);
  expect(view.current.selected).toEqual([]);
  observations.active = [backgroundPaper];
  await view.refresh();
  expect(view.current.items).toEqual([backgroundPaper]);
  await act(async () => view.current.choose(backgroundPaper, false));
  expect(view.current.selected).toEqual([backgroundPaper]);
  observations.active = [activePaper];
  await view.refresh();
  expect(view.current.items).toEqual([]);
  expect(view.current.capture("request").persistent).toEqual([activePaper, backgroundPaper]);
  expect(readConversationContext("conversation").persistent).toEqual([activePaper]);
  await dispose?.(); dispose = undefined;
  const restored = await mount();
  expect(restored.current.items).toEqual([]);
  observations.active = [backgroundPaper];
  await restored.refresh();
  expect(restored.current.items).toEqual([backgroundPaper]);
  expect(restored.current.selected).toEqual([backgroundPaper]);
  await restored.accept();
  expect(restored.current.items).toEqual([]);
  expect(readConversationContext("conversation").persistent).toEqual([activePaper, backgroundPaper]);
});
