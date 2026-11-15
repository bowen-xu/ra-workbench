/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { AgentConversationSummary } from "@research-assistant/core/agent";
import type { AgentCatalogGateway } from "@research-assistant/data/agent";
import { BoundConversationList } from "./BoundConversationList";
import { invalidateConversationCatalog } from "../../runtime/conversationCatalogEvents";

vi.mock("../../services/notifications", () => ({ notify: { error: vi.fn(), success: vi.fn() } }));
vi.mock("../../services/agentGateway", async importOriginal => ({ ...await importOriginal<typeof import("../../services/agentGateway")>(), agentArchiveGateway: {}, agentCatalogGateway: {}, agentDeletionGateway: {}, agentSearchGateway: {} }));
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
const row = (id: string, archived = false): AgentConversationSummary => ({id, title: id, archived, durableCount: 0, createdAt: "now", updatedAt: "now"});
const page = (rows: AgentConversationSummary[], next: string | null = null) => ({schemaVersion: 1 as const, conversations: rows, next: next ? {id:next, createdAt:"now"} : null});
function deferred<T>() { let resolve!: (value:T)=>void, reject!: (reason:unknown)=>void; const promise = new Promise<T>((yes,no) => {resolve=yes;reject=no;}); return {promise,resolve,reject}; }
const stops: (()=>Promise<void>)[] = [];
afterEach(async () => {for (const stop of stops.splice(0)) await stop(); vi.restoreAllMocks();});
async function mount(list: AgentCatalogGateway["list"], read: (request: {id:string})=>Promise<AgentConversationSummary>, search?: import("@research-assistant/data/agent").AgentSearchGateway) {
  const host=document.createElement("div"); document.body.append(host); const root=createRoot(host);
  const catalog={list,rename:vi.fn(),renameStatus:vi.fn()}, metadata={read}, onSelect=vi.fn();
  const render=async (selectedId:string | null) => {await act(async()=>root.render(<BoundConversationList binding={null} selectedId={selectedId} onSelect={onSelect} catalog={catalog} metadata={metadata} {...(search ? {search} : {})}/>));};
  stops.push(async()=>{await act(async()=>root.unmount());host.remove();});
  await render("selected");
  return {host,render,button:(name:string)=>host.querySelector<HTMLButtonElement>(`[aria-label="${name}"]`)!, onSelect};
}
it("renders the first catalog page while selected metadata is pending, then supplements the selected row", async () => {
  const lookup=deferred<AgentConversationSummary>();
  const {host}=await mount(vi.fn(async()=>page([row("first")],"first")),()=>lookup.promise);
  expect(host.textContent).toContain("first"); expect(host.textContent).not.toContain("正在加载对话");
  await act(async()=>lookup.resolve(row("selected")));
  expect([...host.querySelectorAll<HTMLElement>("[data-conversation-id]")].map(node=>node.dataset.conversationId)).toEqual(["selected","first"]);
});
it("keeps the loaded page visible if the selected metadata fails", async () => {
  const lookup=deferred<AgentConversationSummary>();
  const {host}=await mount(vi.fn(async()=>page([row("first")])),()=>lookup.promise);
  await act(async()=>lookup.reject(new Error("offline")));
  expect(host.textContent).toContain("first"); expect(host.textContent).not.toContain("重新加载对话");
});
it("ignores metadata returned after selection changes", async () => {
  const lookup=deferred<AgentConversationSummary>();
  const {host,render}=await mount(vi.fn(async()=>page([row("first")])),()=>lookup.promise);
  await render("different"); await act(async()=>lookup.resolve(row("selected")));
  expect(host.querySelector('[data-conversation-id="selected"]')).toBeNull();
  expect(host.textContent).toContain("first");
});
it("does not insert a late current-view row into the archived view", async () => {
  const lookup=deferred<AgentConversationSummary>();
  const list=vi.fn<AgentCatalogGateway["list"]>(async query=>page([row(query.archived?"archive":"first",query.archived)]));
  const {host,button}=await mount(list,()=>lookup.promise);
  await act(async()=>button("查看归档对话").click()); await act(async()=>lookup.resolve(row("selected")));
  expect(host.textContent).toContain("archive"); expect(host.querySelector('[data-conversation-id="selected"]')).toBeNull();
});
it("ignores a stale lookup after loading another page and does not duplicate its selected row", async () => {
  const lookup=deferred<AgentConversationSummary>();
  const list=vi.fn<AgentCatalogGateway["list"]>(async query=>query.before ? page([row("selected")]) : page([row("first")],"first"));
  const {host}=await mount(list,()=>lookup.promise);
  const more=[...host.querySelectorAll<HTMLButtonElement>("button")].find(button=>button.textContent?.includes("加载更多"))!;
  await act(async()=>more.click()); await act(async()=>lookup.resolve(row("selected")));
  expect(host.querySelectorAll('[data-conversation-id="selected"]')).toHaveLength(1);
  expect(host.textContent).toContain("first");
});
it("ignores a late selected lookup after a search replaces the catalog page", async () => {
  const lookup=deferred<AgentConversationSummary>();
  const search=vi.fn(async()=>({schemaVersion:1 as const,hits:[],next:null}));
  const {host,button}=await mount(vi.fn(async()=>page([row("first")])),()=>lookup.promise,{search});
  await act(async()=>button("搜索对话").click());
  const input=host.querySelector<HTMLInputElement>("input")!;
  await act(async()=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(input,"query");input.dispatchEvent(new Event("input",{bubbles:true}));});
  await act(async()=>host.querySelector("form")!.dispatchEvent(new Event("submit",{bubbles:true,cancelable:true})));
  await act(async()=>lookup.resolve(row("selected")));
  expect(search).toHaveBeenCalled(); expect(host.querySelector('[data-conversation-id="selected"]')).toBeNull();
});
it("keeps the refreshed page when an older lookup returns after background synchronization", async () => {
  const lookup=deferred<AgentConversationSummary>();
  const list=vi.fn<AgentCatalogGateway["list"]>().mockResolvedValueOnce(page([row("first")])).mockResolvedValue(page([row("selected"),row("updated")]));
  const {host}=await mount(list,()=>lookup.promise);
  await act(async()=>{invalidateConversationCatalog();await new Promise(resolve=>setTimeout(resolve,10));});
  await act(async()=>lookup.resolve({...row("selected"),title:"stale"}));
  expect(host.querySelectorAll('[data-conversation-id="selected"]')).toHaveLength(1);
  expect(host.textContent).toContain("updated"); expect(host.textContent).not.toContain("stale");
});
it("shares a pending metadata lookup across repeated refreshes and keeps its result on later refreshes", async () => {
  const lookup=deferred<AgentConversationSummary>();
  const read=vi.fn(()=>lookup.promise);
  const list=vi.fn<AgentCatalogGateway["list"]>(async()=>page([row("first")]));
  const {host}=await mount(list,read);
  for (let i=0;i<3;i++) await act(async()=>{invalidateConversationCatalog();await new Promise(resolve=>setTimeout(resolve,10));});
  expect(list).toHaveBeenCalledTimes(4); expect(read).toHaveBeenCalledTimes(1);
  await act(async()=>lookup.resolve(row("selected")));
  expect(host.querySelectorAll('[data-conversation-id="selected"]')).toHaveLength(1);
  await act(async()=>{invalidateConversationCatalog();await new Promise(resolve=>setTimeout(resolve,10));});
  expect(read).toHaveBeenCalledTimes(2);
  expect(host.querySelectorAll('[data-conversation-id="selected"]')).toHaveLength(1);
});
it("rechecks a previously supplemented row after a catalog refresh", async () => {
  const lookup=deferred<AgentConversationSummary>();
  const read=vi.fn().mockResolvedValueOnce(row("selected")).mockReturnValueOnce(lookup.promise);
  const {host}=await mount(vi.fn(async()=>page([row("first")])),read);
  expect(host.querySelector('[data-conversation-id="selected"]')).not.toBeNull();
  await act(async()=>{invalidateConversationCatalog();await new Promise(resolve=>setTimeout(resolve,10));});
  expect(host.querySelector('[data-conversation-id="selected"]')).not.toBeNull();
  await act(async()=>lookup.reject(new Error("removed")));
  expect(host.querySelector('[data-conversation-id="selected"]')).toBeNull();
  expect(host.textContent).toContain("first");
});
it("shares the same pending lookup while loading another page", async () => {
  const lookup=deferred<AgentConversationSummary>();
  const read=vi.fn(()=>lookup.promise);
  const list=vi.fn<AgentCatalogGateway["list"]>(async query=>query.before ? page([row("second")]) : page([row("first")],"first"));
  const {host}=await mount(list,read);
  const more=[...host.querySelectorAll<HTMLButtonElement>("button")].find(button=>button.textContent?.includes("加载更多"))!;
  await act(async()=>more.click()); expect(read).toHaveBeenCalledTimes(1);
  await act(async()=>lookup.resolve(row("selected")));
  expect(host.textContent).toContain("first"); expect(host.textContent).toContain("second"); expect(host.textContent).toContain("selected");
});
