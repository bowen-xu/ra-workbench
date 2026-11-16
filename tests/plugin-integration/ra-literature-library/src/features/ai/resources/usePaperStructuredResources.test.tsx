// @vitest-environment happy-dom
import { act, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { StructuredResource } from "@research-assistant/core/structured-resources";
import type { StructuredResourceGateway } from "../../../../../../../apps/web/src/core/services/structuredResourceGateway";
import { usePaperStructuredResources } from "../../../../../../../../plugins/ra-literature-library/src/features/ai/resources/usePaperStructuredResources";
const environment = vi.hoisted(() => ({gateway: undefined as StructuredResourceGateway | undefined,
  runtime: {mode: "preview", gateways: {getStructuredResourceGateway: true}}}));
vi.mock("../../../../../../../apps/web/src/core/runtime/appRuntime", () => ({getAppRuntime: () => environment.runtime}));
vi.mock("../../../../../../../apps/web/src/core/services/structuredResourceGateway", () => ({getStructuredResourceGateway: () => environment.gateway}));
vi.mock("../../../../../../../apps/web/src/core/runtime/structuredResourceStartup", () => ({ensureStructuredResourceStartup: async () => undefined}));
(globalThis as {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
let dispose: (() => Promise<void>) | undefined;
afterEach(async () => {await dispose?.(); dispose = undefined; document.body.innerHTML = "";});
const reading: StructuredResource = {id: "reading", kind: "structuredReading", paperId: "paper",
  sourceUri: "source://library/paper/reading", title: "Reading", revision: 1, content: {fields: []},
  reviewBaseline: null, pendingFieldIds: ["q"], reviewedFieldIds: []};
async function setup(subscribe = true) {
  let current = reading;
  const listeners = new Set<(change: {resourceId: string; paperId: string | null}) => void>();
  const list = vi.fn(async () => [structuredClone(current)]), getSources = vi.fn(async () => []);
  environment.gateway = {list, getSources, ...(subscribe ? {subscribeChanges: async (changed: (change: {resourceId: string; paperId: string | null}) => void) => {
    listeners.add(changed); return () => {listeners.delete(changed);};
  }} : {})} as unknown as StructuredResourceGateway;
  const views: Record<string, ReturnType<typeof usePaperStructuredResources>> = {};
  const installed: Record<string, StructuredResource[]> = {left: [], right: []};
  function Host({name}: {readonly name: string}) {
    const view = usePaperStructuredResources("paper"); views[name] = view;
    useEffect(() => {if (view.reading) installed[name]!.push(view.reading);}, [view.reading]);
    return <div>{view.reading?.revision}</div>;
  }
  const container = document.createElement("div"); document.body.append(container);
  const root = createRoot(container); dispose = () => act(async () => root.unmount());
  await act(async () => root.render(<><Host name="left"/><Host name="right"/></>));
  return {views, installed, list, getSources, set: (resource: StructuredResource) => {current = resource;},
    emit: () => listeners.forEach(listener => listener({resourceId: "reading", paperId: "paper"}))};
}

it("refreshes both rails once per gateway event without invalidating legacy candidates or reinstalling identical revisions", async () => {
  const f = await setup(); const calls = f.list.mock.calls.length;
  const legacy = vi.fn(); window.addEventListener("research-assistant:paper-reading-changed", legacy);
  try {
    const next = {...reading, revision: 2, reviewBaseline: reading.content, pendingFieldIds: []};
    f.set(next);
    await act(async () => {f.emit(); f.views.left!.changed(next);});
    expect(f.list).toHaveBeenCalledTimes(calls + 2);
    expect(legacy).not.toHaveBeenCalled();
    expect(f.installed.left!.map(value => value.revision)).toEqual([1, 2]);
    expect(f.installed.right!.map(value => value.revision)).toEqual([1, 2]);
    const saved = f.views.left!.reading;
    await act(async () => f.emit());
    expect(f.views.left!.reading).toBe(saved);
    expect(f.installed.left!).toHaveLength(2);
    expect(f.views.left!.loading).toBe(false);
  } finally {window.removeEventListener("research-assistant:paper-reading-changed", legacy);}
});

it("keeps same-version repair drafts and out-of-band revisions observable", async () => {
  const f = await setup();
  const draft = {id: "draft", resourceId: "reading", baseRevision: 1, revision: 1,
    content: "{invalid", contentHash: "hash", errors: []};
  f.set({...reading, draft}); await act(async () => f.emit());
  expect(f.views.left!.reading?.draft?.content).toBe("{invalid");
  f.set({...reading, revision: 3, content: {fields: [{text: "external"}]}});
  await act(async () => f.emit()); expect(f.views.left!.reading?.revision).toBe(3);
  f.set(reading); await act(async () => f.emit());
  expect(f.views.left!.reading?.revision).toBe(3);
});

it("falls back to paper notifications and reload when subscriptions are unavailable", async () => {
  const f = await setup(false); const calls = f.list.mock.calls.length;
  const next = {...reading, revision: 2}; f.set(next);
  await act(async () => f.views.left!.changed(next));
  expect(f.list.mock.calls.length).toBeGreaterThan(calls);
  expect(f.views.right!.reading?.revision).toBe(2);
});
