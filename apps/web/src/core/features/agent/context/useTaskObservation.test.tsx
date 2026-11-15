// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, expect, it, vi} from "vitest";
import type {AgentTaskObservationGateway} from "@research-assistant/data/agent";
import {registerStoragePersistence, type StoragePersistence} from "../../../runtime/storagePersistence";
import {useTaskObservation} from "./useTaskObservation";

const id = "11111111-1111-4111-8111-111111111111", other = "22222222-2222-4222-8222-222222222222";
const identity = {conversationId:id, turnId:id, runId:id};
const key = `research-assistant:agent-task-observation:v1:${id}`;
let cleanup: (() => Promise<void>) | undefined;
afterEach(async () => {await cleanup?.(); cleanup = undefined; localStorage.clear();});

function Probe({conversationId, gateway, storage}: {conversationId:string; gateway:AgentTaskObservationGateway; storage:Storage}) {
  const task = useTaskObservation(conversationId, storage, gateway);
  return <div><span>{task.state?.status ?? "idle"}:{String(task.blocked)}</span>
    <span>{task.state?.observationFailure?.phase}:{task.state?.observationFailure?.cause}</span>
    <button onClick={() => task.recheck()}>check</button>
    <button onClick={() => void task.clearSettled({schemaVersion:1,conversationId,requestId:id,turnId:id,alreadyStarted:true,outcome:"completed"})}>settled</button>
    <button onClick={() => void task.track({schemaVersion:1,conversationId,requestId:id,turnId:id,runId:other,alreadyStarted:false,outcome:null})}>track other</button></div>;
}

it("restores a live pending run without treating it as an observation failure", async () => {
  localStorage.setItem(key, JSON.stringify(identity));
  const observe = vi.fn(async () => ({schemaVersion:1 as const,...identity,status:{type:"pending" as const}}));
  const {host} = await mount({observe});
  await vi.waitFor(() => expect(host.textContent).toContain("tracking:false"));
  expect(localStorage.getItem(key)).not.toBeNull();
});

it("identifies an unavailable owner separately from a failed observation", async () => {
  localStorage.setItem(key, JSON.stringify(identity));
  const observe = vi.fn(async () => ({schemaVersion:1 as const,...identity,status:{type:"unavailable" as const}}));
  const {host} = await mount({observe});
  await vi.waitFor(() => expect(host.textContent).toContain("observe:owner_unavailable"));
});

it("classifies observation errors without including their private text", async () => {
  localStorage.setItem(key, JSON.stringify(identity));
  const observe = vi.fn().mockRejectedValue(new Error("timeout private document content"));
  const {host} = await mount({observe});
  await vi.waitFor(() => expect(host.textContent).toContain("observe:timeout"));
  expect(host.textContent).not.toContain("private document content");
});

it("records an identity mismatch rather than treating another run as the original", async () => {
  localStorage.setItem(key, JSON.stringify(identity));
  const observe = vi.fn(async () => ({schemaVersion:1 as const,...identity,runId:other,status:{type:"pending" as const}}));
  const {host} = await mount({observe});
  await vi.waitFor(() => expect(host.textContent).toContain("observe:identity_mismatch"));
  expect(localStorage.getItem(key)).not.toBeNull();
});
async function mount(gateway: AgentTaskObservationGateway, storage:Storage = localStorage) {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  cleanup = async () => {await act(async () => root.unmount()); host.remove();};
  const render = async (conversationId: string) => act(async () => root.render(<Probe conversationId={conversationId} gateway={gateway} storage={storage}/>));
  await render(id); return {host, render};
}

it("keeps a restored locator blocked during recheck and never treats unavailable as not started", async () => {
  localStorage.setItem(key, JSON.stringify(identity));
  let resolve!: (value: Awaited<ReturnType<AgentTaskObservationGateway["observe"]>>) => void;
  const observe = vi.fn(() => new Promise<Awaited<ReturnType<AgentTaskObservationGateway["observe"]>>>(done => {resolve = done;}));
  const {host} = await mount({observe});
  expect(host.textContent).toContain("checking:true");
  await act(async () => resolve({schemaVersion:1,...identity,status:{type:"unavailable"}}));
  expect(host.textContent).toContain("unconfirmed:true");
  expect(localStorage.getItem(key)).not.toBeNull();
  await act(async () => host.querySelector("button")!.click());
  expect(host.textContent).toContain("checking:true");
  await act(async () => resolve({schemaVersion:1,...identity,status:{type:"settled"}}));
  expect(host.textContent).toContain("idle:false"); expect(localStorage.getItem(key)).toBeNull();
});

it("clears only the matching locator when a durable settlement is proven", async () => {
  localStorage.setItem(key, JSON.stringify(identity));
  const observe = vi.fn(() => new Promise<Awaited<ReturnType<AgentTaskObservationGateway["observe"]>>>(() => undefined));
  const {host} = await mount({observe});
  const button = [...host.querySelectorAll("button")].find(item => item.textContent === "settled")!;
  await act(async () => button.click());
  expect(localStorage.getItem(key)).toBeNull(); expect(host.textContent).toContain("idle:false");
});

it("does not clear a locator for a different turn on settlement", async () => {
  const different = {...identity, turnId:other}; localStorage.setItem(key, JSON.stringify(different));
  const observe = vi.fn(() => new Promise<Awaited<ReturnType<AgentTaskObservationGateway["observe"]>>>(() => undefined));
  const {host} = await mount({observe});
  const button = [...host.querySelectorAll("button")].find(item => item.textContent === "settled")!;
  await act(async () => button.click());
  expect(JSON.parse(localStorage.getItem(key)!)).toEqual(different);
  expect(host.textContent).toContain("checking:true");
});

it("ignores a late observation after switching conversations", async () => {
  localStorage.setItem(key, JSON.stringify(identity));
  let resolve!: (value: Awaited<ReturnType<AgentTaskObservationGateway["observe"]>>) => void;
  const observe = vi.fn(() => new Promise<Awaited<ReturnType<AgentTaskObservationGateway["observe"]>>>(done => {resolve = done;}));
  const {host, render} = await mount({observe});
  await render(other); expect(host.textContent).toContain("idle:false");
  await act(async () => resolve({schemaVersion:1,...identity,status:{type:"finished",completion:"recoveryRequired",diagnostic:null}}));
  expect(host.textContent).toContain("idle:false"); expect(localStorage.getItem(key)).not.toBeNull();
});

it.each(["not-json", ""])("retains a malformed locator and blocks without inventing an identity", async raw => {
  localStorage.setItem(key, raw);
  const observe = vi.fn(); const {host} = await mount({observe});
  expect(host.textContent).toContain("unconfirmed:true");
  await act(async () => host.querySelector("button")!.click());
  expect(observe).not.toHaveBeenCalled(); expect(localStorage.getItem(key)).toBe(raw);
});

it("does not overwrite another unresolved original locator", async () => {
  localStorage.setItem(key, JSON.stringify(identity));
  const observe = vi.fn(() => new Promise<Awaited<ReturnType<AgentTaskObservationGateway["observe"]>>>(() => undefined));
  const {host} = await mount({observe});
  const button = [...host.querySelectorAll("button")].find(item => item.textContent === "track other")!;
  await act(async () => button.click());
  expect(host.textContent).toContain("unconfirmed:true");
  expect(JSON.parse(localStorage.getItem(key)!)).toEqual(identity);
});

it("keeps the original locator when clearing it cannot be synchronized", async () => {
  const entries = new Map<string,string>([[key, JSON.stringify(identity)]]);
  const storage:Storage = {get length() {return entries.size;}, key:index => [...entries.keys()][index] ?? null,
    getItem:name => entries.get(name) ?? null, setItem:(name,value) => {entries.set(name,value);},
    removeItem:name => {entries.delete(name);}, clear:() => entries.clear()};
  const flush = vi.fn().mockRejectedValueOnce(new Error("write failed")).mockResolvedValue(undefined);
  registerStoragePersistence(storage, {flush} as unknown as StoragePersistence);
  const observe = vi.fn(async () => ({schemaVersion:1 as const,...identity,
    status:{type:"finished" as const,completion:"completed" as const,diagnostic:null}}));
  const {host} = await mount({observe}, storage);
  await vi.waitFor(() => expect(host.textContent).toContain("unconfirmed:true"));
  expect(JSON.parse(storage.getItem(key)!)).toEqual(identity);
  expect(flush).toHaveBeenCalledTimes(2);
});

it("can retry a proven settlement after a locator flush failure", async () => {
  const entries = new Map<string,string>([[key, JSON.stringify(identity)]]);
  const storage:Storage = {get length() {return entries.size;}, key:index => [...entries.keys()][index] ?? null,
    getItem:name => entries.get(name) ?? null, setItem:(name,value) => {entries.set(name,value);},
    removeItem:name => {entries.delete(name);}, clear:() => entries.clear()};
  const flush = vi.fn().mockRejectedValueOnce(new Error("write failed")).mockResolvedValue(undefined);
  registerStoragePersistence(storage, {flush} as unknown as StoragePersistence);
  const observe = vi.fn(() => new Promise<Awaited<ReturnType<AgentTaskObservationGateway["observe"]>>>(() => undefined));
  const {host} = await mount({observe}, storage);
  const button = [...host.querySelectorAll("button")].find(item => item.textContent === "settled")!;
  await act(async () => button.click());
  expect(host.textContent).toContain("unconfirmed:true"); expect(JSON.parse(storage.getItem(key)!)).toEqual(identity);
  await act(async () => button.click());
  expect(host.textContent).toContain("idle:false"); expect(storage.getItem(key)).toBeNull();
});
